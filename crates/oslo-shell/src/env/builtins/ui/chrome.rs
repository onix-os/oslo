//! The presentation options every widget shares: the legend, a border, the whole screen, and
//! where on it.
//!
//! One parser rather than the same six arms copied into eight `match`es — which is how
//! `ui input --border` and `ui choose --border` would end up meaning subtly different things.
//! The Lua side reads the same names from a table; both build the same [`Chrome`].

use super::take;
use crate::env::origin_now;
use oslo_ui::ask::Border;
use oslo_ui::ask::chrome::{Chrome, Fit, Place};
use oslo_ui::theme;

/// What happened when a widget's option loop offered a flag to the shared chrome parser.
pub(super) enum Chromed {
    /// It was a chrome flag and has been consumed.
    Took,
    /// Not one of these; the widget should report it as unknown.
    NotMine,
    /// It was one and its value was wrong. The status to exit with.
    Bad(i32),
}

/// The options every widget shares: the legend, a border, the whole screen, and where on it.
///
/// One parser rather than the same six arms copied into eight `match`es — which is how `ui input
/// --border` and `ui choose --border` would end up meaning subtly different things. The Lua side
/// reads the same fields from a table; both end up building the same [`Chrome`].
pub(super) fn chrome_flag(chrome: &mut Chrome, args: &[String], at: &mut usize) -> Chromed {
    match args[*at].as_str() {
        // Off, not on: the legend is drawn by default and this is the only spelling that stops it.
        "--no-legend" => chrome.legend = false,
        "--fullscreen" | "--alt" => chrome.fullscreen = true,
        "--padding-x" => match take(args, at).parse::<usize>() {
            Ok(n) => chrome.padding_x = n,
            Err(_) => {
                eprintln!("{}ui: --padding-x wants a number", origin_now());
                return Chromed::Bad(2);
            }
        },
        "--padding-y" => match take(args, at).parse::<usize>() {
            Ok(n) => chrome.padding_y = n,
            Err(_) => {
                eprintln!("{}ui: --padding-y wants a number", origin_now());
                return Chromed::Bad(2);
            }
        },
        "--legend-gap" => match take(args, at).parse::<usize>() {
            Ok(n) => chrome.legend_gap = n,
            Err(_) => {
                eprintln!("{}ui: --legend-gap wants a number", origin_now());
                return Chromed::Bad(2);
            }
        },
        "--border" => match Border::parse(&take(args, at)) {
            Some(border) => chrome.border = border,
            None => {
                eprintln!(
                    "{}ui: border is none, rounded, square, double or thick",
                    origin_now()
                );
                return Chromed::Bad(2);
            }
        },
        "--border-fg" => match theme::Color::parse(&take(args, at)) {
            Some(colour) => chrome.border_style = theme::Style::fg(colour),
            None => {
                eprintln!("{}ui: --border-fg wants a colour", origin_now());
                return Chromed::Bad(2);
            }
        },
        "--border-fit" | "--fit" => match Fit::parse(&take(args, at)) {
            Some(fit) => chrome.fit = fit,
            None => {
                eprintln!("{}ui: fit is content or full", origin_now());
                return Chromed::Bad(2);
            }
        },
        flag @ ("--align" | "--align-x" | "--align-y") => match Place::parse(&take(args, at)) {
            Some(place) => {
                if flag != "--align-y" {
                    chrome.align_x = place;
                }
                if flag != "--align-x" {
                    chrome.align_y = place;
                }
            }
            None => {
                eprintln!(
                    "{}ui: alignment is start/left/top, center or end/right/bottom",
                    origin_now()
                );
                return Chromed::Bad(2);
            }
        },
        _ => return Chromed::NotMine,
    }
    Chromed::Took
}
