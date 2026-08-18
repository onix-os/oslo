//! The marker in front of a continuation line.

/// The marker in front of a **continuation** line, padded to sit under the primary prompt.
///
/// `>>>` at a Lua prompt and `>` at a shell one. Not decoration: the two languages end a block
/// differently — an empty line finishes a Lua block, an unclosed quote or `do` finishes a shell one
/// — so the marker says which set of rules you are inside.
///
/// **Right-aligned to `width`**, the printed width of the primary prompt this block began under.
/// The point is that the *code* lines up: everything you type on a continuation row starts in the
/// same column as the first line, so a block reads as a block instead of as a ragged left edge. A
/// `width` of zero, or one too small to hold the marker, just gives the marker and a space.
pub fn continuation_marker(language: &str, width: usize) -> String {
    let mark = match language {
        "lua" => ">>>",
        _ => ">",
    };
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

#[cfg(test)]
mod tests {
    use super::continuation_marker;

    /// **`>>>` at a Lua prompt, `>` at a shell one**, padded so the code lines up under the first
    /// line of the block rather than starting in column zero.
    #[test]
    fn the_marker_names_the_language_and_lines_the_block_up() {
        let lua = continuation_marker("lua", 20);
        let plain: String = strip(&lua);
        assert_eq!(plain, format!("{}>>> ", " ".repeat(16)));
        assert_eq!(plain.chars().count(), 20, "it fills the prompt's width");

        let shell = strip(&continuation_marker("sh", 20));
        assert_eq!(shell, format!("{}> ", " ".repeat(18)));

        // A width too small to hold the marker gives the marker rather than a panic or a negative
        // pad — the block simply starts at column zero.
        assert_eq!(strip(&continuation_marker("lua", 0)), ">>> ");
        assert_eq!(strip(&continuation_marker("lua", 2)), ">>> ");
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
