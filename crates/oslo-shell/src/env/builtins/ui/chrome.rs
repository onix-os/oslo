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
/// Refuse a flag's *value*, with the caret under the value rather than the flag.
///
/// The flag is spelled right; the word after it is not. Pointing at the flag would mark the one
/// part of the line that is correct.
/// `ui` goes back on the front, because every `run_*` here is handed its args already stripped of
/// the two words that named it — so a line rebuilt from them alone would head the report
/// `--padding-x` and read as though the flag were the command.
fn bad_value(args: &[String], value: &str, body: &str, label: &str) -> Chromed {
    let mut words = vec!["ui".to_string()];
    words.extend(args.iter().cloned());
    crate::env::complain(&words, value, body, label, None);
    Chromed::Bad(2)
}

pub(super) fn chrome_flag(chrome: &mut Chrome, args: &[String], at: &mut usize) -> Chromed {
    match args[*at].as_str() {
        // Off, not on: the legend is drawn by default and this is the only spelling that stops it.
        "--no-legend" => chrome.legend = false,
        "--fullscreen" | "--alt" => chrome.fullscreen = true,
        "--padding-x" => {
            let value = take(args, at);
            match value.parse::<usize>() {
                Ok(n) => chrome.padding_x = n,
                Err(_) => {
                    return bad_value(
                        args,
                        &value,
                        "ui: --padding-x wants a number",
                        "not a number",
                    );
                }
            }
        }
        "--padding-y" => {
            let value = take(args, at);
            match value.parse::<usize>() {
                Ok(n) => chrome.padding_y = n,
                Err(_) => {
                    return bad_value(
                        args,
                        &value,
                        "ui: --padding-y wants a number",
                        "not a number",
                    );
                }
            }
        }
        "--legend-gap" => {
            let value = take(args, at);
            match value.parse::<usize>() {
                Ok(n) => chrome.legend_gap = n,
                Err(_) => {
                    return bad_value(
                        args,
                        &value,
                        "ui: --legend-gap wants a number",
                        "not a number",
                    );
                }
            }
        }
        "--border" => {
            let value = take(args, at);
            match Border::parse(&value) {
                Some(border) => chrome.border = border,
                None => {
                    return bad_value(
                        args,
                        &value,
                        "ui: border is none, rounded, square, double or thick",
                        "not one of the five",
                    );
                }
            }
        }
        "--border-fg" => {
            let value = take(args, at);
            match theme::Color::parse(&value) {
                Some(colour) => chrome.border_style = theme::Style::fg(colour),
                None => {
                    return bad_value(
                        args,
                        &value,
                        "ui: --border-fg wants a colour",
                        "not a colour",
                    );
                }
            }
        }
        "--border-fit" | "--fit" => {
            let value = take(args, at);
            match Fit::parse(&value) {
                Some(fit) => chrome.fit = fit,
                None => {
                    return bad_value(
                        args,
                        &value,
                        "ui: fit is content or full",
                        "content or full",
                    );
                }
            }
        }
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
