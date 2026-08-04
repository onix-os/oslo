//! `ui file` — walk the filesystem and pick something.
//!
//! [`super::choose`] whose list is a directory and whose Enter sometimes means "go in here"
//! instead of "this one". That one difference is the whole widget: everything else is the same
//! list, the same keys and the same filter.
//!
//! # Reading a directory per keystroke is fine
//!
//! It is one `readdir` of one directory, which is a few hundred microseconds on anything with a
//! page cache — and caching it would mean showing a file that has since been deleted, which for a
//! file picker is the one wrong answer worth avoiding. Moving into a directory rereads it.

use super::{Answer, legend, show};
use crate::interactive::dropdown::width::{terminal_cols, terminal_rows, truncate_to_width};
use crate::interactive::matching::{Fuzzed, Fuzzy};
use crate::interactive::term::{Key, Keys, Restore};
use crate::interactive::theme;
use std::path::{Path, PathBuf};

/// What may be picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    Files,
    Directories,
    /// Either — Enter on a directory picks it, and Right goes into it.
    Both,
}

/// How the picker is asked.
#[derive(Debug, Clone)]
pub struct Browse {
    pub start: PathBuf,
    pub want: Want,
    /// Show entries whose name begins with a dot.
    pub hidden: bool,
    pub height: usize,
    pub fuzzy: Fuzzy,
}

impl Default for Browse {
    fn default() -> Self {
        Browse {
            start: PathBuf::from("."),
            want: Want::Files,
            hidden: false,
            height: 12,
            fuzzy: Fuzzy::Smart,
        }
    }
}

/// One row of the listing.
#[derive(Debug, Clone)]
struct Entry {
    name: String,
    directory: bool,
}

/// Read `at`, sorted directories first and then by name.
///
/// Sorted that way because moving *through* the tree is the common act and the directories are
/// what you move through; a name-only sort scatters them among the files.
fn read(at: &Path, hidden: bool) -> Vec<Entry> {
    let Ok(entries) = std::fs::read_dir(at) else {
        return Vec::new();
    };
    let mut out: Vec<Entry> = entries
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if !hidden && name.starts_with('.') {
                return None;
            }
            Some(Entry {
                directory: entry.file_type().map(|t| t.is_dir()).unwrap_or(false),
                name,
            })
        })
        .collect();
    out.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.name.cmp(&b.name)));
    out
}

pub fn file(spec: &Browse) -> Answer<String> {
    let ui = theme::current().ui;
    let depth = theme::depth();

    let Some(raw) = Restore::enter(false) else {
        return Answer::NoTerminal;
    };

    let mut at = spec
        .start
        .canonicalize()
        .unwrap_or_else(|_| spec.start.clone());
    let mut entries = read(&at, spec.hidden);
    let mut query = String::new();
    let mut shown: Vec<usize> = (0..entries.len()).collect();
    let mut selected = 0usize;
    let mut offset = 0usize;
    let mut keys = Keys::on(raw.fd());
    let mut drawn = 0usize;

    loop {
        let height = spec
            .height
            .min(shown.len().max(1))
            .min(terminal_rows().saturating_sub(4).max(1));
        if selected >= shown.len() {
            selected = shown.len().saturating_sub(1);
        }
        if selected < offset {
            offset = selected;
        } else if selected >= offset + height {
            offset = selected + 1 - height;
        }

        let cols = terminal_cols();
        let mut frame = String::new();
        if drawn > 0 {
            frame.push_str(&format!("\x1b[{drawn}A"));
        }
        frame.push_str(&format!(
            "\r\x1b[K{}\r\n",
            ui.question
                .paint(&truncate_to_width(&at.display().to_string(), cols), depth)
        ));
        frame.push_str(&format!(
            "\r\x1b[K{} {}\r\n",
            ui.accent.paint("❯", depth),
            if query.is_empty() {
                ui.muted.paint("type to filter", depth)
            } else {
                query.clone()
            }
        ));
        for row in 0..height {
            let text = match shown.get(offset + row) {
                Some(&index) => {
                    let entry = &entries[index];
                    let here = offset + row == selected;
                    // A trailing slash says which rows Right will go into, without a second column.
                    let label = if entry.directory {
                        format!("{}/", entry.name)
                    } else {
                        entry.name.clone()
                    };
                    format!(
                        "{}{}",
                        ui.accent.paint(if here { "❯ " } else { "  " }, depth),
                        if here {
                            ui.accent
                        } else if entry.directory {
                            ui.question
                        } else {
                            theme::Style::default()
                        }
                        .paint(&truncate_to_width(&label, cols.saturating_sub(3)), depth)
                    )
                }
                None => String::new(),
            };
            frame.push_str(&format!("\r\x1b[K{text}\r\n"));
        }
        frame.push_str(&format!(
            "\r\x1b[K{}",
            legend(&[("↑↓", "move"), ("←→", "in/out"), ("enter", "choose")])
        ));
        show(&frame);
        drawn = height + 3;

        let Some(pressed) = keys.read() else {
            erase(drawn);
            return Answer::Cancelled;
        };

        // Moving between directories is three things at once: reread, refilter, and put the
        // cursor back at the top. Doing them together is what keeps the three in step.
        let go = |to: PathBuf,
                  at: &mut PathBuf,
                  entries: &mut Vec<Entry>,
                  shown: &mut Vec<usize>,
                  query: &mut String,
                  selected: &mut usize,
                  offset: &mut usize| {
            *at = to;
            *entries = read(at, spec.hidden);
            query.clear();
            *shown = (0..entries.len()).collect();
            *selected = 0;
            *offset = 0;
        };

        match pressed {
            Key::Cancel => {
                erase(drawn);
                return Answer::Cancelled;
            }
            Key::Accept => {
                let Some(&index) = shown.get(selected) else {
                    continue;
                };
                let entry = &entries[index];
                let full = at.join(&entry.name);
                // Enter on a directory means "go in" unless directories are what is wanted.
                if entry.directory && spec.want == Want::Files {
                    go(
                        full,
                        &mut at,
                        &mut entries,
                        &mut shown,
                        &mut query,
                        &mut selected,
                        &mut offset,
                    );
                    continue;
                }
                if !entry.directory && spec.want == Want::Directories {
                    continue;
                }
                erase(drawn);
                return Answer::Given(full.display().to_string());
            }
            Key::Right => {
                if let Some(&index) = shown.get(selected)
                    && entries[index].directory
                {
                    let full = at.join(&entries[index].name);
                    go(
                        full,
                        &mut at,
                        &mut entries,
                        &mut shown,
                        &mut query,
                        &mut selected,
                        &mut offset,
                    );
                }
            }
            Key::Left => {
                if let Some(parent) = at.parent().map(Path::to_path_buf) {
                    go(
                        parent,
                        &mut at,
                        &mut entries,
                        &mut shown,
                        &mut query,
                        &mut selected,
                        &mut offset,
                    );
                }
            }
            Key::Up => selected = selected.saturating_sub(1),
            Key::Down => selected = (selected + 1).min(shown.len().saturating_sub(1)),
            Key::PageUp | Key::Home => selected = 0,
            Key::PageDown | Key::End => selected = shown.len().saturating_sub(1),
            Key::Char(c) => {
                query.push(c);
                shown = narrow(&entries, &query, spec.fuzzy);
                selected = 0;
                offset = 0;
            }
            Key::Backspace => {
                query.pop();
                shown = narrow(&entries, &query, spec.fuzzy);
                selected = 0;
                offset = 0;
            }
            _ => {}
        }
    }
}

fn narrow(entries: &[Entry], query: &str, fuzzy: Fuzzy) -> Vec<usize> {
    if query.is_empty() {
        return (0..entries.len()).collect();
    }
    let pattern = Fuzzed::new(query, fuzzy);
    let mut scored: Vec<(i32, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| pattern.score(&entry.name).map(|s| (s, index)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, index)| index).collect()
}

fn erase(rows: usize) {
    if rows == 0 {
        return;
    }
    let mut out = format!("\x1b[{rows}A");
    for _ in 0..rows {
        out.push_str("\r\x1b[K\r\n");
    }
    out.push_str(&format!("\x1b[{rows}A"));
    show(&out);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Directories come first, then names — because moving through the tree is the common act and
    /// a name-only sort scatters the things you move through.
    #[test]
    fn directories_sort_before_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("zzz-dir")).expect("mkdir");
        std::fs::write(dir.path().join("aaa-file"), "").expect("write");
        std::fs::write(dir.path().join("bbb-file"), "").expect("write");

        let entries = read(dir.path(), false);
        assert_eq!(entries[0].name, "zzz-dir");
        assert!(entries[0].directory);
        assert_eq!(entries[1].name, "aaa-file");
        assert_eq!(entries[2].name, "bbb-file");
    }

    #[test]
    fn dotfiles_are_hidden_unless_asked_for() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(".secret"), "").expect("write");
        std::fs::write(dir.path().join("plain"), "").expect("write");

        assert_eq!(read(dir.path(), false).len(), 1);
        assert_eq!(read(dir.path(), true).len(), 2);
    }

    /// A directory that cannot be read is empty rather than a crash — a picker walking into
    /// `/proc` or somewhere unreadable must keep working.
    #[test]
    fn an_unreadable_directory_is_empty() {
        assert!(read(Path::new("/no/such/directory/anywhere"), false).is_empty());
    }

    #[test]
    fn filtering_matches_names() {
        let entries = vec![
            Entry {
                name: "alpha.txt".to_string(),
                directory: false,
            },
            Entry {
                name: "beta.rs".to_string(),
                directory: false,
            },
        ];
        assert_eq!(narrow(&entries, "alpha", Fuzzy::Smart), vec![0]);
        assert_eq!(narrow(&entries, "", Fuzzy::Smart), vec![0, 1]);
        assert!(narrow(&entries, "zzz", Fuzzy::Smart).is_empty());
    }

    #[test]
    fn without_a_terminal_it_refuses() {
        assert_eq!(file(&Browse::default()), Answer::NoTerminal);
    }
}
