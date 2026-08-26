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
/// **At column zero.** It was right-aligned under the primary prompt so the code lined up with the
/// first line; that put the marker adrift in the middle of the row, a different distance from the
/// left edge in every directory, and the thing you are reading down a block is the *marker* — it is
/// what tells you the block is still open and how deep it is. A fixed left edge is what makes that
/// readable.
pub fn continuation_marker(language: &str, depth_of_block: usize) -> String {
    let mark = match language {
        // One extra `>` per level of nesting, so the marker says how deep the block is and how many
        // `end`s are still owed. A block you have been typing for four lines is exactly where that
        // stops being obvious from looking at it.
        "lua" => ">".repeat(3 + depth_of_block.min(6)),
        _ => ">".to_string(),
    };
    let theme = super::theme::current();
    // One trailing space, so the text does not touch the marker.
    theme
        .prompt
        .continuation
        .paint(&format!("{mark} "), super::theme::depth())
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
///
/// **Every step is a whole character wide.** It used to read `bytes[i] as char` and advance by that
/// character's width, which is a different number: byte `0xE6` — the first of `日` — reads as
/// `U+00E6`, two bytes wide, so the cursor advanced two of the three and landed *inside* the
/// character. The next `source[i..]` then panicked, and with `panic = "abort"` the shell went with
/// it. Typing a non-ASCII word at a Lua prompt was enough.
fn words_outside_text(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut i = 0;
    while let Some(c) = source[i..].chars().next() {
        if source[i..].starts_with("--") {
            // To the end of the line, which is where a short comment stops. `\n` is ASCII, so the
            // offset `find` answers with is a boundary.
            i += source[i..].find('\n').unwrap_or(source.len() - i);
            continue;
        }
        if c == '"' || c == '\'' {
            i += c.len_utf8();
            while let Some(inside) = source[i..].chars().next() {
                i += inside.len_utf8();
                if inside == '\\' {
                    // The escaped character goes with it, whatever its width.
                    i += source[i..].chars().next().map_or(0, char::len_utf8);
                    continue;
                }
                if inside == c {
                    break;
                }
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            word.push(c);
            i += c.len_utf8();
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

    /// **`>>>` at a Lua prompt, `>` at a shell one**, at column zero.
    #[test]
    fn the_marker_names_the_language_and_lines_the_block_up() {
        assert_eq!(strip(&continuation_marker("lua", 0)), ">>> ");
        assert_eq!(strip(&continuation_marker("sh", 0)), "> ");
    }

    /// **One extra `>` per level**, so the marker says how many `end`s are still owed.
    #[test]
    fn the_marker_grows_with_the_nesting() {
        assert_eq!(strip(&continuation_marker("lua", 1)), ">>>> ");
        assert_eq!(strip(&continuation_marker("lua", 3)), ">>>>>> ");
        // Capped, so a deeply nested block cannot push the code off the screen.
        assert_eq!(strip(&continuation_marker("lua", 99)), ">>>>>>>>> ");
        // A shell block is unaffected: its marker says nothing about depth.
        assert_eq!(strip(&continuation_marker("sh", 3)), "> ");
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

/// **Non-ASCII input must not take the shell with it.**
///
/// Every step through these scanners used to be `bytes[i] as char` wide rather than the character's
/// own width, so a multibyte character left the cursor inside itself and the next slice panicked.
/// With `panic = "abort"` that is the whole session, from typing an ordinary word.
#[cfg(test)]
mod utf8_tests {
    use super::words_outside_text;

    #[test]
    fn a_multibyte_word_is_scanned_without_panicking() {
        // Each of these panicked before: the scanner stepped two bytes into a three-byte character.
        for source in [
            "for f in 日本 do",
            "local x = '日'",
            "-- 日本語のコメント\nfunction f",
            "local s = \"a\\日b\" end",
            "日",
            "x = 'é' .. \"ü\"",
        ] {
            let _ = words_outside_text(source);
        }
    }

    /// …and the words either side of it are still found, so the depth count stays right.
    #[test]
    fn the_keywords_around_a_multibyte_word_are_still_read() {
        let words = words_outside_text("function 日本() return 1 end");
        assert!(words.contains(&"function".to_string()), "{words:?}");
        assert!(words.contains(&"end".to_string()), "{words:?}");
        assert!(words.contains(&"return".to_string()), "{words:?}");
    }

    /// A multibyte character inside a string stays inside it: the closing quote is still found, so
    /// the `end` after it is a keyword rather than more string.
    #[test]
    fn a_string_holding_a_multibyte_character_still_closes() {
        let words = words_outside_text("local x = '日本' end");
        assert_eq!(
            words,
            vec!["local".to_string(), "x".to_string(), "end".to_string()]
        );
    }
}
