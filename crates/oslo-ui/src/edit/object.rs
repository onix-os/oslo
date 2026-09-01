//! Vi text objects: what `iw`, `a"` and `i(` mean on a single line.
//!
//! ```text
//! echo he|llo there    ciw  →  echo | there          the word the cursor is in
//! echo he|llo there    caw  →  echo |there           the word and the space after it
//! echo "he|llo"        ci"  →  echo "|"              inside the quotes
//! echo "he|llo"        ca"   →  echo |               the quotes as well
//! f(a, b|, c)          ci(  →  f(|)                  inside the innermost pair
//! ```
//!
//! # Why this is a file and not four match arms
//!
//! A text object is not a motion. A motion answers *where the cursor goes* and the operator turns
//! that into a range, which is why `cw` has to be special-cased into `ce` — the range and the
//! movement disagree. An object answers the range **directly**, and never moves anything on its
//! own. Keeping the two apart is what stops the second one inheriting the first one's exceptions.
//!
//! # One line, and lexical
//!
//! Everything here reads characters. It does not know what the shell will make of the line, so
//! `ci"` inside a here-document body or a `$(…)` finds the quotes it can see rather than the ones a
//! parse would agree with. That is the same trade [`super::pair`] makes and for the same reason:
//! the answer has to be available halfway through typing, when there is no parse to be had.
//!
//! Vim's `iw`/`aw` classes, restated: whitespace, word characters, and punctuation are three kinds,
//! and an object is the run of one kind. `W` collapses the last two, which is exactly what makes
//! `ciW` take a whole path where `ciw` takes one segment of it.

use super::buffer::Buffer;

/// A half-open range of character indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Span {
    pub from: usize,
    pub to: usize,
}

/// The range `object` names at `at`, or `None` if there is no such thing here.
///
/// `around` is the difference between `i` and `a`: whether the delimiters, or the whitespace after
/// a word, are part of it.
pub(super) fn find(buf: &Buffer, at: usize, around: bool, object: char) -> Option<Span> {
    match object {
        'w' | 'W' => word(buf, at, around, object == 'W'),
        '"' | '\'' | '`' => quoted(buf, at, around, object),
        // `b` and `B` are vim's own aliases for the two people reach for most.
        '(' | ')' | 'b' => bracket(buf, at, around, '(', ')'),
        '[' | ']' => bracket(buf, at, around, '[', ']'),
        '{' | '}' | 'B' => bracket(buf, at, around, '{', '}'),
        '<' | '>' => bracket(buf, at, around, '<', '>'),
        _ => None,
    }
}

/// Which of the three kinds a character is: whitespace, word, or punctuation.
///
/// The three-way split *is* `iw`: `foo.bar` is three objects to `w` and one to `W`, which is the
/// whole difference between them and the reason a path wants the capital.
fn kind(c: char, big: bool) -> u8 {
    match c {
        c if c.is_whitespace() => 0,
        c if c.is_alphanumeric() || c == '_' => 1,
        // Under `W` there are only two kinds, so punctuation joins the word.
        _ => match big {
            true => 1,
            false => 2,
        },
    }
}

fn kind_at(buf: &Buffer, at: usize, big: bool) -> Option<u8> {
    buf.char_at(at).map(|c| kind(c, big))
}

/// `iw` is the run of one kind; `aw` adds the whitespace that goes with it.
fn word(buf: &Buffer, at: usize, around: bool, big: bool) -> Option<Span> {
    let len = buf.len();
    // The cursor may sit one past the end, where vi's own never does.
    let at = at.min(len.checked_sub(1)?);
    let here = kind_at(buf, at, big)?;

    let mut from = at;
    while from > 0 && kind_at(buf, from - 1, big) == Some(here) {
        from -= 1;
    }
    let mut to = at + 1;
    while kind_at(buf, to, big) == Some(here) {
        to += 1;
    }

    if around {
        match here {
            // **On whitespace, `aw` takes the word after it** — vim's rule, and the one that makes
            // `daw` between two words leave one space rather than none.
            0 => {
                let next = kind_at(buf, to, big);
                while next.is_some() && kind_at(buf, to, big) == next {
                    to += 1;
                }
            }
            // The trailing whitespace, or — when there is none, at the end of a line — the leading.
            _ => {
                let was = to;
                while kind_at(buf, to, big) == Some(0) {
                    to += 1;
                }
                if to == was {
                    while from > 0 && kind_at(buf, from - 1, big) == Some(0) {
                        from -= 1;
                    }
                }
            }
        }
    }
    Some(Span { from, to })
}

/// `i"` is what is between a pair of quotes; `a"` is the pair as well.
///
/// **Paired from the start of the line**, not searched outward from the cursor. Quotes have no
/// direction — the same character opens and closes — so the only way to know whether the one to
/// your left opens or closes is to count the ones before it.
fn quoted(buf: &Buffer, at: usize, around: bool, quote: char) -> Option<Span> {
    let len = buf.len();
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut open: Option<usize> = None;
    let mut i = 0;
    while i < len {
        match buf.char_at(i) {
            // An escaped quote is a character in a string, not an end to it.
            Some('\\') => i += 1,
            Some(c) if c == quote => match open.take() {
                Some(start) => pairs.push((start, i)),
                None => open = Some(i),
            },
            _ => {}
        }
        i += 1;
    }
    // The pair the cursor is in, or — as in vim — the next one along the line.
    let (start, end) = pairs
        .iter()
        .find(|(open, close)| *open <= at && at <= *close)
        .or_else(|| pairs.iter().find(|(open, _)| *open >= at))
        .copied()?;
    Some(match around {
        true => Span {
            from: start,
            to: end + 1,
        },
        false => Span {
            from: start + 1,
            to: end,
        },
    })
}

/// `i(` is what is inside the innermost pair the cursor is in; `a(` is the brackets too.
fn bracket(buf: &Buffer, at: usize, around: bool, open: char, close: char) -> Option<Span> {
    let len = buf.len();
    let start = match buf.char_at(at) {
        // Sitting on the opener is being inside it, which is what makes `ci(` work with the cursor
        // where `f(` just left it.
        Some(c) if c == open => at,
        _ => opener_before(buf, at, open, close)?,
    };

    let mut depth = 0usize;
    let mut end = None;
    for j in start + 1..len {
        match buf.char_at(j) {
            Some(c) if c == open => depth += 1,
            Some(c) if c == close => match depth {
                0 => {
                    end = Some(j);
                    break;
                }
                _ => depth -= 1,
            },
            _ => {}
        }
    }
    let end = end?;
    Some(match around {
        true => Span {
            from: start,
            to: end + 1,
        },
        false => Span {
            from: start + 1,
            to: end,
        },
    })
}

/// The unmatched opener to the left of `at`, counting past any pair that closes on the way.
fn opener_before(buf: &Buffer, at: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = at;
    while i > 0 {
        i -= 1;
        match buf.char_at(i) {
            Some(c) if c == close => depth += 1,
            Some(c) if c == open => match depth {
                0 => return Some(i),
                _ => depth -= 1,
            },
            _ => {}
        }
    }
    None
}

#[cfg(test)]
#[path = "object/tests.rs"]
mod tests;
