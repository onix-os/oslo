//! Painting, at a depth the test pins rather than inherits.

use super::*;
use oslo_ui::theme::held_at;

#[test]
fn an_inline_spec_paints_and_resets() {
    let _held = held_at(Depth::Ansi16);
    let painted = styles::parse("fg:red bold").paint("x", Depth::Ansi16);
    assert!(painted.starts_with('\x1b'), "{painted:?}");
    assert!(painted.ends_with("\x1b[0m"), "{painted:?}");
    assert!(painted.contains('x'));
}

/// **The reason a caller never checks.** At `NO_COLOR` the answer is the text, not an empty escape.
#[test]
fn no_colour_leaves_the_text_alone() {
    assert_eq!(styles::parse("fg:red bold").paint("x", Depth::None), "x");
}

#[test]
fn a_defined_name_resolves_to_what_it_was_defined_as() {
    styles::define("probe_warn", "fg:yellow bold");
    let looked = styles::lookup("probe_warn").expect("defined");
    assert!(looked.bold);
    styles::clear();
}

/// Every name the binding hands back must be one `Depth::named` takes.
#[test]
fn the_depth_names_round_trip() {
    for depth in [Depth::None, Depth::Ansi16, Depth::Ansi256, Depth::True] {
        assert_eq!(Depth::named(name_of(depth)), Some(depth), "{depth:?}");
    }
}
