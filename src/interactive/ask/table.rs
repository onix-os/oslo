//! `ui table` — pick a row out of columns.
//!
//! [`super::choose`] where each item is several fields. The difference that matters is alignment:
//! a list of `"alice  30  admin"` strings drawn with a fixed font is still ragged unless something
//! measures the columns, and once you are measuring you may as well have a header.
//!
//! Input is separated values — commas by default, or whatever `--separator` says — because that is
//! what the things a script already has produce: `cut`, `awk`, `ps`, a `.csv`.
//!
//! The answer is the **whole row**, in its original text. A widget that answered with a field
//! would have to be told which one, and a caller that wants a field can `cut` the row it got back.

use super::look::{Row, Step, View};
use super::{Answer, Inline};
use crate::interactive::dropdown::width::{pad_to_width, terminal_rows, truncate_to_width};
use crate::interactive::matching::{Fuzzed, Fuzzy};
use crate::interactive::term::{Key, Keys, Pressed, Restore, Screen};
use crate::interactive::theme;

/// How the table is asked.
#[derive(Debug, Clone)]
pub struct Table {
    /// Column names. Empty means the first row is the header.
    pub headers: Vec<String>,
    /// One vector of fields per row.
    pub rows: Vec<Vec<String>>,
    /// The rows as they arrived, answered verbatim so nothing is reformatted on the way out.
    pub raw: Vec<String>,
    pub height: usize,
    /// Narrow as you type, over every field of the row.
    pub filter: bool,
    pub fuzzy: Fuzzy,
    /// The legend, the border, the screen and where on it. See `super::chrome`.
    pub chrome: super::chrome::Chrome,
    /// Where the filter sits and what colour the rows take. See `super::look`.
    pub look: super::look::Look,
}

impl Default for Table {
    fn default() -> Self {
        Table {
            headers: Vec::new(),
            rows: Vec::new(),
            raw: Vec::new(),
            height: 10,
            filter: true,
            fuzzy: Fuzzy::Smart,
            chrome: super::chrome::Chrome::default(),
            look: super::look::Look::default(),
        }
    }
}

/// Split `text` into rows of fields on `separator`.
pub fn parse(text: &str, separator: char) -> (Vec<Vec<String>>, Vec<String>) {
    let mut rows = Vec::new();
    let mut raw = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        rows.push(
            line.split(separator)
                .map(|field| field.trim().to_string())
                .collect(),
        );
        raw.push(line.to_string());
    }
    (rows, raw)
}

pub fn table(spec: &Table) -> Answer<String> {
    if spec.rows.is_empty() {
        return Answer::Cancelled;
    }
    let ui = theme::current().ui;
    let depth = theme::depth();

    let Some(raw_mode) = Restore::enter(Screen::Inline) else {
        return Answer::NoTerminal;
    };

    // One width per column, from the header and every row, so the table is aligned before a key is
    // pressed rather than shifting as it filters.
    let columns = spec
        .rows
        .iter()
        .map(Vec::len)
        .chain(std::iter::once(spec.headers.len()))
        .max()
        .unwrap_or(0);
    let mut widths = vec![0usize; columns];
    for (index, width) in widths.iter_mut().enumerate() {
        *width = spec
            .rows
            .iter()
            .filter_map(|row| row.get(index))
            .chain(spec.headers.get(index))
            .map(|field| crate::interactive::prompt::printed_width(field))
            .max()
            .unwrap_or(0)
            .min(30);
    }

    let mut query = String::new();
    let mut shown: Vec<usize> = (0..spec.rows.len()).collect();
    let mut selected = 0usize;
    let mut offset = 0usize;
    let mut keys = Keys::on(raw_mode.fd());
    let mut panel = Inline::with_chrome(spec.chrome.clone());
    let since = super::Since::now();

    loop {
        // Computed from the same booleans the frame draws with, so the clamp and the frame cannot
        // disagree — a hard-coded constant here is how this reserved a row it never used.
        let chrome = spec.chrome.extra_rows()
            + usize::from(!spec.headers.is_empty())
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

        let cols = spec.chrome.room();
        // `from` is how many leading fields the look has taken as metadata columns; their widths
        // are skipped here so the rest still line up against the widths measured for them.
        let render = |fields: &[String], from: usize| -> String {
            let mut line = String::new();
            for (index, width) in widths.iter().skip(from).enumerate() {
                let field = fields.get(index).map(String::as_str).unwrap_or("");
                line.push_str(&pad_to_width(&truncate_to_width(field, *width), *width));
                line.push_str("  ");
            }
            truncate_to_width(line.trim_end(), cols)
        };

        let meta = spec.look.meta_columns.min(widths.len());
        let mut frame = String::new();
        if !spec.headers.is_empty() {
            let headers = &spec.headers[meta.min(spec.headers.len())..];
            frame.push_str(&format!(
                "\r\n\r\x1b[K  {}",
                ui.question.paint(&render(headers, meta), depth)
            ));
        }
        // The look decides what colour the rows take and where the filter sits, exactly as it does
        // for `choose`. One renderer, so the two lists cannot drift apart.
        //
        // `meta_columns` splits each row: the first N fields become right-aligned metadata columns
        // and the rest is the text. That is the whole difference between a table and the history
        // browser — `1d  118×  cargo test` is three fields with the first two as columns.
        let rows: Vec<Row> = shown
            .iter()
            .map(|&index| {
                let fields = &spec.rows[index];
                Row {
                    meta: fields.iter().take(meta).cloned().collect(),
                    ..Row::new(render(&fields[meta.min(fields.len())..], meta))
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
                total: spec.rows.len(),
                marked: 0,
                cols,
                filtering: spec.filter,
                elapsed_ms: since.ms(),
            },
        ));
        panel.draw(&frame, &[("↑↓", "move"), ("enter", "choose")]);

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
                let picked = shown.get(selected).and_then(|&i| spec.raw.get(i)).cloned();
                panel.close();
                return match picked {
                    Some(row) => Answer::Given(row),
                    None => Answer::Cancelled,
                };
            }
            // The arrows follow the screen, not the list — see `Look::step`.
            key if spec.look.step(key).is_some() => {
                let step = spec.look.step(key).unwrap_or(Step::Back);
                selected = step.from(selected, shown.len());
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
            _ => {}
        }
    }
}

/// Rows matching `query` across all their fields, best first.
fn narrow(spec: &Table, query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..spec.rows.len()).collect();
    }
    let pattern = Fuzzed::new(query, spec.fuzzy);
    let mut scored: Vec<(i32, usize)> = spec
        .rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| {
            // **Each field on its own, best wins.** Not the joined row: oslo's matcher anchors a
            // match at the start of what it is given, so `admin` against `"alice 30 admin"` scores
            // nothing at all. Scoring per field is what makes typing a value from the third column
            // find its row, which is the whole of what a table search is for.
            row.iter()
                .filter_map(|field| pattern.score(field))
                .max()
                .map(|score| (score, index))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, index)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(text: &str) -> Table {
        let (rows, raw) = parse(text, ',');
        Table {
            rows,
            raw,
            ..Table::default()
        }
    }

    #[test]
    fn fields_are_split_and_trimmed() {
        let (rows, raw) = parse("a, b ,c\nd,e,f", ',');
        assert_eq!(rows[0], vec!["a", "b", "c"]);
        assert_eq!(rows[1], vec!["d", "e", "f"]);
        // The raw line is kept exactly, because that is what is answered with.
        assert_eq!(raw[0], "a, b ,c");
    }

    #[test]
    fn blank_lines_are_not_rows() {
        let (rows, _) = parse("a,b\n\n\nc,d\n", ',');
        assert_eq!(rows.len(), 2);
    }

    /// A search matches any field, which is what a person expects of a table.
    #[test]
    fn filtering_looks_at_every_column() {
        let t = built("alice,30,admin\nbob,25,user");
        assert_eq!(narrow(&t, "admin"), vec![0]);
        assert_eq!(narrow(&t, "bob"), vec![1]);
        assert_eq!(narrow(&t, "25"), vec![1]);
        assert!(narrow(&t, "nobody").is_empty());
    }

    #[test]
    fn an_empty_table_is_a_cancel() {
        assert_eq!(table(&Table::default()), Answer::Cancelled);
    }

    #[test]
    fn without_a_terminal_it_refuses() {
        assert_eq!(table(&built("a,b")), Answer::NoTerminal);
    }

    /// A ragged row is not an error: the short one simply has empty cells.
    #[test]
    fn rows_may_have_different_lengths() {
        let (rows, _) = parse("a,b,c\nd", ',');
        assert_eq!(rows[1], vec!["d"]);
    }
}
