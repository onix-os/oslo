//! `ui choose` and `ui filter` — pick from a list.
//!
//! One widget, two doors. `choose` shows the list and moves through it; `filter` does the same and
//! also narrows it as you type. Keeping them one implementation is what makes typing in a `choose`
//! do the obvious thing instead of nothing.
//!
//! # Drawn in place, not on the alternate screen
//!
//! The list is printed below the question and erased when you leave, so the answer stays in the
//! scrollback and the rest of the screen is untouched. The history finder takes the whole screen
//! because you go there to browse; a script asking you to pick a branch should not clear what you
//! were reading.
//!
//! # Multi-select
//!
//! Space checks a row, Enter takes everything checked — or the row under the cursor when nothing
//! is. That last rule is what stops "I pressed Enter and got nothing" from being a state.

use super::look::{Row, Step, View};
use super::{Answer, Inline};
use crate::ui::dropdown::width::{terminal_rows, truncate_to_width};
use crate::ui::matching::{Fuzzed, Fuzzy, Quality};
use crate::ui::term::{Key, Keys, Pressed, Restore, Screen};
use crate::ui::theme;

/// How a list is asked.
#[derive(Debug, Clone)]
pub struct Choice {
    pub header: String,
    pub items: Vec<String>,
    /// Let the person check more than one.
    pub multi: bool,
    /// Narrow the list as they type.
    pub filter: bool,
    /// Rows of list to show at once.
    pub height: usize,
    /// How loosely the filter matches.
    pub fuzzy: Fuzzy,
    /// The legend, the border, the screen and where on it. See `super::chrome`.
    pub chrome: super::chrome::Chrome,
    /// Where the filter sits and what colour the rows take. See `super::look`.
    pub look: super::look::Look,
}

impl Default for Choice {
    fn default() -> Self {
        Choice {
            header: String::new(),
            items: Vec::new(),
            multi: false,
            filter: false,
            height: 10,
            fuzzy: Fuzzy::Smart,
            chrome: super::chrome::Chrome::default(),
            look: super::look::Look::default(),
        }
    }
}

/// Show the list and move through it.
pub fn choose(spec: &Choice) -> Answer<Vec<String>> {
    run(spec)
}

/// The same list, narrowed as you type.
pub fn filter(spec: &Choice) -> Answer<Vec<String>> {
    run(&Choice {
        filter: true,
        ..spec.clone()
    })
}

fn run(spec: &Choice) -> Answer<Vec<String>> {
    if spec.items.is_empty() {
        // Nothing to choose from is not a question. Answering "cancelled" keeps
        // `x=$(… | ui choose) || exit` correct when the pipeline produced no lines.
        return Answer::Cancelled;
    }
    let ui = theme::current().ui;
    let depth = theme::depth();

    let Some(raw) = Restore::enter(Screen::Inline) else {
        return Answer::NoTerminal;
    };

    let mut query = String::new();
    let mut shown: Vec<usize> = (0..spec.items.len()).collect();
    let mut checked = vec![false; spec.items.len()];
    let mut selected = 0usize;
    let mut offset = 0usize;
    let mut keys = Keys::on(raw.fd());
    let mut panel = Inline::with_chrome(spec.chrome.clone());
    let since = super::Since::now();

    loop {
        // The chrome is whatever this frame will actually draw — the legend, plus a header and
        // whatever rows the look puts around the list. Computed from the same values the drawing
        // uses, so the clamp and the frame cannot disagree; a hard-coded constant here is how two
        // of these widgets ended up reserving a row they never drew.
        let chrome = spec.chrome.extra_rows()
            + usize::from(!spec.header.is_empty())
            + spec.look.extra_rows(spec.filter);
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

        // Every row starts with `\r\n`, which is what makes the row count exact — see `Inline`.
        let mut frame = String::new();
        // Truncated like everything else: a header or a query longer than the terminal wraps, and
        // a wrapped row is two physical rows the count knows nothing about. The width comes from
        // the chrome, which is the only thing that knows what the border and padding will take.
        let room = spec.chrome.room();

        if !spec.header.is_empty() {
            frame.push_str(&format!(
                "\r\n\r\x1b[K{}",
                ui.question
                    .paint(&truncate_to_width(&spec.header, room), depth)
            ));
        }
        // **The caret and the checkbox are two columns, not one.** They answer different
        // questions — where you are, and what is chosen — and folding them together left
        // `--multi` with no cursor at all. The caret is the look's marker; the box is the lead.
        let rows: Vec<Row> = shown
            .iter()
            .map(|&item| Row {
                text: spec.items[item].clone(),
                lead: match spec.multi {
                    true if checked[item] => "◉ ".to_string(),
                    true => "◯ ".to_string(),
                    false => String::new(),
                },
                marked: checked[item],
                ..Row::new(String::new())
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
                total: spec.items.len(),
                marked: checked.iter().filter(|c| **c).count(),
                cols: room,
                filtering: spec.filter,
                elapsed_ms: since.ms(),
            },
        ));
        let keys_shown: &[(&str, &str)] = if spec.multi {
            &[
                ("↑↓", "move"),
                ("space", "check"),
                ("enter", "confirm"),
                ("esc", "cancel"),
            ]
        } else {
            &[("↑↓", "move"), ("enter", "confirm"), ("esc", "cancel")]
        };
        panel.draw(&frame, keys_shown);

        // A look that animates comes back on its own to draw the next frame; one that does not
        // blocks, exactly as before. See `super::awaited`.
        let pressed = match super::awaited(&mut keys, spec.look.tick_ms()) {
            Pressed::Key(key) => key,
            Pressed::Timeout => continue,
            Pressed::Ended => {
                panel.close();
                return Answer::Cancelled;
            }
        };
        match pressed {
            // An abort is a cancel here: there is an answer to decline either way.
            Key::Cancel | Key::Abort => {
                panel.close();
                return Answer::Cancelled;
            }
            Key::Accept => {
                let picked: Vec<String> = if spec.multi {
                    let explicit: Vec<String> = (0..spec.items.len())
                        .filter(|&i| checked[i])
                        .map(|i| spec.items[i].clone())
                        .collect();
                    // Nothing checked means the row under the cursor, so Enter always answers.
                    if explicit.is_empty() {
                        shown
                            .get(selected)
                            .map(|&i| vec![spec.items[i].clone()])
                            .unwrap_or_default()
                    } else {
                        explicit
                    }
                } else {
                    shown
                        .get(selected)
                        .map(|&i| vec![spec.items[i].clone()])
                        .unwrap_or_default()
                };
                panel.close();
                if picked.is_empty() {
                    return Answer::Cancelled;
                }
                // Nothing is echoed. The widget erases itself completely and the answer — which
                // the caller prints to stdout — is the line left in the transcript. Echoing as
                // well showed it twice, once from each stream. This is gum's arrangement and the
                // reason it is right: there is exactly one record, and `$(…)` captures it.
                return Answer::Given(picked);
            }
            // The arrows follow the screen: a reversed list draws index 0 at the bottom, so Up has
            // to walk *towards* the far end of it. See `Look::step`.
            key if spec.look.step(key).is_some() => {
                let step = spec.look.step(key).unwrap_or(Step::Back);
                selected = step.from(selected, shown.len());
            }
            // Tab always toggles; space only when there is no query to type into. Swallowing
            // space unconditionally made multi-word filtering impossible in the one widget that
            // offers both.
            Key::ToggleScope if spec.multi => {
                if let Some(&item) = shown.get(selected) {
                    checked[item] = !checked[item];
                }
            }
            Key::Char(' ') if spec.multi && !spec.filter => {
                if let Some(&item) = shown.get(selected) {
                    checked[item] = !checked[item];
                }
            }
            Key::Char(c) if spec.filter => {
                query.push(c);
                shown = narrow(spec, &query);
                selected = 0;
                offset = 0;
            }
            Key::Backspace if spec.filter => {
                query.pop();
                shown = narrow(spec, &query);
                selected = 0;
                offset = 0;
            }
            Key::Clear if spec.filter => {
                query.clear();
                shown = narrow(spec, &query);
                selected = 0;
                offset = 0;
            }
            _ => {}
        }
    }
}

/// The indices matching `query`, best first.
fn narrow(spec: &Choice, query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..spec.items.len()).collect();
    }
    let pattern = Fuzzed::new(query, spec.fuzzy);
    let mut scored: Vec<(Quality, i32, usize)> = spec
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| pattern.rank(item).map(|(q, s)| (q, s, i)))
        .collect();
    // **Kind first, the same order the history finder uses.** A list filtered here and a list
    // filtered there should not disagree about which match is better, and `--look history` exists
    // to make this widget *be* the history browser.
    //
    // There is no recency to order within a kind — these items are whatever the caller piped in —
    // so the score breaks the tie, and then the original order, which keeps a list the caller
    // sorted sorted among equals.
    scored.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    scored.into_iter().map(|(_, _, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(items: &[&str]) -> Choice {
        Choice {
            items: items.iter().map(|s| s.to_string()).collect(),
            ..Choice::default()
        }
    }

    /// An empty list is not a question. Answering "cancelled" is what keeps
    /// `x=$(… | ui choose) || exit` correct when the pipeline produced nothing.
    #[test]
    fn an_empty_list_cancels() {
        assert_eq!(choose(&spec(&[])), Answer::Cancelled);
    }

    /// With no terminal there is nobody to ask, and unlike `input` there is no sensible default —
    /// picking the first item for a script would be worse than refusing.
    #[test]
    fn without_a_terminal_it_refuses() {
        assert_eq!(choose(&spec(&["a", "b"])), Answer::NoTerminal);
    }

    #[test]
    fn an_empty_filter_keeps_everything_in_order() {
        let s = spec(&["alpha", "beta", "gamma"]);
        assert_eq!(narrow(&s, ""), vec![0, 1, 2]);
    }

    #[test]
    fn a_filter_narrows_and_ranks() {
        let s = spec(&["cargo build", "cargo test", "npm install"]);
        let found = narrow(&s, "cargo");
        assert_eq!(found.len(), 2, "npm should not match");
        assert!(found.contains(&0) && found.contains(&1));
        assert!(narrow(&s, "zzzz").is_empty());
    }

    /// Among equal scores the caller's order survives, so a sorted list stays sorted.
    #[test]
    fn ties_keep_the_original_order() {
        let s = spec(&["match one", "match two", "match three"]);
        let found = narrow(&s, "match");
        assert_eq!(found, vec![0, 1, 2]);
    }
}
