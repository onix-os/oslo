//! The options that say how a list is drawn: where the filter sits, what is beside it, and what
//! colour the rows take.
//!
//! The companion to [`super::chrome`], and the same reason for existing: one parser, so
//! `ui choose --stripe` and `ui filter --stripe` cannot come to mean different things. The Lua
//! side reads the same names from a table and both build the same [`Look`].

use super::take;
use crate::env::origin_now;
use oslo_ui::ask::look::{Look, Preset, Where, Width};
use oslo_ui::scanner::Scanner;
use oslo_ui::theme::{Color, Style};

/// What happened when a widget's option loop offered a flag to the shared look parser.
pub(super) enum Looked {
    Took,
    NotMine,
    Bad(i32),
}

/// One flag of the shared list styling. See the module note.
pub(super) fn look_flag(look: &mut Look, args: &[String], at: &mut usize) -> Looked {
    match args[*at].as_str() {
        // The preset first, so `--look history --stripe 0` is "that look, with this changed".
        // Applying it as a whole rather than field by field is what makes the combination hold
        // together — see `Preset`.
        "--look" | "--preset" => match Preset::parse(&take(args, at)) {
            Some(preset) => *look = preset.look(),
            None => return bad("look is plain, history or menu"),
        },
        "--filter-at" => match Where::parse(&take(args, at)) {
            Some(place) => look.filter_at = place,
            None => return bad("--filter-at is top or bottom"),
        },
        "--reverse" => look.reverse = true,
        "--slot-left" => look.left = take(args, at),
        "--slot-right" => look.right = take(args, at),
        "--prompt" => look.prompt = take(args, at),
        "--placeholder" => look.placeholder = take(args, at),
        "--marker" => look.marker = take(args, at),
        "--list-width" => match Width::parse(&take(args, at)) {
            Some(width) => look.width = width,
            None => return bad("--list-width is content or full"),
        },
        "--surface-rows" => match number(args, at) {
            Some(n) => look.surface_rows = n.max(1),
            None => return bad("--surface-rows wants a number"),
        },
        "--list-gap" => match number(args, at) {
            Some(n) => look.gap = n,
            None => return bad("--list-gap wants a number"),
        },
        "--list-pad" => match number(args, at) {
            Some(n) => look.pad = n,
            None => return bad("--list-pad wants a number"),
        },
        // A colour on its own is a background here, because that is what these four are for: the
        // surface under the query and the three row states. Foregrounds have their own flags.
        "--surface" => match colour(args, at) {
            Some(c) => look.surface = Some(c),
            None => return bad("--surface wants a colour"),
        },
        "--stripe" => match colour(args, at) {
            Some(c) => {
                look.stripe = Some(Style {
                    bg: Some(c),
                    ..Style::default()
                })
            }
            None => return bad("--stripe wants a colour"),
        },
        "--sel-bg" => match colour(args, at) {
            Some(c) => look.selected.bg = Some(c),
            None => return bad("--sel-bg wants a colour"),
        },
        "--sel-fg" => match colour(args, at) {
            Some(c) => look.selected.fg = Some(c),
            None => return bad("--sel-fg wants a colour"),
        },
        "--row-fg" => match colour(args, at) {
            Some(c) => look.row.fg = Some(c),
            None => return bad("--row-fg wants a colour"),
        },
        "--row-bg" => match colour(args, at) {
            Some(c) => look.row.bg = Some(c),
            None => return bad("--row-bg wants a colour"),
        },
        "--accent" => match colour(args, at) {
            Some(c) => look.accent = Style::fg(c),
            None => return bad("--accent wants a colour"),
        },
        "--hit-fg" => match colour(args, at) {
            Some(c) => look.hit.fg = Some(c),
            None => return bad("--hit-fg wants a colour"),
        },
        "--hit-bg" => match colour(args, at) {
            Some(c) => look.hit.bg = Some(c),
            None => return bad("--hit-bg wants a colour"),
        },
        // The sweep at the head of the filter row. A width rather than a bare switch, because the
        // one thing anyone changes about it is how far the head travels.
        "--scanner" => {
            look.scanner = Some(Scanner {
                width: 9,
                ..Scanner::default()
            })
        }
        "--scanner-width" => match number(args, at) {
            Some(n) => {
                look.scanner = Some(Scanner {
                    width: n.clamp(2, 32) as u8,
                    ..look.scanner.unwrap_or_default()
                })
            }
            None => return bad("--scanner-width wants a number"),
        },
        "--no-scanner" => look.scanner = None,
        "--badge" => look.badge = take(args, at),
        "--badge-fg" => match colour(args, at) {
            Some(c) => look.badge_style.fg = Some(c),
            None => return bad("--badge-fg wants a colour"),
        },
        "--badge-bg" => match colour(args, at) {
            Some(c) => look.badge_style.bg = Some(c),
            None => return bad("--badge-bg wants a colour"),
        },
        "--meta-columns" => match number(args, at) {
            Some(n) => look.meta_columns = n,
            None => return bad("--meta-columns wants a number"),
        },
        "--meta-fg" => match colour(args, at) {
            Some(c) => look.meta_style = Style::fg(c),
            None => return bad("--meta-fg wants a colour"),
        },
        _ => return Looked::NotMine,
    }
    Looked::Took
}

fn bad(why: &str) -> Looked {
    eprintln!("{}ui: {why}", origin_now());
    Looked::Bad(2)
}

fn number(args: &[String], at: &mut usize) -> Option<usize> {
    take(args, at).parse::<usize>().ok()
}

fn colour(args: &[String], at: &mut usize) -> Option<Color> {
    Color::parse(&take(args, at))
}
