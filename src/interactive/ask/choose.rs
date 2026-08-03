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

use super::{Answer, legend, show};
use crate::interactive::dropdown::width::{terminal_cols, terminal_rows, truncate_to_width};
use crate::interactive::matching::{Fuzzed, Fuzzy};
use crate::interactive::term::{Key, Keys, Restore};
use crate::interactive::theme;

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

    let Some(raw) = Restore::enter(false) else {
        return Answer::NoTerminal;
    };

    let mut query = String::new();
    let mut shown: Vec<usize> = (0..spec.items.len()).collect();
    let mut checked = vec![false; spec.items.len()];
    let mut selected = 0usize;
    let mut offset = 0usize;
    let mut keys = Keys::on(raw.fd());
    // Rows this widget printed last time, so the next frame erases exactly them. Erasing more
    // would eat the caller's output; erasing fewer leaves half a list on the screen.
    let mut drawn = 0usize;

    loop {
        let height = spec
            .height
            .min(shown.len().max(1))
            .min(terminal_rows().saturating_sub(3).max(1));
        if selected >= shown.len() {
            selected = shown.len().saturating_sub(1);
        }
        if selected < offset {
            offset = selected;
        } else if selected >= offset + height {
            offset = selected + 1 - height;
        }

        let mut frame = String::new();
        // Back to the top of what was drawn, then repaint downward.
        if drawn > 0 {
            frame.push_str(&format!("\x1b[{drawn}A"));
        }
        let cols = terminal_cols();

        if !spec.header.is_empty() {
            frame.push_str(&format!(
                "\r\x1b[K{}\r\n",
                ui.question.paint(&spec.header, depth)
            ));
        }
        if spec.filter {
            frame.push_str(&format!(
                "\r\x1b[K{} {}\r\n",
                ui.accent.paint("❯", depth),
                if query.is_empty() {
                    ui.muted.paint("type to filter", depth)
                } else {
                    query.clone()
                }
            ));
        }
        for row in 0..height {
            let text = match shown.get(offset + row) {
                Some(&item) => {
                    let here = offset + row == selected;
                    let mark = if spec.multi {
                        if checked[item] { "◉ " } else { "◯ " }
                    } else if here {
                        "❯ "
                    } else {
                        "  "
                    };
                    let label = truncate_to_width(&spec.items[item], cols.saturating_sub(4));
                    let style = if here {
                        ui.accent
                    } else {
                        theme::Style::default()
                    };
                    format!(
                        "{}{}",
                        if here { ui.accent } else { ui.muted }.paint(mark, depth),
                        style.paint(&label, depth)
                    )
                }
                None => String::new(),
            };
            frame.push_str(&format!("\r\x1b[K{text}\r\n"));
        }
        let keys_shown: &[(&str, &str)] = if spec.multi {
            &[("↑↓", "move"), ("space", "check"), ("enter", "done")]
        } else {
            &[("↑↓", "move"), ("enter", "choose")]
        };
        frame.push_str(&format!("\r\x1b[K{}", legend(keys_shown)));
        show(&frame);
        drawn = height + 1 + usize::from(!spec.header.is_empty()) + usize::from(spec.filter);

        let Some(pressed) = keys.read() else {
            erase(drawn);
            return Answer::Cancelled;
        };
        match pressed {
            Key::Cancel => {
                erase(drawn);
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
                erase(drawn);
                if picked.is_empty() {
                    return Answer::Cancelled;
                }
                if !spec.header.is_empty() {
                    show(&format!(
                        "{} {}\r\n",
                        ui.question.paint(&spec.header, depth),
                        ui.done.paint(&picked.join(", "), depth)
                    ));
                }
                return Answer::Given(picked);
            }
            Key::Up => selected = selected.saturating_sub(1),
            Key::Down => selected = (selected + 1).min(shown.len().saturating_sub(1)),
            Key::PageUp | Key::Home => selected = 0,
            Key::PageDown | Key::End => selected = shown.len().saturating_sub(1),
            Key::ToggleScope | Key::Char(' ') if spec.multi => {
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
    let mut scored: Vec<(i32, usize)> = spec
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| pattern.score(item).map(|s| (s, i)))
        .collect();
    // Best first, then the original order — so a list the caller sorted stays sorted among equals.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}

/// Erase the `rows` this widget printed, leaving the cursor where it started.
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
