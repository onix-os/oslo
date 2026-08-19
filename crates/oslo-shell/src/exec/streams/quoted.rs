//! A coordinate inside double quotes.
//!
//! `echo "ran {%0:0} on {%0:1} and got {0}"` is the shape the feature is for — a coordinate in the
//! middle of a message rather than standing alone as an argument. That could not be written at all
//! while a coordinate was literal in both quote styles, and the fix is the rule the shell already
//! has everywhere else: **single quotes are text, double quotes expand.**
//!
//! ```text
//! echo "got {0:0}"     →  got web-01
//! echo 'got {0:0}'     →  got {0:0}
//! ```
//!
//! # Inside quotes the values join, and the word stays one word
//!
//! Unquoted, a lone `{*:0}` is one argument per line — that is what makes `ping {*:0}` one process
//! with three arguments. Inside quotes it cannot be: a quoted word is one word by definition, and
//! there is text either side of it to keep attached. So the values join with a space, exactly as
//! `"${a[*]}"` does, and the distinction between the two spellings is preserved rather than lost.
//!
//! # Nothing here can split or glob
//!
//! The substituted text lands in a `Literal` part *inside* a `DoubleQuoted`, and the expansion
//! pipeline does not field-split or glob what is inside one. So a value holding a space or a `*`
//! arrives whole without needing the `SingleQuoted` wrapper the unquoted path uses — the quotes
//! the user typed are already doing that job.

use super::{Streams, split, values};
use oslo_base::ast::{Word, WordPart};

/// Replace every coordinate inside this word's double-quoted parts.
///
/// Answers whether anything changed, so the gate and the rewriter agree about what counts.
pub(super) fn rewrite_inside_quotes(word: &mut Word, streams: &Streams) -> bool {
    let mut changed = false;
    for part in &mut word.parts {
        let WordPart::DoubleQuoted(inner) = part else {
            continue;
        };
        for part in inner.iter_mut() {
            let WordPart::Literal(text) = part else {
                continue;
            };
            if let Some(replaced) = substitute_text(text, streams) {
                *part = WordPart::Literal(replaced);
                changed = true;
            }
        }
    }
    changed
}

/// Whether this word has a coordinate inside a double-quoted part.
///
/// The gate's half of the same question. It has to mirror [`rewrite_inside_quotes`] exactly: a gate
/// that answered `false` here would leave `echo "{0:0}"` on the concurrent path, where there is no
/// captured stage for it to read and the coordinate would print as text.
pub(super) fn holds_a_quoted_coordinate(word: &Word) -> bool {
    word.parts.iter().any(|part| match part {
        WordPart::DoubleQuoted(inner) => inner.iter().any(|part| match part {
            // Parsed rather than scanned. The cheap scan is right for a bare word, where a false
            // positive costs only the sequential path — but `"(y{1,2})"` passes it and is brace
            // expansion, and there is no reason to make an ordinary quoted string pay.
            WordPart::Literal(text) => super::holds_a_coordinate(text),
            _ => false,
        }),
        _ => false,
    })
}

/// Every coordinate in one piece of text, replaced by its values joined with a space.
///
/// `None` when there is no coordinate in it, so an ordinary quoted string is left untouched rather
/// than rebuilt into an identical copy of itself.
fn substitute_text(text: &str, streams: &Streams) -> Option<String> {
    let mut out = String::new();
    let mut rest = text;
    let mut found = false;
    while let Some((before, coord, after)) = split(rest) {
        found = true;
        out.push_str(before);
        out.push_str(&values(&coord, streams).join(" "));
        rest = after;
    }
    if !found {
        return None;
    }
    out.push_str(rest);
    Some(out)
}
