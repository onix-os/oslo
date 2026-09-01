//! Saying a diagnostic once, in whichever of its two faces the reader can use.
//!
//! [`origin_now`](super::origin_now) answers *where* a diagnostic is speaking from. This answers
//! *how* it is drawn: a one-line message to a pipe, and on a terminal the same message with a caret
//! under the word at fault.
//!
//! ```text
//! oslo: kill: NOPE: invalid signal specification      ← a pipe, a script, a test
//!
//! oslo: kill: NOPE: invalid signal specification      ← a terminal
//!    ╭─[ kill:1:9 ]
//!  1 │ kill -s NOPE 1
//!    │         ──┬─
//!    │           ╰─── not a signal
//!    │ Help: a signal is a name (TERM), a number (15), or SIG-prefixed
//! ───╯
//! ```
//!
//! # One call, not five
//!
//! There are two hundred and fifty diagnostics in the builtins alone, and every one of them is
//! today a single `eprintln!`. If converting one meant five lines — ask whether to draw, build a
//! snapshot, find the word, build a report, fall back — then converting them all would be a
//! thousand lines of the same five, and the two hundred and fifty-first would be written the old
//! way because the new way is a chore.
//!
//! So it is one call with the same shape as the `eprintln!` it replaces, and the fallback is inside
//! it. A caller that has nothing to point at keeps its `eprintln!`; that is a decision about the
//! error, not an omission.
//!
//! # What `body` is
//!
//! Everything after the origin — `kill: NOPE: invalid signal specification` — exactly as the
//! `eprintln!` wrote it. **The message a pipe sees is byte-for-byte what it saw before**, which is
//! what `tests/diagnostics_stay_plain.rs` exists to hold true, and it is also the report's own
//! first line, so the two faces cannot drift into saying different things.

use super::origin_now;
use oslo_base::diag;

/// The one-liner, with a caret under `word` when there is a terminal to draw one on.
///
/// `words` is the command as the shell has it — the name and its operands. `word` is the one at
/// fault; when it is not among them the report is skipped and the one-liner printed, which is the
/// right answer for a word the message rewrote on its way there.
///
/// **Answers whether it drew**, which most callers ignore. The ones that do not are the ones with a
/// second line to print — a usage block after the diagnostic — because that line belongs *inside*
/// the report as its help and *after* the message when there is no report. Ignoring the answer is
/// always safe; the message is printed either way.
pub fn complain(words: &[String], word: &str, body: &str, label: &str, help: Option<&str>) -> bool {
    complain_from(&origin_now(), words, word, body, label, help)
}

/// The same, for a caller that already holds the origin.
///
/// **Not every diagnostic comes from a builtin.** `x=1` against a readonly name is a complaint about
/// a *line of a script* with no builtin involved, and `rm` walks a tree carrying the origin it
/// started with — neither has published one to the thread-local [`origin_now`] reads, so calling
/// that would print `oslo: ` where the file and line belong. The prefix a site already has is the
/// right one, and passing it is what keeps the message byte-identical.
pub fn complain_from(
    origin: &str,
    words: &[String],
    word: &str,
    body: &str,
    label: &str,
    help: Option<&str>,
) -> bool {
    let message = format!("{origin}{body}");
    if drawn(origin, words, word, &message, label, help) {
        return true;
    }
    eprintln!("{message}");
    false
}

/// The one-liner followed by a usage block — the commonest shape in the builtins.
///
/// **The block moves into the report rather than under it.** Printed beneath a drawn box it reads
/// as a second, unrelated message; as the report's help it is the answer to the question the caret
/// just asked. Where nothing is drawn it is the line it has always been, in the place it has always
/// been, which is what a pipe still sees.
pub fn complain_with_usage(words: &[String], word: &str, body: &str, label: &str, usage: &str) {
    if !complain(words, word, body, label, Some(usage)) {
        eprintln!("{usage}");
    }
}

/// The same, with the caret under one option **letter**.
///
/// `-pqz` is one word carrying three options, and a message about `-z` names a word that is not in
/// the argv at all. So the letter is found inside whichever word groups it, and the caret is that
/// wide — one character, where the mistake is.
pub fn complain_option(words: &[String], letter: char, body: &str, usage: &str) {
    let grouped = words.iter().find(|word| {
        word.starts_with('-')
            && !word.starts_with("--")
            && word.chars().skip(1).any(|c| c == letter)
    });
    let drew = match grouped {
        Some(word) => {
            // Byte offset, because that is what the span is measured in — a grouped option can
            // follow a multi-byte character in a word somebody typed by accident.
            let at = word
                .char_indices()
                .find(|(_, c)| *c == letter)
                .map(|(at, c)| at..at + c.len_utf8());
            match at {
                Some(inside) => {
                    complain_within(words, word, inside, body, "not an option here", Some(usage))
                }
                None => false,
            }
        }
        None => complain(
            words,
            &format!("-{letter}"),
            body,
            "not an option here",
            Some(usage),
        ),
    };
    if !drew {
        eprintln!("{usage}");
    }
}

/// The same, for a caret under part of a word: `cols a,b,nmae` under `nmae` alone.
///
/// `inside` is a byte range within `word`.
pub fn complain_within(
    words: &[String],
    word: &str,
    inside: std::ops::Range<usize>,
    body: &str,
    label: &str,
    help: Option<&str>,
) -> bool {
    complain_within_from(&origin_now(), words, word, inside, body, label, help)
}

/// The same, for a caller that already holds the origin — see [`complain_from`].
#[allow(clippy::too_many_arguments)]
pub fn complain_within_from(
    origin: &str,
    words: &[String],
    word: &str,
    inside: std::ops::Range<usize>,
    body: &str,
    label: &str,
    help: Option<&str>,
) -> bool {
    let message = format!("{origin}{body}");
    if diag::enabled() {
        // The script first, for the same reason [`drawn`] tries it first: a real path and a real
        // line beat a rebuilt one.
        if in_the_script(origin, word, Some(inside.clone()), &message, label, help) {
            return true;
        }
        let report = diag::Report {
            message: &message,
            source: source_of(words),
            label,
            help,
        };
        let snapshot = diag::Snapshot::of(words);
        if let Some(at) = snapshot.index_of(word)
            && snapshot.draw_within(at, inside, &report)
        {
            return true;
        }
    }
    eprintln!("{message}");
    false
}

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
pub fn complain_at(name: &str, text: &str, body: &str) -> bool {
    if !diag::enabled() {
        return false;
    }
    let message = format!("{}{body}", origin_now());
    let Some(at) = offset_of(text, body) else {
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
fn offset_of(text: &str, message: &str) -> Option<usize> {
    let (line, column) = line_and_column(message)?;
    let start = text
        .lines()
        .take(line.checked_sub(1)?)
        .map(|l| l.len() + 1)
        .sum::<usize>();
    let at = start + column.checked_sub(1)?;
    (at < text.len()).then(|| diag::floor_boundary(text, at))
}

/// The first `N,M` in a message, as numbers.
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

/// Whether a report was drawn. Split out so both entry points ask the question the same way.
fn drawn(
    origin: &str,
    words: &[String],
    word: &str,
    message: &str,
    label: &str,
    help: Option<&str>,
) -> bool {
    // **Asked before anything is built.** On a pipe this is the whole cost of the feature: one
    // cached bool, no snapshot, no format beyond the message that was going to be printed anyway.
    if !diag::enabled() {
        return false;
    }
    // **A real file beats a rebuilt line, every time.** When the diagnostic came from a script the
    // origin already names the file and the line; reading that line and pointing into it gives a
    // path, a line number and the code as written — which is the difference between a caret and a
    // compiler's diagnostic. The snapshot below is the fallback for a prompt and a `-c` string,
    // which have no file to name.
    if in_the_script(origin, word, None, message, label, help) {
        return true;
    }
    let snapshot = diag::Snapshot::of(words);
    let Some(at) = snapshot.index_of(word) else {
        return false;
    };
    snapshot.draw(
        at,
        &diag::Report {
            message,
            source: source_of(words),
            label,
            help,
        },
    )
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
fn in_the_script(
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
///
/// A builtin's argv is not a file, and pretending it has a path would be a lie in the one place a
/// person looks for one. `kill:1:9` reads as "the ninth column of what you typed", which is what it
/// is.
fn source_of(words: &[String]) -> &str {
    words.first().map(String::as_str).unwrap_or("oslo")
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
        assert_eq!(offset_of(text, "at 1,1"), Some(0));
        assert_eq!(offset_of(text, "at 1,6"), Some(5), "the `o` of `one`");
        assert_eq!(offset_of(text, "at 2,1"), Some(9), "past the first newline");
        assert_eq!(offset_of(text, "at 3,1"), Some(18));
    }

    /// **Every offset is inside the text and on a character boundary**, or ariadne panics and, with
    /// `panic = "abort"`, takes the shell with it while it is reporting an error.
    #[test]
    fn an_offset_past_the_end_is_refused() {
        let text = "echo hi";
        assert_eq!(offset_of(text, "at 9,1"), None, "no such line");
        assert_eq!(offset_of(text, "at 1,99"), None, "no such column");
        assert_eq!(offset_of(text, "at 0,1"), None, "lines are one-based");
        assert_eq!(offset_of(text, "at 1,0"), None, "so are columns");
        assert_eq!(offset_of("", "at 1,1"), None);
    }

    /// A column that lands inside a multi-byte character is floored to its start.
    #[test]
    fn an_offset_inside_a_character_is_floored() {
        let text = "echo é字";
        for column in 1..=10 {
            if let Some(at) = offset_of(text, &format!("at 1,{column}")) {
                assert!(text.is_char_boundary(at), "column {column} gave {at}");
            }
        }
    }
}

/// A command line rebuilt from its name and its operands.
///
/// **For the many functions handed `names` rather than argv.** `print_variables(env, names)`,
/// `export_functions(env, names)`, `select(jobs, operands, name)` — each knows the operands it is
/// working through and the builtin it belongs to, and neither has the original `args` slice. Those
/// two are what a source line is: `export foo bar` is faithful enough to point into, and threading
/// argv down through five signatures to recover the same three words is not.
pub fn line(name: &str, operands: &[String]) -> Vec<String> {
    std::iter::once(name.to_string())
        .chain(operands.iter().cloned())
        .collect()
}

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
