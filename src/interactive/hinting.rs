//! The ghost suggestion shown past the cursor.
//!
//! Three sources, in fish's order: **history, then completions, then file paths.** History is a
//! line the user has actually run, which is a better guess than anything that can be ranked;
//! completions answer for a command name being typed; and paths answer for the argument after it,
//! which is the case the other two cannot see at all.
//!
//! Two things were wrong with it before. It only fired at column zero, so `true && ec` suggested
//! nothing; and it sorted alphabetically, so typing `exit` suggested `exitsnoop-bpfcc` — a
//! command the user has never run, ahead of the one they were plainly typing.

use super::OsloHelper;
use super::command_index::CommandIndex;
use super::words::{Quote, current_word};

/// How a hint candidate was found, best first. Ties on frecency are broken by this.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
enum Origin {
    /// An external program on `$PATH`.
    External,
    /// A builtin, alias or function: part of this shell, so nearer to hand.
    Shell,
}

impl OsloHelper {
    /// The completion of the command name being typed, or `None`.
    ///
    /// Returns the *tail* to append, which is what rustyline draws past the cursor.
    pub fn command_hint(&self, line: &str, pos: usize) -> Option<String> {
        let word = current_word(line, pos);
        // A half-typed quoted argument is a filename, not a command, and guessing at it in grey
        // text is worse than saying nothing.
        if !word.command_position || word.quote != Quote::None || word.stem.is_empty() {
            return None;
        }
        let stem = word.stem.as_str();

        let env = self.env.lock().unwrap();
        let path = env.get_var("PATH").unwrap_or_default().to_string();
        let is_shell_name = |n: &str| {
            env.is_builtin(n) || env.get_alias(n).is_some() || env.get_function(n).is_some()
        };

        // If what has been typed already names a command, it is not a prefix of the answer — it
        // *is* the answer. This is the `exit` case: bash shows nothing, and so should we.
        if is_shell_name(stem) || CommandIndex::contains(&path, stem) {
            return None;
        }

        let mut best: Option<Ranked> = None;
        let mut consider = |name: &str, origin: Origin, score: f64| {
            if !name.starts_with(stem) || name.len() == stem.len() {
                return;
            }
            let candidate = Ranked {
                score,
                origin,
                name: name.to_string(),
            };
            if best.as_ref().is_none_or(|current| candidate.beats(current)) {
                best = Some(candidate);
            }
        };

        for name in env.builtin_names() {
            consider(name, Origin::Shell, self.frecency.score(name));
        }
        for name in env.get_aliases().keys() {
            consider(name, Origin::Shell, self.frecency.score(name));
        }
        for name in env.get_functions().keys() {
            consider(name, Origin::Shell, self.frecency.score(name));
        }
        drop(env);

        for name in CommandIndex::executables(&path).iter() {
            // The score lookup is behind the prefix test on purpose: it takes a lock, and 3373
            // lock acquisitions per keystroke is the cost this whole change exists to remove.
            if name.starts_with(stem) {
                consider(name, Origin::External, self.frecency.score(name));
            }
        }

        best.map(|b| b.name[stem.len()..].to_string())
    }
}

impl OsloHelper {
    /// The completion of a *path* being typed, or `None`.
    ///
    /// fish's third source, and the one that covers the argument rather than the command. Only
    /// for a word that is not in command position: a bare name at the start of a line is a command
    /// to look up, not a file in the current directory, and suggesting `./notes.txt` when someone
    /// typed `no` would be nonsense.
    pub fn path_hint(&self, line: &str, pos: usize) -> Option<String> {
        let word = current_word(line, pos);
        if word.command_position || word.stem.is_empty() {
            return None;
        }

        // Split what was typed into the directory to look in and the stem to match. A trailing
        // `/` means the directory itself is complete and every entry in it is a candidate.
        let typed = word.stem.as_str();
        let (dir, stem) = match typed.rfind('/') {
            Some(cut) => (&typed[..=cut], &typed[cut + 1..]),
            None => ("", typed),
        };
        // Only once there is something to match on. Listing a directory on the keystroke that
        // begins a word would fire for every argument of every command.
        if stem.is_empty() && dir.is_empty() {
            return None;
        }

        let expanded = expand_tilde(dir);
        let base = if expanded.is_empty() { "." } else { &expanded };
        let mut best: Option<String> = None;
        for entry in std::fs::read_dir(base).ok()?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(stem) || name.len() == stem.len() {
                continue;
            }
            // A dotfile is only offered once the user has typed the dot, the same rule globbing
            // follows — otherwise every argument suggests `.git`.
            if name.starts_with('.') && !stem.starts_with('.') {
                continue;
            }
            let suffix = if entry.file_type().is_ok_and(|t| t.is_dir()) {
                "/"
            } else {
                ""
            };
            let candidate = format!("{name}{suffix}");
            // Shortest wins: it is the least presumptuous completion, and the one the user is
            // most likely already heading for.
            if best
                .as_ref()
                .is_none_or(|b| (candidate.len(), &candidate) < (b.len(), b))
            {
                best = Some(candidate);
            }
        }
        best.map(|name| name[stem.len()..].to_string())
    }
}

/// A leading `~`, so a path typed with one can still be suggested.
fn expand_tilde(dir: &str) -> String {
    match dir.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => match std::env::var("HOME") {
            Ok(home) if !home.is_empty() => format!("{home}{rest}"),
            _ => dir.to_string(),
        },
        _ => dir.to_string(),
    }
}

/// One candidate hint, with everything the ordering looks at.
struct Ranked {
    score: f64,
    origin: Origin,
    name: String,
}

impl Ranked {
    /// Most used first; then shell-provided; then shortest; then alphabetical.
    ///
    /// Length before the alphabet because with nothing else to go on the shortest completion is
    /// the least presumptuous — it is the one the user is most likely already heading for.
    fn beats(&self, other: &Self) -> bool {
        match self.score.partial_cmp(&other.score) {
            Some(std::cmp::Ordering::Greater) => return true,
            Some(std::cmp::Ordering::Less) | None => return false,
            Some(std::cmp::Ordering::Equal) => {}
        }
        if self.origin != other.origin {
            return self.origin > other.origin;
        }
        if self.name.len() != other.name.len() {
            return self.name.len() < other.name.len();
        }
        self.name < other.name
    }
}
