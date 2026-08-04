//! What the markdown subset renders, and — as much — what it leaves alone.

use super::*;

/// Escapes stripped, so a test asserts on structure rather than on the theme.
fn plain(rendered: &str) -> String {
    let mut out = String::new();
    let mut chars = rendered.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
    }
    out
}

fn md(text: &str) -> String {
    plain(&format(text, As::Markdown, &[]))
}

#[test]
fn headings_lose_their_hashes() {
    assert_eq!(md("# Title"), "\nTitle");
    assert_eq!(md("### Deeper"), "\nDeeper");
    // Not a heading without the space, which is how `#!/bin/sh` survives.
    assert_eq!(md("#nospace"), "#nospace");
    assert_eq!(md("#!/bin/sh"), "#!/bin/sh");
}

#[test]
fn bullets_become_bullets_and_numbers_renumber() {
    assert_eq!(md("- one\n- two"), "• one\n• two");
    assert_eq!(md("* one"), "• one");
    // The written numbers are ignored and the list is counted, as every markdown renderer does.
    assert_eq!(md("1. a\n1. b\n1. c"), "1. a\n2. b\n3. c");
}

/// A blank line ends a list, so the next one starts at 1 again.
#[test]
fn a_blank_line_restarts_the_numbering() {
    assert_eq!(md("1. a\n1. b\n\n1. x"), "1. a\n2. b\n\n1. x");
}

#[test]
fn inline_markers_are_consumed() {
    assert_eq!(md("**bold** text"), "bold text");
    assert_eq!(md("*italic* text"), "italic text");
    assert_eq!(md("`code` text"), "code text");
    // Longest marker first, or `**x**` reads as two italics around `*x*`.
    assert_eq!(md("**x**"), "x");
}

/// A link keeps its text and shows where it goes, since a terminal cannot be clicked.
#[test]
fn a_link_shows_its_target() {
    assert_eq!(
        md("[oslo](https://example.com)"),
        "oslo (https://example.com)"
    );
}

/// A fenced block is verbatim: its asterisks are asterisks.
#[test]
fn a_code_fence_is_left_alone() {
    let out = md("```\n**not bold**\n- not a bullet\n```");
    assert_eq!(out, "  **not bold**\n  - not a bullet");
}

#[test]
fn a_quote_is_marked() {
    assert_eq!(md("> quoted"), "│ quoted");
}

/// Anything unrecognised passes through rather than being swallowed — the property that keeps a
/// document this cannot render readable.
#[test]
fn unknown_markup_passes_through() {
    assert_eq!(md("| a | b |"), "| a | b |");
    assert_eq!(md("plain sentence."), "plain sentence.");
    assert_eq!(md("<html>"), "<html>");
}

/// A template fills what it knows and leaves what it does not, so a missing field is visible
/// where it was written instead of silently empty.
#[test]
fn a_template_leaves_unknown_names_alone() {
    let values = [("name".to_string(), "ada".to_string())];
    assert_eq!(
        format("hello {{name}}, {{missing}}", As::Template, &values),
        "hello ada, {{missing}}"
    );
}

#[test]
fn text_is_untouched() {
    let awkward = "**not bold** # not a heading";
    assert_eq!(format(awkward, As::Text, &[]), awkward);
}

#[test]
fn a_type_that_is_not_one_is_refused() {
    assert_eq!(As::parse("markdown"), Some(As::Markdown));
    assert_eq!(As::parse("md"), Some(As::Markdown));
    assert_eq!(As::parse("template"), Some(As::Template));
    assert_eq!(As::parse("code"), Some(As::Code));
    assert_eq!(As::parse("rtf"), None);
}
