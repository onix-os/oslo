//! Session history and its append-only backing file.
//!
//! # Appended, never rewritten
//!
//! `save_history` writes the whole file, so two shells open at once each ended with only their own
//! commands (PLAN R9.11). Every line is appended as it is typed instead, which is also what makes a
//! command that hangs or kills the shell still turn up in the history afterwards.
//!
//! # The size limit is applied on load
//!
//! Trimming on every append would mean rewriting the file per command. The file is allowed to grow
//! past the limit between sessions and is trimmed when it is next read, which is the same trade
//! bash makes and is invisible unless you look at the file.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The in-memory history, oldest first.
#[derive(Debug, Default)]
pub struct History {
    entries: Vec<String>,
    file: Option<PathBuf>,
    max: usize,
    /// How many lines the file holds, so it can be rewritten when it outgrows the limit without
    /// being read back to find out.
    file_lines: usize,
}

impl History {
    /// Read `file`, keeping at most `max` entries.
    ///
    /// A missing or unreadable file is an empty history rather than an error: a first run has no
    /// file, and a shell that refused to start over an unreadable one would be worse than a shell
    /// that starts without your history.
    pub fn open(file: Option<PathBuf>, max: usize) -> History {
        let mut entries: Vec<String> = match &file {
            Some(path) => std::fs::read_to_string(path)
                .map(|text| text.lines().map(unescape).collect())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        entries.retain(|line| !line.is_empty());
        if max > 0 && entries.len() > max {
            entries.drain(..entries.len() - max);
        }
        History {
            file_lines: entries.len(),
            entries,
            file,
            max,
        }
    }

    /// Every entry, oldest first.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Remember a line, and append it to the file.
    ///
    /// A line identical to the one before it is not stored twice — the editor's Up walk would
    /// otherwise show it twice in a row, which reads as the key having failed. The *file* still
    /// receives it, because `$HISTFILE` is a record of what ran.
    pub fn add(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        if self.entries.last().map(String::as_str) != Some(line) {
            self.entries.push(line.to_string());
            if self.max > 0 && self.entries.len() > self.max {
                self.entries.remove(0);
            }
        }
        self.append_to_file(line);
    }

    fn append_to_file(&mut self, line: &str) {
        let Some(path) = self.file.clone() else {
            return;
        };
        if let Err(e) = append_line(&path, &escape(line)) {
            self.report(&path, e);
            return;
        }
        self.file_lines += 1;

        // The file is trimmed only when it has outgrown the limit by a margin, so the rewrite it
        // costs is paid once per `max / 4` commands rather than on every one past the cap. The
        // alternative — rewriting the whole file per command — is what makes an append-only
        // history worth having in the first place.
        let slack = self.max / 4;
        if self.max > 0 && self.file_lines > self.max + slack {
            let kept: Vec<String> = self.entries.iter().map(|line| escape(line)).collect();
            match std::fs::write(&path, kept.join("\n") + "\n") {
                Ok(()) => self.file_lines = self.entries.len(),
                Err(e) => self.report(&path, e),
            }
        }
    }

    fn report(&self, path: &Path, e: std::io::Error) {
        eprintln!(
            "oslo: {}: {}",
            oslo_ui::marks::path(&path.display().to_string()),
            e
        );
    }

    /// Forget everything in memory. The file is left alone, which is what `history -c` means here:
    /// see `recall`'s note on why the directories are kept too.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// One entry, on one line.
///
/// A newline inside a command would be an entry boundary on the next load, splitting one command
/// into several — so it is written as `\n` and read back. The backslash has to be escaped too, or
/// a command ending in one would swallow the next line's boundary.
fn escape(line: &str) -> String {
    line.replace('\\', "\\\\").replace('\n', "\\n")
}

/// The inverse.
fn unescape(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_remembered_and_appended() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hist");
        let mut history = History::open(Some(path.clone()), 100);
        history.add("echo one");
        history.add("echo two");
        assert_eq!(history.entries(), ["echo one", "echo two"]);

        // A second shell reading the same file sees both.
        let reopened = History::open(Some(path), 100);
        assert_eq!(reopened.entries(), ["echo one", "echo two"]);
    }

    /// The same line twice running is one entry, or the Up walk shows it twice and looks broken.
    #[test]
    fn an_immediate_repeat_is_not_stored_twice() {
        let mut history = History::open(None, 100);
        history.add("ls");
        history.add("ls");
        history.add("pwd");
        history.add("ls");
        assert_eq!(
            history.entries(),
            ["ls", "pwd", "ls"],
            "but a later repeat is"
        );
    }

    /// A multi-line command is one entry, and comes back with its newlines.
    ///
    /// A raw newline in the file is an entry boundary, so it would split one command into three on
    /// the next load — and the pieces would not parse.
    #[test]
    fn a_multi_line_command_stays_one_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hist");
        let command = "for i in a b\ndo echo $i\ndone";
        let mut history = History::open(Some(path.clone()), 100);
        history.add(command);

        let text = std::fs::read_to_string(&path).expect("file");
        assert_eq!(text.lines().count(), 1, "one entry is one line: {text:?}");

        let reopened = History::open(Some(path), 100);
        assert_eq!(reopened.entries(), [command], "and it round trips");
    }

    /// A backslash at the end of a command must not swallow the entry boundary.
    #[test]
    fn a_trailing_backslash_survives() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hist");
        let mut history = History::open(Some(path.clone()), 100);
        history.add("echo ending in a backslash \\");
        history.add("echo after");
        let reopened = History::open(Some(path), 100);
        assert_eq!(
            reopened.entries(),
            ["echo ending in a backslash \\", "echo after"]
        );
    }

    /// The **file** is capped too, not only the memory — trimmed with slack so the rewrite it
    /// costs is not paid on every command past the limit.
    #[test]
    fn the_file_is_trimmed_when_it_outgrows_the_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hist");
        let mut history = History::open(Some(path.clone()), 2);
        for line in [": a", ": b", ": c", ": d"] {
            history.add(line);
        }
        let text = std::fs::read_to_string(&path).expect("file");
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [": c", ": d"],
            "the file kept the newest: {text:?}"
        );
    }

    /// The limit is applied when the file is read, keeping the newest.
    #[test]
    fn the_size_limit_keeps_the_newest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hist");
        let mut history = History::open(Some(path.clone()), 100);
        for i in 0..10 {
            history.add(&format!("command {i}"));
        }
        let trimmed = History::open(Some(path), 3);
        assert_eq!(trimmed.entries(), ["command 7", "command 8", "command 9"]);
    }

    /// No file at all is a working history that simply is not saved — `HISTFILE=` asks for this.
    #[test]
    fn a_session_without_a_file_still_remembers() {
        let mut history = History::open(None, 10);
        history.add("secret work");
        assert_eq!(history.entries(), ["secret work"]);
    }

    #[test]
    fn clearing_forgets_the_session_and_keeps_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hist");
        let mut history = History::open(Some(path.clone()), 100);
        history.add("kept in the file");
        history.clear();
        assert!(history.entries().is_empty());
        assert!(
            std::fs::read_to_string(&path)
                .expect("file")
                .contains("kept"),
            "the file is the record of what ran"
        );
    }
}
