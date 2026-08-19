//! What a Lua line is painted as.
//!
//! Each check asks whether a token was painted with a *named* style, by building what that style
//! would produce and looking for it. Reverse-mapping escapes back to a style name cannot work: two
//! kinds may share a colour in any given theme, so the answer would depend on the theme rather than
//! on the highlighter.

use super::*;
use crate::theme::{Depth, Theme};

const DEPTH: Depth = Depth::Ansi256;

fn nothing_known(_: &str) -> Option<bool> {
    None
}

/// Whether `text` appears painted with `style`.
#[track_caller]
fn painted_with(
    line: &str,
    text: &str,
    style: &Style,
    known: &dyn Fn(&str) -> Option<bool>,
) -> bool {
    let out = paint(line, &Theme::default(), DEPTH, known);
    out.contains(&style.paint(text, DEPTH))
}

/// **A keyword, a number, a string and a comment are each coloured by the text alone.**
#[test]
fn the_text_alone_decides_keywords_numbers_strings_and_comments() {
    let theme = Theme::default();
    assert!(painted_with(
        "local x = 42",
        "local",
        &theme.syntax.keyword,
        &nothing_known
    ));
    assert!(painted_with(
        "local x = 42",
        "42",
        &theme.syntax.number,
        &nothing_known
    ));
    assert!(painted_with(
        "f('a')",
        "'a'",
        &theme.syntax.single_quote,
        &nothing_known
    ));
    assert!(painted_with(
        "f(\"b\")",
        "\"b\"",
        &theme.syntax.double_quote,
        &nothing_known
    ));
    assert!(painted_with(
        "x = 1 -- why",
        "-- why",
        &theme.syntax.comment,
        &nothing_known
    ));
}

/// **An unknown name is left plain, never marked wrong.**
///
/// The whole reason the shell's rules could not be reused: they paint a name that resolves to
/// nothing in the *error* style, and at a Lua prompt a name that does not exist yet is the normal
/// case — it is what you are about to assign to.
#[test]
fn an_unknown_name_is_not_an_error() {
    let theme = Theme::default();
    let out = paint("zzz = 1", &theme, DEPTH, &nothing_known);
    assert!(
        !out.contains(&theme.syntax.error.paint("zzz", DEPTH)),
        "an undefined name must not be painted as an error: {out:?}"
    );
    assert!(painted_with(
        "zzz = 1",
        "zzz",
        &theme.syntax.param,
        &nothing_known
    ));
}

/// A name the session knows is drawn as what it is.
#[test]
fn a_known_name_is_drawn_as_what_it_is() {
    let theme = Theme::default();
    let known = |name: &str| match name {
        "print" => Some(true),
        "math" => Some(false),
        _ => None,
    };
    assert!(painted_with(
        "print(math.pi)",
        "print",
        &theme.syntax.function,
        &known
    ));
    assert!(painted_with(
        "print(math.pi)",
        "math",
        &theme.syntax.builtin,
        &known
    ));
}

/// A name followed by `(` reads as a call before anything defines it, because the text says so.
#[test]
fn a_call_is_a_call_before_it_exists() {
    let theme = Theme::default();
    assert!(painted_with(
        "later_on()",
        "later_on",
        &theme.syntax.function,
        &nothing_known
    ));
}

/// **A half-typed line still paints.** A highlighter runs on every keystroke against text that
/// usually does not parse, so an unterminated string simply colours to the end of the line — which
/// is what it looks like to a reader too.
#[test]
fn a_half_typed_line_still_paints() {
    let theme = Theme::default();
    assert!(painted_with(
        "print(\"unfinis",
        "\"unfinis",
        &theme.syntax.double_quote,
        &nothing_known
    ));
    assert!(painted_with(
        "x = 1 --",
        "--",
        &theme.syntax.comment,
        &nothing_known
    ));
}
