//! The marker in front of a continuation line.

/// The marker in front of a **continuation** line, padded to sit under the primary prompt.
///
/// `>>>` at a Lua prompt and `>` at a shell one. Not decoration: the two languages end a block
/// differently — an empty line finishes a Lua block, an unclosed quote or `do` finishes a shell one
/// — so the marker says which set of rules you are inside.
///
/// **One extra `>` per level of nesting**, so the marker also says how many `end`s are still owed.
/// Four lines into a block that is not obvious from looking at it, and counting `end`s by eye is
/// exactly the thing an editor should be doing for you. Capped, so a deeply nested block cannot
/// push the code off the screen.
///
/// **Right-aligned to `width`**, the printed width of the primary prompt this block began under.
/// The point is that the *code* lines up: everything you type on a continuation row starts in the
/// same column as the first line, so a block reads as a block instead of as a ragged left edge. A
/// `width` of zero, or one too small to hold the marker, just gives the marker and a space.
pub fn continuation_marker(language: &str, width: usize, depth_of_block: usize) -> String {
    let mark = match language {
        // One extra `>` per level of nesting, so the marker says how deep the block is and how many
        // `end`s are still owed. A block you have been typing for four lines is exactly where that
        // stops being obvious from looking at it.
        "lua" => ">".repeat(3 + depth_of_block.min(6)),
        _ => ">".to_string(),
    };
    let mark = mark.as_str();
    let theme = super::theme::current();
    let depth = super::theme::depth();
    // One trailing space, so the text does not touch the marker.
    let painted = theme.prompt.continuation.paint(&format!("{mark} "), depth);
    // Padding is plain spaces rather than part of the painted run: a background colour stretched
    // across the indent would draw a bar to the left of every continuation line.
    match width.checked_sub(mark.chars().count() + 1) {
        Some(pad) => format!("{}{painted}", " ".repeat(pad)),
        None => painted,
    }
}

/// How deeply nested the Lua source in `block` is: how many `end`s and `until`s it still owes.
///
/// **Counted from tokens, not parsed.** This runs against a block that is half-written by
/// definition, so it reads the words that open and close a scope and ignores everything else. It
/// skips strings and comments, because `-- end` and `"end"` are not `end` — that is the one thing a
/// counter like this gets wrong often enough to be worth doing properly.
///
/// A `then` or a `do` that belongs to an `if`/`while`/`for` is not counted twice: only the opener
/// counts, and `do`/`then` are counted only when nothing has opened them.
pub fn block_depth(block: &str) -> usize {
    let mut depth: i32 = 0;
    // Set by `if`/`while`/`for`, and cleared by the `then`/`do` that belongs to it — so that pair
    // adds one level between them rather than two.
    let mut awaiting_body = false;
    for word in words_outside_text(block) {
        match word.as_str() {
            "if" | "while" | "for" => {
                depth += 1;
                awaiting_body = true;
            }
            "then" | "do" if awaiting_body => awaiting_body = false,
            // A bare `do` block, or a `do` with no opener in front of it.
            "do" => depth += 1,
            "function" | "repeat" => depth += 1,
            "end" | "until" => depth -= 1,
            _ => {}
        }
    }
    depth.max(0) as usize
}

/// The bare words of `source`, with strings and comments left out.
fn words_outside_text(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut word = String::new();
    while i < bytes.len() {
        let c = bytes[i] as char;
        if source[i..].starts_with("--") {
            // To the end of the line, which is where a short comment stops.
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == '"' || c == '\'' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] as char == c {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            word.push(c);
            i += 1;
            continue;
        }
        if !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
        i += c.len_utf8();
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::continuation_marker;

    /// **`>>>` at a Lua prompt, `>` at a shell one**, padded so the code lines up under the first
    /// line of the block rather than starting in column zero.
    #[test]
    fn the_marker_names_the_language_and_lines_the_block_up() {
        let lua = continuation_marker("lua", 20, 0);
        let plain: String = strip(&lua);
        assert_eq!(plain, format!("{}>>> ", " ".repeat(16)));
        assert_eq!(plain.chars().count(), 20, "it fills the prompt's width");

        let shell = strip(&continuation_marker("sh", 20, 0));
        assert_eq!(shell, format!("{}> ", " ".repeat(18)));

        // A width too small to hold the marker gives the marker rather than a panic or a negative
        // pad — the block simply starts at column zero.
        assert_eq!(strip(&continuation_marker("lua", 0, 0)), ">>> ");
        assert_eq!(strip(&continuation_marker("lua", 2, 0)), ">>> ");
    }

    /// **One extra `>` per level**, so the marker says how many `end`s are still owed.
    #[test]
    fn the_marker_grows_with_the_nesting() {
        assert_eq!(strip(&continuation_marker("lua", 0, 1)), ">>>> ");
        assert_eq!(strip(&continuation_marker("lua", 0, 3)), ">>>>>> ");
        // Capped, so a deeply nested block cannot push the code off the screen.
        assert_eq!(strip(&continuation_marker("lua", 0, 99)), ">>>>>>>>> ");
        // A shell block is unaffected: its marker says nothing about depth.
        assert_eq!(strip(&continuation_marker("sh", 0, 3)), "> ");
    }

    /// **Depth is counted from tokens, and strings and comments are not tokens.**
    ///
    /// `-- end` and `"end"` are the cases a naive counter gets wrong, and getting them wrong means
    /// the marker shrinks while the block is still open.
    #[test]
    fn the_depth_counts_what_is_still_open() {
        use super::block_depth;
        assert_eq!(block_depth(""), 0);
        assert_eq!(block_depth("local function f()"), 1);
        assert_eq!(block_depth("local function f()\nend"), 0);
        assert_eq!(block_depth("if x then"), 1);
        assert_eq!(
            block_depth("for i = 1, 3 do"),
            1,
            "the opener and its `do` are one level"
        );
        assert_eq!(block_depth("do"), 1, "a bare do block still opens one");
        assert_eq!(block_depth("repeat"), 1);
        assert_eq!(block_depth("if x then\n  if y then"), 2);

        // Text is skipped.
        assert_eq!(
            block_depth("if x then\n  -- end"),
            1,
            "a comment is not an `end`"
        );
        assert_eq!(
            block_depth("if x then\n  s = \"end\""),
            1,
            "a string is not an `end`"
        );

        // More `end`s than openers is zero, not a negative that would panic on `repeat`.
        assert_eq!(block_depth("end end end"), 0);
    }

    fn strip(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c != '\u{1b}' {
                out.push(c);
                continue;
            }
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
        out
    }
}
