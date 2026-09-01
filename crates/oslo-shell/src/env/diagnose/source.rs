//! Pointing into something that really is a source: a script, or a Lua chunk.
//!
//! [`super`] rebuilds a command's own words into a line to point into, which is what makes a caret
//! affordable across eighty sites — no parser learns to keep spans and no signature changes. This
//! is the other half, and the better one wherever it applies: a real path, the file's own line
//! number, and the code as written.
//!
//! # Nothing here is plumbed; it is all read back
//!
//! Three different messages already carry a position, and none of them had to grow a field:
//!
//! * `env::scope::record::origin` answers `file: line N: ` for every diagnostic in a script — it is
//!   what prints the prefix, so the file and the line were decided long before any report.
//! * the parser writes `at 1,6 (detected near line 2 col 1)` into a syntax error.
//! * Lua writes `chunk:line: message` into every error it raises.
//!
//! Reading a number back out of a string is not elegant. It is, in each of these three cases, a
//! great deal less work than threading a span through a type twelve sites construct — and it is
//! only ever used to *draw*, so a message this cannot parse falls back to the one-liner it always
//! printed.

use super::origin_now;
use oslo_base::diag;

/// A syntax error, with a caret into the text the parser was reading.
///
/// The only place a report points into something that really is a *program* rather than into a
/// command's own words — so this is where a diagnostic gets to look like a compiler's, with the
/// failing line quoted and the column marked.
///
/// **The position is read back out of the message rather than carried.** By the time a parse error
/// reaches here it is a formatted string: the position the parser knew was spent writing
/// `at 1,6 (detected near line 2 col 1)` and nothing structured survives. Threading a span out
/// through `ShellError::SyntaxError` would mean a field on a variant twelve sites construct and
/// every match on it — a large change to recover a number the message is already carrying.
///
/// Where the message has no position, nothing is drawn: `syntax error at end of input` is about the
/// absence of text, and there is no column in a file for that.
pub fn complain_at(origin: &str, name: &str, text: &str, line_offset: u32, body: &str) -> bool {
    if !diag::enabled() {
        return false;
    }
    // The caller's origin, not the thread-local one: nothing publishes to that for a parse
    // failure, so reading it would print `oslo: ` where the plain path prints the file and line.
    let message = format!("{origin}{body}");
    let Some(at) = offset_of(text, line_offset, body) else {
        return false;
    };
    diag::draw_source(
        text,
        at..at + 1,
        &diag::Report {
            message: &message,
            source: name,
            label: "here",
            help: None,
        },
    )
}

/// The byte offset a `line,column` in `message` names, as an index into `text`.
///
/// The first `N,M` pair in the message, which is how both the tokenizer and the parser spell a
/// position — `at 1,6` and `near 1,6`. One-based on both counts, as those messages are.
fn offset_of(text: &str, line_offset: u32, message: &str) -> Option<usize> {
    let (line, column) = line_and_column(message)?;
    // The parser counted from the start of the *chunk* it was given; `line_offset` is how much of
    // the file came before it. See `Environment::set_line_offset`.
    let line = line + line_offset as usize;
    let start = text
        .lines()
        .take(line.checked_sub(1)?)
        .map(|l| l.len() + 1)
        .sum::<usize>();
    let at = start + column.checked_sub(1)?;
    (at < text.len()).then(|| diag::floor_boundary(text, at))
}

/// The first `N,M` in a message, as numbers.
/// Exposed because the *prefix* needs it too: `origin` names the last line that ran, and a parse
/// failure did not run anything — so without publishing the parser's own line, a syntax error on
/// line 5 is announced as line 4.
pub fn parsed_position(message: &str) -> Option<(usize, usize)> {
    line_and_column(message)
}

fn line_and_column(message: &str) -> Option<(usize, usize)> {
    let bytes = message.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if !bytes[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        let start = at;
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
        }
        if bytes.get(at) != Some(&b',') {
            continue;
        }
        let comma = at;
        at += 1;
        let second = at;
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
        }
        if at == second {
            continue;
        }
        let line = message[start..comma].parse().ok()?;
        let column = message[second..at].parse().ok()?;
        return Some((line, column));
    }
    None
}
/// The most a diagnostic will read of a script to point into it.
///
/// A shell script is a few kilobytes; a megabyte of one is a generated file nobody is reading a
/// caret in. The cap is here rather than nowhere because this runs on the failure path and a
/// diagnostic must not be the slowest thing in the shell.
const MOST_OF_A_SCRIPT: u64 = 1 << 20;

/// Draw into the script the diagnostic came from, if there is one and `word` is on the line.
///
/// The origin is `file: line N: ` — see `env::scope::record::origin` — so the file and the line are
/// already decided by the time anything here runs. All this adds is *reading* the file, finding the
/// word on that line, and handing ariadne a real source.
///
/// Every step may answer no, and each is ordinary: a `-c` string has no file, a prompt has no file,
/// `$LINENO` has not always been published, the file may have changed since it was read, and the
/// word may have come from an expansion and not appear in the text at all. Any of those falls back
/// to the rebuilt line, which is what the shell drew before this existed.
pub(super) fn in_the_script(
    origin: &str,
    word: &str,
    inside: Option<std::ops::Range<usize>>,
    message: &str,
    label: &str,
    help: Option<&str>,
) -> bool {
    let Some((path, line)) = file_and_line(origin) else {
        return false;
    };
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MOST_OF_A_SCRIPT) {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    // Where line `line` starts, counting the newline each earlier line ended with.
    let start: usize = text
        .lines()
        .take(line.saturating_sub(1))
        .map(|l| l.len() + 1)
        .sum();
    let Some(rest) = text.get(start..) else {
        return false;
    };
    let on_this_line = rest.split('\n').next().unwrap_or("");
    let Some(at) = on_this_line.find(word) else {
        return false;
    };
    // A caret inside the word when the caller asked for one — a single option letter out of a
    // cluster, or one name out of a comma-separated operand.
    let found = start + at;
    let span = match inside {
        Some(inside) if inside.end <= word.len() && inside.start < inside.end => {
            found + inside.start..found + inside.end
        }
        _ => found..found + word.len(),
    };
    diag::draw_source(
        &text,
        span,
        &diag::Report {
            message,
            source: path,
            label,
            help,
        },
    )
}

/// The file and the 1-based line an origin names, when it names both.
///
/// `script.sh: line 4: ` is the shape, and `oslo: ` — a prompt or a `-c` string — is not. A file
/// with no line is not enough either: `origin` writes that form when nothing has published a line
/// yet, and guessing at one would put the caret on a line that had nothing to do with it.
fn file_and_line(origin: &str) -> Option<(&str, usize)> {
    let (path, rest) = origin.split_once(": line ")?;
    let number = rest.strip_suffix(": ")?;
    Some((path, number.parse().ok()?))
}

/// What the report calls the line it is pointing into: the command's own name.
/// A Lua error, with a caret on the line the interpreter named.
///
/// **Lua says where.** Every error it raises is `chunk:line: message` — `init.lua:12: attempt to
/// index a nil value` — so the line is already in the text of the message, and the caller that ran
/// the chunk is holding the source it ran. Those two are everything a report needs.
///
/// The caret covers the **whole line** rather than a word, because that is the resolution Lua
/// works at: it reports a line and not a column, and inventing a column would be pointing at a
/// guess. Showing the line is what a person needs anyway — the message names the operation, the
/// line shows what it was applied to.
pub fn complain_lua(path: &str, text: &str, body: &str) -> bool {
    if !diag::enabled() {
        return false;
    }
    let message = format!("{}{body}", origin_now());
    let Some(line) = lua_line(body) else {
        return false;
    };
    let start: usize = text
        .lines()
        .take(line.saturating_sub(1))
        .map(|l| l.len() + 1)
        .sum();
    let Some(rest) = text.get(start..) else {
        return false;
    };
    let on_this_line = rest.split('\n').next().unwrap_or("");
    // The indent is not the mistake, so the caret starts where the code does.
    let indent = on_this_line.len() - on_this_line.trim_start().len();
    let end = on_this_line.trim_end().len();
    if end <= indent {
        return false;
    }
    diag::draw_source(
        text,
        start + indent..start + end,
        &diag::Report {
            message: &message,
            source: path,
            label: "raised here",
            help: None,
        },
    )
}

/// The line number in a `chunk:line: message`.
///
/// The first `:N:` in the message, because that is the shape Lua writes and a chunk name is a path
/// that will not contain one.
fn lua_line(message: &str) -> Option<usize> {
    let bytes = message.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b':' {
            at += 1;
            continue;
        }
        let start = at + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end > start && bytes.get(end) == Some(&b':') {
            return message[start..end].parse().ok();
        }
        at += 1;
    }
    None
}

#[cfg(test)]
mod script_tests {
    use super::{file_and_line, lua_line};

    /// The origin is `file: line N: `, and both halves have to come back out of it.
    #[test]
    fn an_origin_names_a_file_and_a_line() {
        assert_eq!(file_and_line("deploy.sh: line 4: "), Some(("deploy.sh", 4)));
        assert_eq!(
            file_and_line("/a/b/c.sh: line 140: "),
            Some(("/a/b/c.sh", 140))
        );
    }

    /// **A prompt and a `-c` string have no file**, and a file with no line is not enough either:
    /// `origin` writes that form when nothing has published a line yet, and guessing at one would
    /// put the caret on a line that had nothing to do with the error.
    #[test]
    fn an_origin_without_both_names_neither() {
        assert_eq!(file_and_line("oslo: "), None);
        assert_eq!(file_and_line("deploy.sh: "), None);
        assert_eq!(file_and_line("deploy.sh: line x: "), None);
        assert_eq!(file_and_line(""), None);
    }

    /// Lua writes `chunk:line: message`, and the chunk is a path that will not contain `:N:`.
    #[test]
    fn a_lua_error_names_its_line() {
        assert_eq!(
            lua_line("init.lua:3: could not index into a nil value"),
            Some(3)
        );
        assert_eq!(lua_line("/home/x/.config/oslo/init.lua:12: boom"), Some(12));
        assert_eq!(
            lua_line("[string \"where\"]:1: attempt to compare"),
            Some(1)
        );
    }

    #[test]
    fn a_message_with_no_line_has_none() {
        assert_eq!(lua_line("Lua error: out of memory"), None);
        assert_eq!(lua_line("init.lua: boom"), None);
        assert_eq!(lua_line(""), None);
    }
}

#[cfg(test)]
mod offset_tests {
    use super::offset_of;

    /// **The offset is what makes a chunk's line a file's line.** A script that does not parse is
    /// run a command at a time, and the parser counts from the start of the piece it was given.
    #[test]
    fn a_chunks_line_is_shifted_into_the_file() {
        let file = "echo one\necho two\necho three\nkill -s NOPE 1\n";
        // The parser saw only `kill -s NOPE 1` and called it line 1; three lines came before it.
        assert_eq!(offset_of(file, 3, "at 1,1"), Some(29), "the `k` of `kill`");
        assert_eq!(offset_of(file, 3, "at 1,9"), Some(37), "the `N` of `NOPE`");
    }

    /// An offset that walks off the end is refused rather than clamped: a caret in the wrong place
    /// is worse than no caret.
    #[test]
    fn an_offset_past_the_file_is_refused() {
        assert_eq!(offset_of("echo hi\n", 40, "at 1,1"), None);
    }
}
#[cfg(test)]
mod tests {
    use super::{line_and_column, offset_of};

    /// The parser and the tokenizer both spell a position `N,M`, in among a sentence.
    #[test]
    fn a_position_is_read_out_of_the_message() {
        assert_eq!(
            line_and_column("unterminated double quote at 1,6 (detected near line 2 col 1)"),
            Some((1, 6)),
            "the first pair, not the prose after it"
        );
        assert_eq!(line_and_column("near 12,3"), Some((12, 3)));
    }

    /// A message with no position draws nothing — which is the right answer for `syntax error at
    /// end of input`, an error about the *absence* of text.
    #[test]
    fn a_message_without_a_position_has_none() {
        assert_eq!(line_and_column("syntax error at end of input"), None);
        assert_eq!(line_and_column("no numbers here"), None);
        assert_eq!(line_and_column("one 5 number"), None);
        assert_eq!(line_and_column("a comma, and 5"), None);
        assert_eq!(line_and_column("5, 6"), None, "a space is not a position");
    }

    /// The line and column become a byte offset into the script, counting the newlines the lines
    /// were split on.
    #[test]
    fn a_position_becomes_an_offset() {
        let text = "echo one\necho two\nfor x in; do";
        assert_eq!(offset_of(text, 0, "at 1,1"), Some(0));
        assert_eq!(offset_of(text, 0, "at 1,6"), Some(5), "the `o` of `one`");
        assert_eq!(
            offset_of(text, 0, "at 2,1"),
            Some(9),
            "past the first newline"
        );
        assert_eq!(offset_of(text, 0, "at 3,1"), Some(18));
    }

    /// **Every offset is inside the text and on a character boundary**, or ariadne panics and, with
    /// `panic = "abort"`, takes the shell with it while it is reporting an error.
    #[test]
    fn an_offset_past_the_end_is_refused() {
        let text = "echo hi";
        assert_eq!(offset_of(text, 0, "at 9,1"), None, "no such line");
        assert_eq!(offset_of(text, 0, "at 1,99"), None, "no such column");
        assert_eq!(offset_of(text, 0, "at 0,1"), None, "lines are one-based");
        assert_eq!(offset_of(text, 0, "at 1,0"), None, "so are columns");
        assert_eq!(offset_of("", 0, "at 1,1"), None);
    }

    /// A column that lands inside a multi-byte character is floored to its start.
    #[test]
    fn an_offset_inside_a_character_is_floored() {
        let text = "echo é字";
        for column in 1..=10 {
            if let Some(at) = offset_of(text, 0, &format!("at 1,{column}")) {
                assert!(text.is_char_boundary(at), "column {column} gave {at}");
            }
        }
    }
}
