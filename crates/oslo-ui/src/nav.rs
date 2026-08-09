//! The `nav` filesystem navigator.

use crate::ask::look::{Row, Step, View};
use crate::ask::{Inline, Look};
use crate::dropdown::{human_age, human_size};
use crate::matching::{Fuzzed, Fuzzy};
use crate::settings::Icons;
use crate::term::{Key, Keys, Pressed, Restore, Screen};
use crate::{ask, theme};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct Navigator {
    pub start: PathBuf,
    pub hidden: bool,
    /// Zero uses the middle half of the terminal.
    pub width: usize,
    /// Zero uses the middle half of a full screen or up to fourteen inline rows.
    pub height: usize,
    pub fuzzy: Fuzzy,
    /// What is drawn in front of each name; see [`Icons`].
    pub icons: Icons,
    pub chrome: ask::chrome::Chrome,
    pub look: Look,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    ChangeTo(PathBuf),
    Cancelled,
    NoTerminal,
}

#[derive(Debug, Clone)]
struct Entry {
    name: String,
    directory: bool,
    symlink: bool,
    size: u64,
    modified: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browse,
    Filter,
    Delete,
}

struct State {
    at: PathBuf,
    entries: Vec<Entry>,
    /// How wide the listing wants to be, measured once per directory.
    ///
    /// **Across every entry, not the ones the filter currently matches**, or the block would be as
    /// wide as the longest surviving name and would resize and re-centre under the cursor on every
    /// keystroke. Cached because it means formatting every row, which a directory of ten thousand
    /// files should not do per keypress.
    natural: Option<usize>,
    error: Option<String>,
    hidden: bool,
    query: String,
    subject: Option<String>,
    shown: Vec<usize>,
    selected: usize,
    offset: usize,
    mode: Mode,
    legend: bool,
}

impl State {
    fn new(spec: &Navigator) -> State {
        let at = spec
            .start
            .canonicalize()
            .unwrap_or_else(|_| spec.start.clone());
        let (entries, error) = read(&at, spec.hidden);
        State {
            shown: (0..entries.len()).collect(),
            at,
            entries,
            natural: None,
            error,
            hidden: spec.hidden,
            query: String::new(),
            subject: None,
            selected: 0,
            offset: 0,
            mode: Mode::Browse,
            legend: spec.chrome.legend,
        }
    }

    fn selected_name(&self) -> Option<String> {
        self.shown
            .get(self.selected)
            .and_then(|index| self.entries.get(*index))
            .map(|entry| entry.name.clone())
    }

    fn load(&mut self, to: PathBuf, highlight: Option<String>) {
        self.at = to.canonicalize().unwrap_or(to);
        (self.entries, self.error) = read(&self.at, self.hidden);
        self.natural = None;
        self.shown = (0..self.entries.len()).collect();
        self.selected = highlight
            .and_then(|name| self.entries.iter().position(|entry| entry.name == name))
            .unwrap_or(0);
        self.query.clear();
        self.subject = None;
        self.mode = Mode::Browse;
        self.offset = 0;
    }

    fn reload(&mut self, highlight: Option<String>) {
        let old_selected = self.selected;
        let selected_name = highlight.or_else(|| self.selected_name());
        (self.entries, self.error) = read(&self.at, self.hidden);
        self.natural = None;
        self.shown = (0..self.entries.len()).collect();
        self.selected = selected_name
            .and_then(|name| self.entries.iter().position(|entry| entry.name == name))
            .unwrap_or_else(|| old_selected.min(self.entries.len().saturating_sub(1)));
        self.offset = 0;
    }

    fn refilter(&mut self, fuzzy: Fuzzy) {
        self.shown = narrow(&self.entries, &self.query, fuzzy);
        self.selected = 0;
        self.offset = 0;
    }

    fn leave_mode(&mut self) {
        let selected_name = self.selected_name();
        self.shown = (0..self.entries.len()).collect();
        self.selected = selected_name
            .and_then(|name| self.entries.iter().position(|entry| entry.name == name))
            .unwrap_or(0);
        self.query.clear();
        self.subject = None;
        self.mode = Mode::Browse;
        self.offset = 0;
    }

    fn open_selected(&mut self) {
        let Some(&index) = self.shown.get(self.selected) else {
            return;
        };
        if self.entries[index].directory {
            self.load(self.at.join(&self.entries[index].name), None);
        }
    }

    fn open_parent(&mut self) {
        let Some(parent) = self.at.parent().map(Path::to_path_buf) else {
            return;
        };
        let child = self
            .at
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        self.load(parent, child);
    }

    fn begin_delete(&mut self) {
        let Some(name) = self.selected_name() else {
            return;
        };
        self.subject = Some(name);
        self.error = None;
        self.mode = Mode::Delete;
    }

    fn commit_delete(&mut self, remove: &mut impl FnMut(&Path) -> bool) {
        let Some(name) = self.subject.clone() else {
            self.leave_mode();
            return;
        };
        let path = self.at.join(&name);
        let removed = remove(&path);
        self.leave_mode();
        if removed {
            self.reload(None);
        } else {
            self.error = Some("delete failed".to_string());
        }
    }
}

fn read(at: &Path, hidden: bool) -> (Vec<Entry>, Option<String>) {
    let entries = match std::fs::read_dir(at) {
        Ok(entries) => entries,
        Err(error) => return (Vec::new(), Some(error.to_string())),
    };
    let mut out: Vec<Entry> = entries
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !hidden && name.starts_with('.') {
                return None;
            }
            let path = entry.path();
            let metadata = path.symlink_metadata().ok()?;
            Some(Entry {
                directory: path.is_dir(),
                symlink: metadata.file_type().is_symlink(),
                size: metadata.len(),

                modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                name,
            })
        })
        .collect();
    out.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.name.cmp(&b.name)));
    (out, None)
}

pub fn open(spec: &Navigator, mut remove: impl FnMut(&Path) -> bool) -> Outcome {
    let Some(raw) = Restore::enter(Screen::Inline) else {
        return Outcome::NoTerminal;
    };
    let mut state = State::new(spec);
    let mut keys = Keys::on(raw.fd());
    let mut panel = Inline::with_chrome(spec.chrome.clone());
    let since = ask::Since::now();
    let ui = theme::current().ui;
    let depth = theme::depth();

    loop {
        let filtering = state.mode != Mode::Browse;
        let mut look = spec.look.clone();
        let input = match state.mode {
            Mode::Filter => state.query.as_str(),
            Mode::Browse | Mode::Delete => "",
        };
        match state.mode {
            Mode::Browse => {}
            Mode::Filter => {
                look.left = "filter @ ".to_string();
                look.right = "{n}/{total} ".to_string();
            }
            Mode::Delete => {
                let name = state.subject.as_deref().unwrap_or("entry");
                look.left = format!("delete {name}? ");
                look.prompt.clear();
                look.placeholder = "y delete  n cancel".to_string();
                look.right.clear();
                look.scanner = None;
            }
        }

        let terminal_rows = crate::dropdown::width::terminal_rows();
        let chrome_rows = spec
            .chrome
            .extra_rows()
            .saturating_sub(spec.chrome.legend_rows())
            + if state.legend {
                spec.chrome.legend_gap + 2
            } else {
                0
            };
        let extra = 1 + chrome_rows + look.extra_rows(filtering);
        let available = terminal_rows.saturating_sub(extra + 1).max(1);
        let height = match spec.height {
            // Half the screen is the *ceiling*, not the size. A directory of nine entries asking
            // for twenty-four rows drew fifteen blank ones, and the block read as a list with a
            // hole in it rather than as a list.
            //
            // Measured against everything here rather than what the filter currently matches, so
            // typing does not resize the widget and move it under the cursor: the block is as tall
            // as the directory, and filtering empties rows inside it.
            0 if spec.chrome.fullscreen => terminal_rows
                .saturating_div(2)
                .saturating_sub(extra)
                .max(1)
                .min(available)
                .min(state.entries.len().max(1)),
            0 => available.min(state.shown.len().max(1)).min(14),
            wanted => wanted.min(available),
        };
        state.selected = state.selected.min(state.shown.len().saturating_sub(1));
        if state.selected < state.offset {
            state.offset = state.selected;
        } else if state.selected >= state.offset + height {
            state.offset = state.selected + 1 - height;
        }

        let rows: Vec<Row> = state
            .shown
            .iter()
            .map(|&index| row_of(&state.entries[index], ui.question, &spec.icons))
            .collect();

        // What the listing would take if nothing were stretched. `cols` is not read for this, and
        // neither is the height: only the counts a trail template could interpolate.
        let natural = match state.natural {
            Some(width) => width,
            None => {
                let all: Vec<Row> = state
                    .entries
                    .iter()
                    .map(|entry| row_of(entry, ui.question, &spec.icons))
                    .collect();
                let width = look
                    .natural_width(
                        &all,
                        &View {
                            selected: 0,
                            offset: 0,
                            height: 0,
                            query: "",
                            matched: state.entries.len(),
                            total: state.entries.len(),
                            marked: 0,
                            cols: 0,
                            filtering,
                            elapsed_ms: 0,
                        },
                    )
                    .max(1);
                state.natural = Some(width);
                width
            }
        };

        // **The path counts towards the width as much as the listing does.** An empty directory has
        // no rows to measure, so a width taken from the listing alone collapsed to a single column
        // and the heading rendered as one ellipsis — the whole widget reduced to `…` the moment you
        // opened a directory with nothing in it.
        let gutter = look.pad + crate::prompt::printed_width(&look.marker);
        let needed =
            natural.max(crate::prompt::printed_width(&state.at.display().to_string()) + gutter);

        let room = spec.chrome.room();
        let cols = match spec.width {
            // Half the terminal is the ceiling here too, and for the same reason it is one
            // vertically: rows are padded out to whatever width they are given, so a hundred-column
            // box holding forty-six columns of listing put fifty blank cells between every filename
            // and its age.
            0 => crate::dropdown::width::terminal_cols()
                .saturating_div(2)
                .max(1)
                .min(room)
                .min(needed),
            wanted => wanted.min(room).max(1),
        };
        // **The heading starts where the rows start.**
        //
        // A list row is `pad`, then the cursor marker, then its columns; the heading is a bare row
        // and had neither, so the path sat that many cells to the left of everything under it and
        // the block read as two things that had come apart. The offset is taken from the look
        // rather than written down, so a wider marker or more padding moves both together.
        let inset = " ".repeat(gutter);
        let cols = cols.max(gutter + 1);
        let told = cols - gutter;
        let path = crate::dropdown::width::truncate_to_width(&state.at.display().to_string(), told);
        let heading = match &state.error {
            Some(problem) => ui.error.paint(
                &crate::dropdown::width::truncate_to_width(&format!("{path}  {problem}"), told),
                depth,
            ),
            None if state.hidden => ui.question.paint(
                &crate::dropdown::width::truncate_to_width(&format!("{path}  [all]"), told),
                depth,
            ),
            None => ui.question.paint(&path, depth),
        };
        let mut frame = format!("\r\n\r\x1b[K{inset}{heading}");
        frame.push_str(&look.frame(
            &rows,
            &View {
                selected: state.selected,
                offset: state.offset,
                height,
                query: input,
                matched: state.shown.len(),
                total: state.entries.len(),
                marked: 0,
                cols,
                filtering,
                elapsed_ms: since.ms(),
            },
        ));
        panel.show_legend(state.legend);
        panel.draw(
            &frame,
            &[
                ("type", "filter"),
                ("↑↓", "move"),
                ("←→", "back/open"),
                ("↵", "open"),
                ("del", "delete"),
                ("esc", "cd+quit"),
                ("?", "hide"),
            ],
        );

        let tick = (state.mode == Mode::Filter)
            .then(|| look.tick_ms())
            .flatten();
        let pressed = match ask::awaited(&mut keys, tick) {
            Pressed::Key(key) => key,
            Pressed::Timeout => continue,
            Pressed::Ended => {
                panel.close();
                return Outcome::Cancelled;
            }
        };

        if state.mode == Mode::Delete {
            match pressed {
                Key::Char('y' | 'Y') | Key::Delete => state.commit_delete(&mut remove),
                Key::Char('n' | 'N') | Key::Cancel => state.leave_mode(),
                Key::Abort => {
                    panel.close();
                    return Outcome::Cancelled;
                }
                _ => {}
            }
            continue;
        }

        if state.mode == Mode::Filter {
            match pressed {
                Key::Cancel => {
                    panel.close();
                    return Outcome::ChangeTo(state.at);
                }
                Key::Abort => {
                    panel.close();
                    return Outcome::Cancelled;
                }
                Key::Accept | Key::Right => state.open_selected(),
                Key::Left => state.open_parent(),
                key if spec.look.step(key).is_some() => {
                    let step = spec.look.step(key).unwrap_or(Step::Back);
                    state.selected = step.from(state.selected, state.shown.len());
                }
                Key::Delete => state.begin_delete(),
                Key::Backspace => {
                    state.query.pop();
                    if state.query.is_empty() {
                        state.leave_mode();
                    } else {
                        state.refilter(spec.fuzzy);
                    }
                }
                Key::Clear => {
                    state.leave_mode();
                }
                Key::Char('?') => state.legend = !state.legend,
                Key::Char(c) => {
                    state.query.push(c);
                    state.refilter(spec.fuzzy);
                }
                _ => {}
            }
            continue;
        }

        match pressed {
            Key::Cancel => {
                panel.close();
                return Outcome::ChangeTo(state.at);
            }
            Key::Abort => {
                panel.close();
                return Outcome::Cancelled;
            }
            Key::Accept | Key::Right => state.open_selected(),
            Key::Left => state.open_parent(),
            key if spec.look.step(key).is_some() => {
                let step = spec.look.step(key).unwrap_or(Step::Back);
                state.selected = step.from(state.selected, state.shown.len());
            }
            Key::Delete => state.begin_delete(),
            Key::Char('?') => state.legend = !state.legend,
            Key::Char(c) => {
                state.query.clear();
                state.query.push(c);
                state.mode = Mode::Filter;
                state.refilter(spec.fuzzy);
            }
            _ => {}
        }
    }
}

/// One row: a mark, the name, and — hard right — how big it is and how long ago it changed.
///
/// **The `dir`/`file` word and the `rwxrwxr-x` mode are gone.** Both were `ls -l` habits rather
/// than answers anybody wanted here: the kind is already said by the trailing `/` on a directory
/// and `@` on a link, and nine characters of mode is a question a file browser is rarely asked —
/// while together they pushed the name a third of the way across the row. What replaces them is
/// one configurable mark; see [`Icons`].
///
/// The size moved to the right for the same reason. A file browser is read down the *names*, and
/// anything to the left of them is something the eye has to skip on every row to get there. On the
/// right it sits beside the age, which is the other number, and the names all start in one place.
fn row_of(entry: &Entry, directory_style: theme::Style, icons: &Icons) -> Row {
    let name = match (entry.directory, entry.symlink) {
        (true, true) => format!("{}@/", entry.name),
        (true, false) => format!("{}/", entry.name),
        (false, true) => format!("{}@", entry.name),
        (false, false) => entry.name.clone(),
    };
    Row {
        // The one column left of the name, where the `dir`/`file` word was, so the mark leads the
        // row and lines up down the list. An icon set to the empty string costs no column at all —
        // `meta` is sized across the listing — so turning the marks off is something a config can
        // actually say.
        meta: vec![icons.of(&entry.name, entry.directory).to_string()],
        // Padded rather than merely concatenated: `trail` is one string drawn hard right, so two
        // numbers only read as two columns if each is given a fixed width of its own. Five holds
        // every size `human_size` produces (`1023B`), four holds every age (`11mo`).
        trail: format!(
            "  {:>5}  {:>4}",
            human_size(entry.size),
            human_age(entry.modified)
        ),
        tint: entry.directory.then_some(directory_style),
        ..Row::new(name)
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
        .filter_map(|(index, entry)| pattern.score(&entry.name).map(|score| (score, index)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, index)| index).collect()
}

#[cfg(test)]
#[path = "nav/tests.rs"]
mod tests;
