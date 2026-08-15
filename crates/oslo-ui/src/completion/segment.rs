//! Where the word being completed really begins.
//!
//! A word as the lexer sees it is not always the thing Tab should complete. `--file=arch` is one
//! word to the shell and two questions to the user — an option and a path — and `{alpha,be` is a
//! brace list with an item half typed inside it. Both are handled the same way: hand back a
//! [`Word`] retargeted at the part actually being completed, so everything downstream goes on
//! treating it as an ordinary word and the replacement lands in the right place.

use crate::words::{Quote, Word, unquote};

/// Retarget a word at the item being typed inside an open `{a,b}` list.
///
/// `rm /dir/{alpha,be` completes `be` against `/dir/`, so the stem becomes `/dir/be` and `start`
/// moves to the `be`. Without this the whole word is one path with a literal brace in it, which
/// matches nothing and left completion silent from the `{` onwards.
pub(super) fn brace_segment(word: Word<'_>) -> Word<'_> {
    let Some(open) = unclosed_brace(word.text) else {
        return word;
    };
    let after = &word.text[open + 1..];
    let item = after.rfind(',').map_or(0, |c| c + 1);
    let head = &word.text[..open];
    let start = word.start + open + 1 + item;
    Word {
        start,
        text: &after[item..],
        stem: format!("{}{}", unquote(head), unquote(&after[item..])),
        ..word
    }
}

/// Retarget a word at the text after its last unquoted `=`.
///
/// **`=` ends a word for completion, as it does in bash**, where it is one of the characters in
/// `COMP_WORDBREAKS`. Without this every word containing one completed against nothing at all,
/// because the whole of `--file=some` was being looked up as a path and no such file exists:
///
/// ```text
/// dd if=/dev/ur          nothing
/// tar --file=arch        nothing
/// ./configure --prefix=/usr/lo   nothing
/// ls =some               nothing
/// ```
///
/// The last `=` rather than the first, so `--opt=a=b` completes `b` — the same rule bash applies,
/// and the only one that leaves an `=` inside a value alone.
///
/// **Not in a quoted word.** `ls "a=b` is somebody completing a filename that has an `=` in it,
/// where the quote is exactly the way to say "this is one word"; splitting there would take the
/// tool away from the person who reached for it.
///
/// `command_position` is cleared because what follows an `=` never names a command: the first word
/// of `FOO=bar ls` is an assignment, and offering the commands starting with `bar` for it would be
/// answering a question nobody asked.
pub(super) fn after_equals(word: Word<'_>) -> Word<'_> {
    if word.quote != Quote::None {
        return word;
    }
    let Some(at) = unquoted_equals(word.text) else {
        return word;
    };
    let after = &word.text[at + 1..];
    Word {
        start: word.start + at + 1,
        text: after,
        stem: unquote(after),
        command_position: false,
        ..word
    }
}

/// The offset of the last `=` in `text` that is not inside quotes and not escaped.
fn unquoted_equals(text: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    let mut last = None;
    for (i, c) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, c) {
            (None, '\\') => escaped = true,
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '=') => last = Some(i),
            _ => {}
        }
    }
    last
}

/// The offset of a `{` that is still open at the end of `text`, ignoring quoted ones.
fn unclosed_brace(text: &str) -> Option<usize> {
    let mut open: Vec<usize> = Vec::new();
    let mut quote = None;
    for (i, c) in text.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '{') => open.push(i),
            (None, '}') => {
                open.pop();
            }
            _ => {}
        }
    }
    open.pop()
}
