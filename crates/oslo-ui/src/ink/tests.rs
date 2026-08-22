//! The builder, pinned at a named depth so no test depends on the terminal it runs in.

use super::*;
use crate::theme::Depth;

#[test]
fn a_basic_colour_is_the_sgr_everybody_knows() {
    assert_eq!(ink("x").red().at(Depth::Ansi16), "\x1b[31mx\x1b[0m");
    assert_eq!(ink("x").on_blue().at(Depth::Ansi16), "\x1b[44mx\x1b[0m");
}

#[test]
fn bright_is_the_high_intensity_pair() {
    assert_eq!(ink("x").bright_red().at(Depth::Ansi16), "\x1b[91mx\x1b[0m");
    assert_eq!(
        ink("x").on_bright_red().at(Depth::Ansi16),
        "\x1b[101mx\x1b[0m"
    );
}

/// The whole point of the shape: it reads left to right and every call keeps the last.
#[test]
fn calls_chain_and_do_not_clobber_one_another() {
    let painted = ink("x").green().on_black().bold().underline();
    let out = painted.at(Depth::Ansi16);
    assert!(out.contains("32"), "no green: {out:?}");
    assert!(out.contains("40"), "no black background: {out:?}");
    assert!(out.contains('1'), "no bold: {out:?}");
    assert!(out.contains('4'), "no underline: {out:?}");
}

/// Naming the same axis twice keeps the last, as an assignment does.
#[test]
fn the_last_colour_on_an_axis_wins() {
    assert_eq!(
        ink("x").red().green().at(Depth::Ansi16),
        ink("x").green().at(Depth::Ansi16)
    );
}

#[test]
fn the_three_attributes_colored_has_and_style_did_not() {
    assert_eq!(ink("x").blink().at(Depth::Ansi16), "\x1b[5mx\x1b[0m");
    assert_eq!(ink("x").hidden().at(Depth::Ansi16), "\x1b[8mx\x1b[0m");
    assert_eq!(ink("x").strike().at(Depth::Ansi16), "\x1b[9mx\x1b[0m");
}

/// **The reason a caller never checks.** At `NO_COLOR` the answer is the text, not an empty escape.
#[test]
fn no_colour_is_the_text_itself() {
    assert_eq!(ink("x").red().bold().at(Depth::None), "x");
    assert_eq!(ink("x").red().plain(), "x");
}

/// Truecolour survives to a terminal that has it and is downgraded, not dropped, where it does not.
#[test]
fn rgb_reaches_a_true_colour_terminal_and_degrades_below_it() {
    let full = ink("x").rgb(0x61, 0xff, 0xca).at(Depth::True);
    assert!(full.contains("38;2;97;255;202"), "{full:?}");
    let lower = ink("x").rgb(0x61, 0xff, 0xca).at(Depth::Ansi256);
    assert!(lower.starts_with('\x1b'), "colour was dropped: {lower:?}");
    assert!(
        !lower.contains("38;2;"),
        "truecolour on a 256 terminal: {lower:?}"
    );
}

/// A theme's spelling, so a config value and a call site mean the same thing.
#[test]
fn a_named_colour_is_the_themes_spelling() {
    assert_eq!(
        ink("x").named("green").at(Depth::Ansi16),
        ink("x").green().at(Depth::Ansi16)
    );
    assert_eq!(
        ink("x").named("#61ffca").at(Depth::True),
        ink("x").rgb(0x61, 0xff, 0xca).at(Depth::True)
    );
}

/// **An unreadable name leaves the colour alone.** A typo should read as "that is not coloured",
/// which is visible, rather than as a colour nobody chose.
#[test]
fn a_colour_nobody_can_read_changes_nothing() {
    assert_eq!(ink("x").named("chartreuse").at(Depth::Ansi16), "x");
}

#[test]
fn plain_text_emits_no_escapes_at_all() {
    assert_eq!(ink("x").at(Depth::True), "x");
}
