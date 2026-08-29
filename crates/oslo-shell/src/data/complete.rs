//! What columns can be named at a point in a half-typed line.
//!
//! ```text
//! ls | sort-by <Tab>        name  size  size_human  is_dir  modified  mode
//! ps | cols pid <Tab>       name  cmdline  is_kernel
//! ```
//!
//! # The other half of the declaration, paying for itself twice
//!
//! [`super::columns`] exists so the planner can refuse a column no stage is carrying. The same
//! knowledge answers the question a person actually has at the prompt — *what are the columns
//! called?* — which until now had no answer at all. Naming a column is the most common thing anyone
//! does at a structured prompt and it was the one thing with no help.
//!
//! # Why a hook rather than a call
//!
//! `oslo-ui` draws the menu and `oslo-shell` owns the registry, and the dependency runs
//! shell → ui. So the shell installs a closure, exactly as it does for
//! [`oslo_ui::completion::set_command_completer`] and for the same layering reason.
//!
//! # Three answers, not two
//!
//! * `None` — not a column position. The menu falls through to filenames, which is right for
//!   `ls <Tab>`.
//! * `Some([…])` — these columns.
//! * `Some([])` — a column position whose columns are not knowable (`from json | cols <Tab>`).
//!   **The menu must not fall through here**: offering filenames where a column belongs is the
//!   wrong nothing, and it is the same rule `spec_candidates` already follows.

use super::columns::{Columns, through};

/// The columns nameable at `pos` in `line`, or `None` if that is not a column position.
pub fn columns_at(line: &str, pos: usize) -> Option<Vec<String>> {
    let typed = line.get(..pos)?;
    let stages: Vec<&str> = split_stages(typed);
    let (current, earlier) = stages.split_last()?;

    let words = words_of(current);
    // The word being typed is the last one, and it is not an operand that has been *given* yet —
    // a trailing space means a new empty word is being started.
    let name = words.first()?.clone();
    // Not a verb, so not a column position — a path or a flag, and somebody else's answer.
    super::tool::lookup(&name)?;
    let at = match current.ends_with(char::is_whitespace) {
        true => words.len(),
        false => words.len().saturating_sub(1),
    };
    if !names_a_column(&name, &words, at) {
        return None;
    }

    // Everything to the left decides what there is to offer.
    let mut columns = Columns::Unknown;
    for stage in earlier {
        let words = words_of(stage);
        let Some(stage_name) = words.first() else {
            columns = Columns::Unknown;
            continue;
        };
        if super::tool::lookup(stage_name).is_none() {
            columns = Columns::Unknown;
            continue;
        }
        columns = through(stage_name, &words, &columns);
    }
    // Not knowable is still a column position: answer nothing rather than falling through.
    Some(columns.names().map(<[String]>::to_vec).unwrap_or_default())
}

/// Whether the operand at `at` of `name` is a column name.
///
/// Positions rather than values, which is what a half-typed line needs — the word may not be there
/// yet. `super::super::exec::pipeline` asks the value-shaped question for its refusal; this is the
/// same table read the other way.
fn names_a_column(name: &str, words: &[String], at: usize) -> bool {
    if at == 0 {
        return false;
    }
    match name {
        // Every operand.
        "cols" | "reject" => true,
        // Every operand that is not a flag.
        "sort-by" => !words.get(at).is_some_and(|w| w.starts_with('-')),
        // The first, and only it.
        "get" | "group-by" | "stats" | "histogram" | "update" | "distinct" | "compact"
        | "rename" | "insert" | "upsert" | "default" => at == 1,
        // After the Lua expression, and after `--keep` when it is there.
        "lookup" => {
            at == if words.get(1).is_some_and(|w| w == "--keep") {
                3
            } else {
                2
            }
        }
        _ => false,
    }
}

/// Split a line into pipeline stages on unquoted `|`.
///
/// `||` is not a pipe, and neither is one inside quotes — both would cut a stage in the wrong place
/// and offer the columns of something that is not upstream.
fn split_stages(line: &str) -> Vec<&str> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut quote: Option<u8> = None;
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        match quote {
            Some(open) => {
                if byte == b'\\' && open == b'"' {
                    at += 2;
                    continue;
                }
                if byte == open {
                    quote = None;
                }
            }
            None => match byte {
                b'\'' | b'"' => quote = Some(byte),
                b'|' if bytes.get(at + 1) == Some(&b'|') => {
                    // `a || b` is a list, not a pipeline: nothing upstream reaches the right side.
                    out.clear();
                    at += 2;
                    start = at;
                    continue;
                }
                b'|' => {
                    out.push(&line[start..at]);
                    start = at + 1;
                }
                // A new command entirely; whatever came before is not upstream of it.
                b';' | b'&' => {
                    out.clear();
                    start = at + 1;
                }
                _ => {}
            },
        }
        at += 1;
    }
    out.push(&line[start..]);
    out
}

/// The words of one stage, unquoted.
///
/// Only what a column operand can be: quotes stripped, whitespace between words. A word carrying an
/// expansion is kept as it was typed, and simply will not match any column — which is the right
/// answer, since what it expands to is not known here.
fn words_of(stage: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = stage.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(open) => {
                if c == '\\' && open == '"' {
                    if let Some(escaped) = chars.next() {
                        word.push(escaped);
                    }
                    continue;
                }
                match c == open {
                    true => quote = None,
                    false => word.push(c),
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    started = true;
                }
                c if c.is_whitespace() => {
                    if started {
                        out.push(std::mem::take(&mut word));
                        started = false;
                    }
                }
                c => {
                    word.push(c);
                    started = true;
                }
            },
        }
    }
    if started {
        out.push(word);
    }
    out
}

#[cfg(test)]
#[path = "complete/tests.rs"]
mod tests;
