//! Stream coordinates: `{line}`, `{line:word}`, `{stream:line:word}`.
//!
//! Addressing the text a command produced, by position, from the command that comes after it.
//!
//! ```text
//! web-01  10.0.0.1  nginx          {0}    the whole of line 0
//! web-02  10.0.0.2  apache         {0:1}  10.0.0.1
//! db-01   10.0.0.9  postgres       {-1:0} db-01
//! ```
//!
//! # How many you write says what you mean
//!
//! One to three dimensions, `:` between them, and nothing is marked:
//!
//! | written | means |
//! |---|---|
//! | `{2}` | line 2 |
//! | `{0:1}` | line 0, word 1 |
//! | `{1:0:1}` | one stream back, line 0, word 1 |
//!
//! Read right to left, the last is always the word, the one before it the line, and the one before
//! that the stream. There is no marker character because there is nowhere to put one: `|` and `;`
//! are lexer metacharacters, so `{3|0:4}` is split into words before any expansion runs and the
//! shell tries to *execute* `0:4}`. Braces do not protect a metacharacter.
//!
//! # Ranges are `..`, and they include both ends
//!
//! `{0..2:}` is three lines, not two. That is a deliberate departure from Python, whose slices are
//! half-open: the neighbouring syntax here is brace expansion, where `{0..2}` has meant `0 1 2` in
//! every shell for thirty years, and having the two disagree by one would be a trap set for the
//! person who just learned the other.
//!
//! **`{0..2}` itself is unavailable** — it *is* brace expansion, and claiming it would break
//! `echo {0..2}`. A whole-line range carries a trailing colon: `{0..2:}`.
//!
//! # An absent word is not the same as every word
//!
//! `{0}` is the whole of line 0, one value, spaces and all. `{0:*}` is every word of line 0, three
//! values. The distinction matters because one of them is a filename that might contain a space.

/// One dimension of a coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sel {
    /// A single position. Negative counts from the end, so `-1` is the last.
    At(isize),
    /// A run, both ends included. `None` means "from the start" or "to the end".
    Span {
        from: Option<isize>,
        to: Option<isize>,
    },
}

impl Sel {
    /// Everything: `*`, `..`, or an empty dimension.
    pub const ALL: Sel = Sel::Span {
        from: None,
        to: None,
    };

    /// The positions this selects out of `len` items, in order.
    ///
    /// Out of range is **empty, never an error**. Input is ragged — a three-line file asked for
    /// `{9}` should give nothing and let the command decide, rather than refusing to run.
    pub fn resolve(self, len: usize) -> Vec<usize> {
        if len == 0 {
            return Vec::new();
        }
        let last = len as isize - 1;
        let place = |i: isize| if i < 0 { i + len as isize } else { i };
        match self {
            Sel::At(i) => {
                let i = place(i);
                match (0..=last).contains(&i) {
                    true => vec![i as usize],
                    false => Vec::new(),
                }
            }
            Sel::Span { from, to } => {
                let from = place(from.unwrap_or(0)).max(0);
                let to = place(to.unwrap_or(last)).min(last);
                match from <= to {
                    true => (from as usize..=to as usize).collect(),
                    false => Vec::new(),
                }
            }
        }
    }
}

/// Which half of a stage a coordinate reads: what it printed, or what it *was*.
///
/// ```text
/// cat one.txt | echo "ran {%0:0} on {%0:1} and got {*}"
///                          │         │              └─ the output
///                          │         └─ its argument      one.txt
///                          └─ the command name            cat
/// ```
///
/// **Why `%` and not `!`.** `!` reads better — it is already the shell's reach-back character, and
/// `!!` has meant "the last command" for forty years. It is also unusable: history expansion runs
/// over the line before any of this, sees `!0` inside the braces and fails the line with
/// `!0: event not found`. `%` survives both the lexer and history, and its other meaning — `%1`
/// for a job — is only ever a whole word, never something inside a brace.
///
/// **A command has no line dimension**, because a command line is one line. So `%` shifts what the
/// dimensions mean by one: `{%1:0}` is *one stage back, word 0*, where `{1:0}` is line 1, word 0.
/// That is the same right-to-left rule the module docs give — the last dimension is always the
/// word — with the line simply absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Subject {
    /// What the stage printed.
    #[default]
    Output,
    /// The stage's own command line, its words being the command and its arguments.
    Command,
}

/// A parsed coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coord {
    /// Output or command. See [`Subject`].
    pub subject: Subject,
    /// How far back down the stack. `At(0)` — this command's own input — unless three dimensions
    /// were written.
    pub stream: Sel,
    pub line: Sel,
    /// `None` when no word dimension was written, which means the whole line rather than every
    /// word. See the module docs.
    pub word: Option<Sel>,
}

/// Read the inside of a `{…}`, without the braces.
///
/// Answers `None` for anything that is not a coordinate, so an ordinary brace group falls through
/// to the expansions that already handle it.
pub fn parse(inside: &str) -> Option<Coord> {
    if let Some(rest) = inside.strip_prefix('%') {
        return parse_command(rest);
    }
    let parts: Vec<&str> = inside.split(':').collect();
    let (stream, line, word) = match parts.as_slice() {
        [line] => (None, *line, None),
        [line, word] => (None, *line, Some(*word)),
        [stream, line, word] => (Some(*stream), *line, Some(*word)),
        _ => return None,
    };
    // An empty *word* is the whole line; an empty line or stream is all of them. Written out
    // rather than folded together because they are different answers to different questions.
    let word = match word {
        Some("") | None => None,
        Some(text) => Some(dimension(text)?),
    };
    Some(Coord {
        subject: Subject::Output,
        stream: match stream {
            Some("") | None => Sel::At(0),
            Some(text) => dimension(text)?,
        },
        line: dimension(line)?,
        word,
    })
}

/// The inside of a `{%…}` — one or two dimensions, because a command line has no line dimension.
///
/// `{%0}` is the whole command, `{%0:0}` its name, `{%0:1}` its first argument, `{%0:*}` every
/// word of it. The line is pinned to zero so the same [`select`] serves both subjects.
fn parse_command(rest: &str) -> Option<Coord> {
    let parts: Vec<&str> = rest.split(':').collect();
    let (stream, word) = match parts.as_slice() {
        [stream] => (*stream, None),
        [stream, word] => (*stream, Some(*word)),
        _ => return None,
    };
    Some(Coord {
        subject: Subject::Command,
        stream: match stream {
            "" => Sel::At(0),
            text => dimension(text)?,
        },
        line: Sel::At(0),
        word: match word {
            Some("") | None => None,
            Some(text) => Some(dimension(text)?),
        },
    })
}

/// One dimension: `2`, `-1`, `0..2`, `..2`, `2..`, `..`, `*`, or empty.
fn dimension(text: &str) -> Option<Sel> {
    if text.is_empty() || text == "*" || text == ".." {
        return Some(Sel::ALL);
    }
    match text.split_once("..") {
        Some((from, to)) => Some(Sel::Span {
            from: end(from)?,
            to: end(to)?,
        }),
        None => Some(Sel::At(number(text)?)),
    }
}

/// One end of a range, which may be absent.
fn end(text: &str) -> Option<Option<isize>> {
    match text.is_empty() {
        true => Some(None),
        false => Some(Some(number(text)?)),
    }
}

/// A signed index, rejecting anything a coordinate cannot hold — so `{a}` and `{1.5}` are left for
/// brace expansion rather than swallowed.
fn number(text: &str) -> Option<isize> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    match !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        true => text.parse().ok(),
        false => None,
    }
}

/// Apply a coordinate's line and word to one stream's text.
///
/// The stream dimension is the caller's business — it chooses which text to hand over.
pub fn select(coord: &Coord, text: &str) -> Vec<String> {
    let lines: Vec<&str> = match text.strip_suffix('\n').unwrap_or(text) {
        // A single trailing newline ends the last line; it does not begin an empty one. Without
        // this every `{-1}` on ordinary command output would answer with the empty string.
        "" => Vec::new(),
        body => body.split('\n').collect(),
    };
    let mut out = Vec::new();
    for at in coord.line.resolve(lines.len()) {
        let line = lines[at];
        let Some(word) = coord.word else {
            // No word dimension: the whole line, one value, spaces intact.
            out.push(line.to_string());
            continue;
        };
        let words: Vec<&str> = line.split_whitespace().collect();
        for at in word.resolve(words.len()) {
            out.push(words[at].to_string());
        }
    }
    out
}

/// Apply a command coordinate's word dimension to a command's words.
///
/// **The words arrive already separated**, rather than being split back out of a rendered line.
/// A command's arguments are known exactly, and an argument that contains a space is the whole
/// reason this feature is careful about words elsewhere — rendering `cat 'my file.txt'` to one
/// string and splitting it again would hand back `'my` and `file.txt'`.
pub fn select_words(coord: &Coord, words: &[String]) -> Vec<String> {
    let Some(word) = coord.word else {
        // No word dimension: the whole command line, one value — the same rule as a whole line.
        return match words.is_empty() {
            true => Vec::new(),
            false => vec![words.join(" ")],
        };
    };
    word.resolve(words.len())
        .into_iter()
        .map(|at| words[at].clone())
        .collect()
}

#[cfg(test)]
#[path = "coords/tests.rs"]
mod tests;
