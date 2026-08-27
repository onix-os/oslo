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
    let head = unquote(&word.text[..open]);
    let start = word.start + open + 1 + item;
    Word {
        start,
        text: &after[item..],
        stem: format!("{}{}", head, unquote(&after[item..])),
        // The `/dir/` is in the stem so the directory can be read, and on the line already — so a
        // completion must not write it again. See [`Word::carried`].
        carried: head.len(),
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
/// **`:` too, and for the same reason.** It is the other break character people meet every day —
/// `FOO=/usr/bin:/us` is somebody editing a `$PATH`, and `scp host:/pa` is somebody naming a remote
/// file. Both completed against nothing before this.
///
/// The last one rather than the first, so `--opt=a=b` completes `b` and `/a:/b:/c` completes `/c` —
/// the same rule bash applies, and the only one that leaves an earlier break inside a value alone.
///
/// **Not in a quoted word.** `ls "a=b` is somebody completing a filename that has an `=` in it,
/// where the quote is exactly the way to say "this is one word"; splitting there would take the
/// tool away from the person who reached for it.
///
/// `command_position` is cleared because what follows a break never names a command: the first
/// word of `FOO=bar ls` is an assignment, and offering the commands starting with `bar` for it
/// would be answering a question nobody asked.
pub(super) fn after_break(word: Word<'_>) -> Word<'_> {
    if word.quote != Quote::None {
        return word;
    }
    let Some(at) = unquoted_break(word.text) else {
        return word;
    };
    let after = &word.text[at + 1..];
    Word {
        start: word.start + at + 1,
        text: after,
        stem: unquote(after),
        command_position: false,
        // The stem is only what follows the break, so all of it is being replaced.
        carried: 0,
        // …and what came off the front is kept, because a spec answers for `--file=` differently
        // from how it answers for a bare path.
        prefix: &word.text[..=at],
        ..word
    }
}

/// Where a word is broken for completion, as `COMP_WORDBREAKS` does it in bash.
///
/// Only these two. The rest of bash's set — `"'><;|&(` — are characters the lexer has already
/// split on by the time a word reaches here, so listing them would be describing work that is
/// done.
const BREAKS: [char; 2] = ['=', ':'];

/// The offset of the last break character in `text` that is not inside quotes and not escaped.
fn unquoted_break(text: &str) -> Option<usize> {
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
            (None, c) if BREAKS.contains(&c) => last = Some(i),
            _ => {}
        }
    }
    last
}

/// The offset of a `{` that is still open at the end of `text`, ignoring quoted ones.
///
/// **`${` opens a parameter expansion, not a list**, and the two cannot share a stack. Counted as a
/// list, `echo ${HO` was retargeted at `HO` with the `$` folded into its stem, and taking the one
/// candidate spliced the line into `echo ${$HOME` — text the user never typed, on the ordinary way
/// of writing `${HOME}`.
///
/// Its `}` has to be skipped with it rather than the `{` alone: popping the shared stack there
/// would close somebody else's list, and `rm /d/{a,${X}b` would lose the brace it is really inside.
fn unclosed_brace(text: &str) -> Option<usize> {
    /// A `{` still waiting for its `}`, and whether a list or an expansion opened it.
    struct Open {
        at: usize,
        list: bool,
    }
    let mut open: Vec<Open> = Vec::new();
    let mut quote = None;
    let bytes = text.as_bytes();
    for (i, c) in text.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '{') => open.push(Open {
                at: i,
                list: i == 0 || bytes[i - 1] != b'$',
            }),
            (None, '}') => {
                open.pop();
            }
            _ => {}
        }
    }
    // The innermost one, and only if a list opened it: inside an unclosed `${…}` the word being
    // typed is a variable name, which the `$` branch of the completer answers for.
    open.pop().filter(|last| last.list).map(|last| last.at)
}
