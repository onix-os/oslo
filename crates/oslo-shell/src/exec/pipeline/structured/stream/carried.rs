//! The verbs that carry state from one batch to the next.
//!
//! Everything else in a streamed pipeline is row-local: its answer for a batch is its answer for
//! each row, so batching is invisible. These two are not, and each is wrong in its own way when
//! applied per batch — `first 2` would take two rows out of *every* one, and `final 2` would answer
//! about the last batch rather than about the stream.

use super::*;

/// A folding verb: swallow the batch, and answer only once the last one has gone in.
///
/// **What is held is bounded, which is the whole licence to do this.** `length` keeps a count and
/// `final n` keeps n rows, so the memory a fold costs does not grow with the upstream. The answer
/// at the end is the real verb's, computed from what was kept, so a streamed `final 3` and a
/// materialised one cannot drift apart.
pub(super) fn folded(
    name: &str,
    words: &[String],
    rows: Vec<Record>,
    state: &mut Counted,
    at_end: bool,
) -> Vec<Record> {
    let n: usize = words.get(1).and_then(|w| w.parse().ok()).unwrap_or(1);
    match name {
        "final" => {
            state.kept.extend(rows);
            // Trimmed every batch rather than at the end, so the window is the bound rather than a
            // thing that merely gets truncated once the whole stream has been held.
            let over = state.kept.len().saturating_sub(n);
            state.kept.drain(..over);
            match at_end {
                true => std::mem::take(&mut state.kept),
                false => Vec::new(),
            }
        }
        // `length`, whose answer needs the count and nothing else.
        _ => {
            state.seen += rows.len();
            match at_end {
                true => vec![Record::from_pairs([(
                    "length",
                    Val::Int(state.seen as i64),
                )])],
                false => Vec::new(),
            }
        }
    }
}

/// A counting verb, with its count carried from the batch before.
///
/// Answers the rows that survive and whether nothing more will ever come out.
pub(super) fn counted(
    name: &str,
    words: &[String],
    rows: Vec<Record>,
    state: &mut Counted,
) -> (Vec<Record>, bool) {
    let n: usize = words.get(1).and_then(|w| w.parse().ok()).unwrap_or(1);
    match name {
        "first" => {
            let room = n.saturating_sub(state.seen);
            let kept: Vec<Record> = rows.into_iter().take(room).collect();
            state.seen += kept.len();
            state.finished = state.seen >= n;
            (kept, state.finished)
        }
        "skip" => {
            let dropping = n.saturating_sub(state.seen).min(rows.len());
            state.seen += dropping;
            (rows.into_iter().skip(dropping).collect(), false)
        }
        "every" => {
            let mut kept = Vec::new();
            for row in rows {
                if n > 0 && state.seen.is_multiple_of(n) {
                    kept.push(row);
                }
                state.seen += 1;
            }
            (kept, false)
        }
        // `enumerate`, whose index has to keep counting across batches or every batch would start
        // again at zero — the wrong answer, and a quiet one.
        _ => {
            let mut kept = Vec::new();
            for row in rows {
                let mut out = Record::from_pairs([("index", Val::Int(state.seen as i64))]);
                for (column, value) in row.columns().iter().zip(row.values()) {
                    out.set(column, value.clone());
                }
                state.seen += 1;
                kept.push(out);
            }
            (kept, false)
        }
    }
}
