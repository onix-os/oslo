//! The `nav` filesystem navigator.

use crate::ui::ask::look::{Row, Step, View};
use crate::ui::ask::{Inline, Look};
use crate::ui::dropdown::{human_age, human_mode, human_size};
use crate::ui::matching::{Fuzzed, Fuzzy};
use crate::ui::term::{Key, Keys, Pressed, Restore, Screen};
use crate::ui::{ask, theme};
use std::os::unix::fs::PermissionsExt;
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
    mode: u32,
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
                mode: metadata.permissions().mode(),
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

        let terminal_rows = crate::ui::dropdown::width::terminal_rows();
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
            0 if spec.chrome.fullscreen => terminal_rows
                .saturating_div(2)
                .saturating_sub(extra)
                .max(1)
                .min(available),
            0 => available.min(state.shown.len().max(1)).min(14),
            wanted => wanted.min(available),
        };
        state.selected = state.selected.min(state.shown.len().saturating_sub(1));
        if state.selected < state.offset {
            state.offset = state.selected;
        } else if state.selected >= state.offset + height {
            state.offset = state.selected + 1 - height;
        }

        let room = spec.chrome.room();
        let cols = match spec.width {
            0 => crate::ui::dropdown::width::terminal_cols()
                .saturating_div(2)
                .max(1)
                .min(room),
            wanted => wanted.min(room).max(1),
        };
        // **The heading starts where the rows start.**
        //
        // A list row is `pad`, then the cursor marker, then its columns; the heading is a bare row
        // and had neither, so the path sat that many cells to the left of everything under it and
        // the block read as two things that had come apart. The offset is taken from the look
        // rather than written down, so a wider marker or more padding moves both together.
        let gutter = look.pad + crate::ui::prompt::printed_width(&look.marker);
        let inset = " ".repeat(gutter);
        let cols = cols.max(gutter + 1);
        let told = cols - gutter;
        let path =
            crate::ui::dropdown::width::truncate_to_width(&state.at.display().to_string(), told);
        let heading = match &state.error {
            Some(problem) => ui.error.paint(
                &crate::ui::dropdown::width::truncate_to_width(&format!("{path}  {problem}"), told),
                depth,
            ),
            None if state.hidden => ui.question.paint(
                &crate::ui::dropdown::width::truncate_to_width(&format!("{path}  [all]"), told),
                depth,
            ),
            None => ui.question.paint(&path, depth),
        };
        let mut frame = format!("\r\n\r\x1b[K{inset}{heading}");
        let rows: Vec<Row> = state
            .shown
            .iter()
            .map(|&index| row_of(&state.entries[index], ui.question))
            .collect();
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

fn row_of(entry: &Entry, directory_style: theme::Style) -> Row {
    let kind = match (entry.directory, entry.symlink) {
        (true, true) => "link/",
        (true, false) => "dir",
        (false, true) => "link",
        (false, false) => "file",
    };
    let name = match (entry.directory, entry.symlink) {
        (true, true) => format!("{}@/", entry.name),
        (true, false) => format!("{}/", entry.name),
        (false, true) => format!("{}@", entry.name),
        (false, false) => entry.name.clone(),
    };
    Row {
        meta: vec![
            kind.to_string(),
            human_mode(entry.mode),
            human_size(entry.size),
        ],
        trail: format!("  {}", human_age(entry.modified)),
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
