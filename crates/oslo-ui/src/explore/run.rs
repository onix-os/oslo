//! Opening the explorer: raw mode, the key loop, and putting the terminal back.
//!
//! **Whatever happens, the terminal is restored.** The alternate screen, the cursor and the original
//! termios are undone on every path out, including the ones nobody plans for — the guard runs on
//! drop, so the loop below cannot forget it. The same rule the history finder is written to, and for
//! the same reason: a viewer that leaves the screen in its own mode leaves a shell you cannot type
//! into.

use super::render::{Frame, fitting, frame, look, widths, window};
use super::{Cell, Sheet};
use crate::dropdown::width::{terminal_cols, terminal_rows};
use crate::matching::{Fuzzed, Fuzzy};
use crate::term::{Key, Keys, Pressed, Restore, Screen};

/// How the explorer was left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Read, and closed.
    Closed,
    /// There were no rows. A viewer that opened onto an empty screen would look like a hang.
    Empty,
    /// Nothing to draw on: no terminal, and no `/dev/tty` either.
    NoTerminal,
}

/// One level of the descent: a table, and where you were in it.
///
/// Kept whole on the way down so backing out puts the cursor back where it was rather than at the
/// top of a table you had already scrolled through.
struct Level {
    sheet: Sheet,
    row: usize,
    column: usize,
    top: usize,
    left: usize,
    query: String,
    shown: Vec<usize>,
}

impl Level {
    fn new(sheet: Sheet) -> Level {
        let shown = (0..sheet.rows.len()).collect();
        Level {
            sheet,
            row: 0,
            column: 0,
            top: 0,
            left: 0,
            query: String::new(),
            shown,
        }
    }

    /// The rows the query leaves, **in the order they arrived**.
    ///
    /// Not ranked by score, unlike the history finder. Row order in a table is data — `sort-by` put
    /// it there, or the producer did — and a filter that quietly re-sorted would answer a different
    /// question than the one the pipeline asked.
    fn narrow(&mut self, fuzzy: Fuzzy) {
        if self.query.is_empty() {
            self.shown = (0..self.sheet.rows.len()).collect();
        } else {
            let matcher = Fuzzed::new(&self.query, fuzzy);
            self.shown = self
                .sheet
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.iter().any(|cell| matcher.score(cell.text()).is_some()))
                .map(|(at, _)| at)
                .collect();
        }
        self.row = self.row.min(self.shown.len().saturating_sub(1));
        self.top = self.top.min(self.row);
    }

    /// The table under the cursor, if the cell has one.
    fn under_the_cursor(&self) -> Option<&Sheet> {
        self.shown
            .get(self.row)
            .and_then(|index| self.sheet.rows[*index].get(self.column))
            .and_then(Cell::sheet)
    }
}

/// Open `sheet` and read keys until it is dismissed.
pub fn open(sheet: Sheet, fuzzy: Fuzzy) -> Outcome {
    if sheet.rows.is_empty() {
        return Outcome::Empty;
    }
    let Some(restore) = Restore::enter(Screen::Alternate) else {
        return Outcome::NoTerminal;
    };

    let mut stack = vec![Level::new(sheet)];
    let mut keys = Keys::on(restore.fd());
    // The last frame written, so a keystroke that changed nothing does not repaint the screen. The
    // sweep in the search bar moves on its own, so most frames do differ — but it holds still at
    // each end of its travel, and those are the ones this saves.
    let mut last = String::new();
    // When the viewer opened, so the sweep knows how far through its stroke it is. Taken once: the
    // animation is a function of elapsed time, not of a counter to keep.
    let opened = std::time::Instant::now();
    let tick = look().tick_ms();

    loop {
        let (cols, rows) = (terminal_cols(), terminal_rows());
        let trail: Vec<String> = stack
            .iter()
            .take(stack.len() - 1)
            .map(|level| level.sheet.title.clone())
            .collect();
        let here = stack.last_mut().expect("a level is always open");
        let height = window(rows);
        let measured = widths(&here.sheet);

        // **Both viewports are clamped here, once, before drawing.** Doing it at each key would
        // mean every movement key repeating the same four lines, and a terminal resized between
        // frames would leave the cursor outside a window nothing had corrected.
        here.row = here.row.min(here.shown.len().saturating_sub(1));
        here.top = here.top.min(here.row);
        if here.row >= here.top + height {
            here.top = here.row + 1 - height;
        }
        here.column = here.column.min(here.sheet.columns.len().saturating_sub(1));
        here.left = here.left.min(here.column);
        while here.column >= here.left + fitting(&measured, here.left, cols) {
            here.left += 1;
        }

        let painted = frame(&Frame {
            sheet: &here.sheet,
            shown: &here.shown,
            row: here.row,
            column: here.column,
            top: here.top,
            left: here.left,
            query: &here.query,
            trail: &trail,
            cols,
            rows,
            elapsed_ms: opened.elapsed().as_millis() as u64,
        });
        if painted != last {
            crate::ask::show(&painted);
            last.clone_from(&painted);
        }

        // **Waited for with a deadline, not blocked on.** A blocking read would freeze the sweep
        // between keystrokes, which is the opposite of what an animation is for. The finder waits
        // the same way, on the same number.
        let pressed = match tick {
            Some(ms) => match keys.read_within(ms) {
                Pressed::Key(key) => key,
                Pressed::Timeout => continue,
                Pressed::Ended => return Outcome::Closed,
            },
            None => match keys.read() {
                Some(key) => key,
                None => return Outcome::Closed,
            },
        };
        match pressed {
            Key::Cancel | Key::Abort => return Outcome::Closed,
            Key::Up => here.row = here.row.saturating_sub(1),
            Key::Down => here.row = (here.row + 1).min(here.shown.len().saturating_sub(1)),
            Key::Left => here.column = here.column.saturating_sub(1),
            Key::Right => {
                here.column = (here.column + 1).min(here.sheet.columns.len().saturating_sub(1))
            }
            Key::PageUp => here.row = here.row.saturating_sub(height),
            Key::PageDown => here.row = (here.row + height).min(here.shown.len().saturating_sub(1)),
            Key::Home => here.row = 0,
            Key::End => here.row = here.shown.len().saturating_sub(1),
            // Down into the cell under the cursor. A cell with nothing under it does nothing rather
            // than opening an empty level, which is why the legend only offers `enter` where there
            // is something to open.
            Key::Accept => {
                if let Some(sheet) = here.under_the_cursor() {
                    let sheet = sheet.clone();
                    stack.push(Level::new(sheet));
                    last.clear();
                }
            }
            // Backspace is the filter's own key first and the way out second: with a query up,
            // deleting it is what the key means everywhere else in this shell.
            Key::Backspace => {
                if !here.query.is_empty() {
                    here.query.pop();
                    here.narrow(fuzzy);
                } else if stack.len() > 1 {
                    stack.pop();
                    last.clear();
                }
            }
            Key::Clear => {
                here.query.clear();
                here.narrow(fuzzy);
            }
            Key::Char(c) => {
                here.query.push(c);
                here.narrow(fuzzy);
            }
            _ => {}
        }
    }
}
