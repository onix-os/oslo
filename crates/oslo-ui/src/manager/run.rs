//! Opening the manager: raw mode, the key loop, and putting the terminal back.
//!
//! The finder's rule, unchanged and for its reason: **whatever happens, the terminal is restored** —
//! the alternate screen, the cursor and the original termios, on every path out including the ones
//! nobody plans for. It is a guard that runs on drop rather than a line at the end of the loop.

use super::render::{Frame, frame, visible_rows};
use super::{Act, Backing, Item, Source};
use crate::matching::{Fuzzed, Fuzzy};
use crate::term::{Key, Keys, Pressed, Restore, Screen};
use std::io::{self, Write};
use std::time::{Duration, Instant};

/// Three presses inside this window are the gesture; a fourth starts a new one.
///
/// **Long enough to be deliberate, short enough not to catch two separate decisions.** A person
/// pressing space three times to mean one thing does it in well under half a second; a person
/// turning one macro off, thinking, and turning it off again is nowhere near this.
const BURST: Duration = Duration::from_millis(600);

/// What the manager was left with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Enter: open this one in the editor. The query comes back too, so reopening the screen
    /// afterwards lands on the same narrowed list rather than at the top of everything.
    Edit { item: Item, query: String },
    /// Esc, or Ctrl-C.
    Cancelled,
}

/// Open the manager over `items` and run it until it is dismissed.
///
/// `None` when there is no terminal to draw on — which is what makes `oslo macros show | cat` print
/// a list instead of trying to take over a screen that is not there.
pub fn open(items: Vec<Item>, seed: &str, backing: &mut dyn Backing) -> Option<Outcome> {
    let restore = Restore::enter(Screen::Alternate)?;
    let opened = Instant::now();
    let mut stdout = io::stdout();
    let mut state = State::new(items, seed);
    // Every tag in the whole set, not just the visible source: a tag has to keep its colour when
    // Tab moves to the other list, or the same word is two colours on two screens.
    let all_tags = state.all_tags();
    let mut keys = Keys::on(restore.fd());
    let mut last = String::new();

    loop {
        let (cols, rows) = terminal_size();
        state.fit(rows);
        let painted = frame(&Frame {
            rows: &state.shown,
            selected: state.selected,
            offset: state.offset,
            query: &state.query,
            elapsed_ms: opened.elapsed().as_millis() as u64,
            confirm: state.confirm,
            source: state.source,
            kind: state.kind(),
            total: state.total(),
            cols,
            rows_available: rows,
            now: super::render::now(),
            tags: &all_tags,
        });
        if painted != last {
            let _ = stdout.write_all(painted.as_bytes());
            let _ = stdout.flush();
            last = painted;
        }

        let pressed = match keys.read_within(120) {
            Pressed::Key(key) => key,
            Pressed::Timeout => continue,
            Pressed::Ended => return Some(Outcome::Cancelled),
        };

        // While the question is up it owns the keyboard, as it does in the finder: anything else
        // typed would filter a list you cannot see the bar for.
        if let Some(yes) = state.confirm {
            match pressed {
                Key::Left | Key::Right | Key::ToggleScope | Key::BackTab => {
                    state.confirm = Some(!yes);
                }
                Key::Accept => {
                    if yes {
                        state.forget_selected(backing);
                    }
                    state.confirm = None;
                }
                Key::Cancel | Key::Abort => state.confirm = None,
                _ => {}
            }
            continue;
        }

        match pressed {
            Key::Cancel | Key::Abort => return Some(Outcome::Cancelled),
            // **Only a stored row can be edited or forgotten.** An inherited one is a fact about
            // this shell — an alias your config defined, a variable your profile exported — and
            // there is no row in the database to open in an editor or to delete. Opening one
            // anyway would write a *new* macro that shadows it, which is a different thing from
            // what Enter looks like it does; doing nothing is the honest answer, and the status
            // line says so before the key is pressed.
            Key::Accept | Key::Delete if state.selected().is_some_and(|item| !item.stored) => {}
            Key::Accept => {
                let item = state.selected()?.clone();
                return Some(Outcome::Edit {
                    item,
                    query: state.query.clone(),
                });
            }
            Key::Up => state.up(),
            Key::Down => state.down(),
            Key::PageUp => state.page_up(),
            Key::PageDown => state.page_down(),
            Key::Right => state.next_kind(),
            Key::Left => state.previous_kind(),
            Key::ToggleScope => state.next_source(),
            Key::Delete => {
                if state.selected().is_some() {
                    state.confirm = Some(false);
                }
            }
            Key::Backspace => {
                state.query.pop();
                state.refilter();
            }
            Key::Clear => {
                state.query.clear();
                state.refilter();
            }
            // **The one key that is two keys.** A space is the common case and stays instant; the
            // third inside the window upgrades what the first two did. See `press_space`.
            Key::Char(' ') => state.press_space(backing),
            Key::Char(c) => {
                state.query.push(c);
                state.refilter();
            }
            _ => {}
        }
    }
}

fn terminal_size() -> (usize, usize) {
    (
        crate::dropdown::width::terminal_cols().max(20),
        crate::dropdown::width::terminal_rows().max(6),
    )
}

/// A burst of spaces on one row.
struct Burst {
    on: String,
    presses: usize,
    at: Instant,
    /// What the session state was before the burst began, so the third press can put back what the
    /// first two did rather than leaving the row in whatever state the counting left it.
    was_off: bool,
}

/// A query split into the tags it asks for and the text that is matched on.
///
/// `#ai tool`, `tool #ai` and `#ai` all mean the same thing: everything tagged `ai`, narrowed by
/// `tool` where a word is left over. **A tag is a set you are asking for, not a word you hope
/// appears in a body**, which is why it is spelled differently — and doing it in the query is what
/// leaves the arrows free for the kind, with no second key to learn.
///
/// A bare `#` is somebody mid-word and asks for nothing yet.
fn split_tags(query: &str) -> (Vec<String>, String) {
    let mut tags = Vec::new();
    let mut text = Vec::new();
    for word in query.split_whitespace() {
        match word.strip_prefix('#') {
            Some("") | None => text.push(word),
            Some(tag) => tags.push(tag.to_string()),
        }
    }
    (tags, text.join(" "))
}

/// The manager's state between keystrokes.
struct State {
    /// Owned: Delete removes a row and a toggle changes one, and a slice cannot do either.
    items: Vec<Item>,
    query: String,
    source: Source,
    /// Index into the kinds in use, `0` being all of them.
    ///
    /// **What ← and → move through**, because the kind is the division you are actually navigating:
    /// five hundred inherited variables and forty aliases are one list until something separates
    /// them, and the kind is what tells them apart. Tags are the finer cut and are asked for in the
    /// query — `#ai` — which needs no key at all. See [`split_tags`].
    kind_at: usize,
    /// The rows on screen, after the source, the kind and the query have had their say.
    shown: Vec<Item>,
    selected: usize,
    offset: usize,
    window: usize,
    confirm: Option<bool>,
    burst: Option<Burst>,
}

impl State {
    fn new(items: Vec<Item>, seed: &str) -> State {
        let mut state = State {
            items,
            query: seed.trim().to_string(),
            source: Source::Stored,
            kind_at: 0,
            shown: Vec::new(),
            selected: 0,
            offset: 0,
            window: 1,
            confirm: None,
            burst: None,
        };
        state.refilter();
        state
    }

    /// Every tag in the whole set, whichever source it came from.
    ///
    /// What the colours are handed out by — so a tag keeps its colour when Tab moves to the other
    /// list, which the source-filtered [`State::tags`] could not promise.
    fn all_tags(&self) -> Vec<String> {
        let mut found: Vec<String> = self
            .items
            .iter()
            .flat_map(|item| item.tags.clone())
            .collect();
        found.sort();
        found.dedup();
        found
    }

    /// The tag being shown, or `None` for all of them.
    /// The kinds in use in the current source, in the order they are cycled through.
    ///
    /// Drawn from what is actually there, so a list with no scripts in it does not offer to show
    /// you the scripts. `0` is all of them.
    fn kinds(&self) -> Vec<String> {
        let mut found: Vec<String> = self
            .items
            .iter()
            .filter(|item| self.source.holds(item))
            .map(|item| item.kind.clone())
            .collect();
        found.sort();
        found.dedup();
        found
    }

    /// The kind being shown, or `None` for all of them.
    fn kind(&self) -> Option<String> {
        self.kind_at
            .checked_sub(1)
            .and_then(|at| self.kinds().get(at).cloned())
    }

    fn refilter(&mut self) {
        let kind = self.kind();
        // `#ai tool`, `tool #ai`, or just `#ai`: the tags are taken out of the query and the rest
        // is what gets fuzzy-matched. A tag is a set you are asking for, not a word you are hoping
        // appears somewhere in the body — and there is no key to learn, which is why the arrows are
        // free for the kind.
        let (tags, text) = split_tags(&self.query);
        let fuzzed = Fuzzed::new(&text, Fuzzy::Smart);
        let mut found: Vec<(i32, Item)> = self
            .items
            .iter()
            .filter(|item| self.source.holds(item))
            .filter(|item| kind.as_ref().is_none_or(|kind| &item.kind == kind))
            .filter(|item| tags.iter().all(|tag| item.tags.contains(tag)))
            .filter_map(|item| {
                // The best field wins, and the name is worth more than the rest: `gs` should find
                // the macro called `gs` before one whose body happens to mention it.
                let best = item
                    .fields()
                    .iter()
                    .enumerate()
                    .filter_map(|(at, field)| {
                        let score = fuzzed.score(field)?;
                        Some(if at == 0 { score + 100 } else { score })
                    })
                    .max()?;
                Some((best, item.clone()))
            })
            .collect();
        // Newest first, and the score only breaks a tie. The list is short and every row already
        // matches, so ordering by how well would mean the rows moving under you as you type — the
        // finder's `rank` says the same thing at greater length, and for the same reason.
        found.sort_by(|(a_score, a), (b_score, b)| {
            b.created
                .cmp(&a.created)
                .then_with(|| b_score.cmp(a_score))
                .then_with(|| a.name.cmp(&b.name))
        });
        self.shown = found.into_iter().map(|(_, item)| item).collect();
        self.selected = 0;
        self.offset = 0;
    }

    fn total(&self) -> usize {
        self.items
            .iter()
            .filter(|item| self.source.holds(item))
            .count()
    }

    fn selected(&self) -> Option<&Item> {
        self.shown.get(self.selected)
    }

    fn next_source(&mut self) {
        self.source = self.source.other();
        // The kind belonged to the old source's list of kinds, and the new one has its own — the
        // stored list has scripts in it and the inherited one has five hundred variables.
        self.kind_at = 0;
        self.refilter();
    }

    fn next_kind(&mut self) {
        let count = self.kinds().len() + 1;
        self.kind_at = (self.kind_at + 1) % count;
        self.refilter();
    }

    fn previous_kind(&mut self) {
        let count = self.kinds().len() + 1;
        self.kind_at = (self.kind_at + count - 1) % count;
        self.refilter();
    }

    /// Space: off for the session. Three inside [`BURST`]: off everywhere.
    ///
    /// The third press first puts the session state back where the burst found it, then flips
    /// `active` — so what you are left with is one change and not three.
    fn press_space(&mut self, backing: &mut dyn Backing) {
        let Some(item) = self.selected().cloned() else {
            return;
        };
        let now = Instant::now();
        let carried = self
            .burst
            .take()
            .filter(|burst| burst.on == item.key() && now.duration_since(burst.at) < BURST);
        let presses = carried.as_ref().map_or(0, |burst| burst.presses) + 1;
        let was_off = carried.map_or(item.session_off, |burst| burst.was_off);

        if presses >= 3 {
            if item.session_off != was_off {
                backing.act(&item, Act::Session(was_off));
                self.change(&item.key(), |row| row.session_off = was_off);
            }
            let active = !item.active;
            backing.act(&item, Act::Everywhere(active));
            self.change(&item.key(), |row| row.active = active);
            self.burst = None;
            return;
        }

        let off = !item.session_off;
        backing.act(&item, Act::Session(off));
        self.change(&item.key(), |row| row.session_off = off);
        self.burst = Some(Burst {
            on: item.key(),
            presses,
            at: now,
            was_off,
        });
    }

    /// Change a row in both lists, so the screen shows it without a reload.
    fn change(&mut self, key: &str, edit: impl Fn(&mut Item)) {
        for row in self.items.iter_mut().filter(|row| row.key() == key) {
            edit(row);
        }
        for row in self.shown.iter_mut().filter(|row| row.key() == key) {
            edit(row);
        }
    }

    fn forget_selected(&mut self, backing: &mut dyn Backing) {
        let Some(item) = self.selected().cloned() else {
            return;
        };
        // **Nothing to forget.** An inherited row has no record in the database; removing it would
        // be removing something from a place it was never in, and the row would come back the next
        // time the shell published what your config defines. The key loop refuses before the
        // question is asked, and this refuses again — a guard that only lives in the key loop is
        // one the next caller does not have.
        if !item.stored {
            return;
        }
        backing.act(&item, Act::Forget);
        self.items.retain(|row| row.key() != item.key());
        let was = self.selected;
        self.refilter();
        // Back to where the eye was: a *query* change makes the old index meaningless, a deletion
        // does not — the rows around it are the same ones.
        self.selected = was.min(self.shown.len().saturating_sub(1));
    }

    fn fit(&mut self, rows: usize) {
        self.window = visible_rows(rows);
        if self.selected >= self.offset + self.window {
            self.offset = self.selected + 1 - self.window;
        }
        if self.selected < self.offset {
            self.offset = self.selected;
        }
    }

    fn up(&mut self) {
        self.selected = (self.selected + 1).min(self.shown.len().saturating_sub(1));
    }

    fn down(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn page_up(&mut self) {
        self.selected = (self.selected + self.window).min(self.shown.len().saturating_sub(1));
    }

    fn page_down(&mut self) {
        self.selected = self.selected.saturating_sub(self.window);
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
