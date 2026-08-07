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

use super::look::{Row, Step, View};
use super::{Answer, Inline};
use crate::interactive::dropdown::width::{terminal_rows, truncate_to_width};
use crate::interactive::matching::{Fuzzed, Fuzzy};
use crate::interactive::term::{Key, Keys, Pressed, Restore, Screen};
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
    /// The legend, the border, the screen and where on it. See `super::chrome`.
    pub chrome: super::chrome::Chrome,
    /// Where the filter sits and what colour the rows take. See `super::look`.
    pub look: super::look::Look,
}

impl Default for Browse {
    fn default() -> Self {
        Browse {
            start: PathBuf::from("."),
            want: Want::Files,
            hidden: false,
            height: 12,
            fuzzy: Fuzzy::Smart,
            chrome: super::chrome::Chrome::default(),
            look: super::look::Look::default(),
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

    let Some(raw) = Restore::enter(Screen::Inline) else {
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
    let mut panel = Inline::with_chrome(spec.chrome.clone());
    let since = super::Since::now();

    loop {
        // The path above, whatever the look puts around the list, the footer below, and one spare
        // so the block can never fill the screen exactly.
        let chrome = 1 + spec.chrome.extra_rows() + spec.look.extra_rows(true);
        let height = spec
            .height
            .min(shown.len().max(1))
            .min(terminal_rows().saturating_sub(chrome + 1).max(1));
        if selected >= shown.len() {
            selected = shown.len().saturating_sub(1);
        }
        if selected < offset {
            offset = selected;
        } else if selected >= offset + height {
            offset = selected + 1 - height;
        }

        let cols = spec.chrome.room();
        let mut frame = String::new();
        frame.push_str(&format!(
            "\r\n\r\x1b[K{}",
            ui.question
                .paint(&truncate_to_width(&at.display().to_string(), cols), depth)
        ));
        let rows: Vec<Row> = shown
            .iter()
            .map(|&index| {
                let entry = &entries[index];
                Row {
                    // A trailing slash says which rows Right will go into, without a second
                    // column.
                    text: match entry.directory {
                        true => format!("{}/", entry.name),
                        false => entry.name.clone(),
                    },
                    tint: entry.directory.then_some(ui.question),
                    ..Row::new(String::new())
                }
            })
            .collect();
        frame.push_str(&spec.look.frame(
            &rows,
            &View {
                selected,
                offset,
                height,
                query: &query,
                matched: shown.len(),
                total: entries.len(),
                marked: 0,
                cols,
                filtering: true,
                elapsed_ms: since.ms(),
            },
        ));
        panel.draw(
            &frame,
            &[("↑↓", "move"), ("←→", "in/out"), ("enter", "choose")],
        );

        let pressed = match super::awaited(&mut keys, spec.look.tick_ms()) {
            Pressed::Key(key) => key,
            Pressed::Timeout => continue,
            Pressed::Ended => {
                panel.close();
                return Answer::Cancelled;
            }
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
            // An abort is a cancel here: there is an answer to decline either way.
            Key::Cancel | Key::Abort => {
                panel.close();
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
                panel.close();
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
            // The arrows follow the screen, not the list — see `Look::step`.
            key if spec.look.step(key).is_some() => {
                let step = spec.look.step(key).unwrap_or(Step::Back);
                selected = step.from(selected, shown.len());
            }
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
