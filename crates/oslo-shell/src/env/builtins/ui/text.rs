//! The `ui` widgets that **print** rather than ask: `write`, `format`, `join`, `log`, `heading`
//! and `title`.
//!
//! Split from `ui.rs` for length, along the seam the module already had: `lists` holds the three
//! that offer a list to choose from, `chrome` and `look` hold what surrounds one, and these are the
//! ones with no question in them at all. What they share is that the answer goes to *stdout* and
//! there is no `Answer` to report — see the note on that split in `super`.

use super::{Chromed, chrome_flag, from_stdin, report, take};
use crate::env::origin_now;
use oslo_ui::ask::{Align, As, Entry, Level, Write, format, horizontal, line, vertical, write};
use oslo_ui::theme;

pub(super) fn run_write(args: &[String]) -> i32 {
    let mut spec = Write::default();
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--header" => spec.header = take(args, &mut at),
            "--placeholder" => spec.placeholder = take(args, &mut at),
            "--value" | "--default" => spec.default = Some(take(args, &mut at)),
            other => {
                // The shared options first — see `chrome_flag`.
                match chrome_flag(&mut spec.chrome, args, &mut at) {
                    Chromed::Took => {
                        at += 1;
                        continue;
                    }
                    Chromed::Bad(status) => return status,
                    Chromed::NotMine => {}
                }
                crate::env::complain(
                    args,
                    other,
                    &format!("ui write: {other}: unknown option"),
                    "not an option here",
                    Some("`ui help` lists every widget and the options each one takes"),
                );
                return 2;
            }
        }
        at += 1;
    }
    report(write(&spec).map(|text| vec![text]))
}

pub(super) fn run_format(args: &[String]) -> i32 {
    let mut kind = As::Markdown;
    let mut values = Vec::new();
    let mut words = Vec::new();
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--type" | "-t" => match As::parse(&take(args, &mut at)) {
                Some(parsed) => kind = parsed,
                None => {
                    eprintln!(
                        "{}ui format: not a type; try markdown, template, code, text",
                        origin_now()
                    );
                    return 2;
                }
            },
            "--field" => {
                let pair = take(args, &mut at);
                match pair.split_once('=') {
                    Some((key, value)) => values.push((key.to_string(), value.to_string())),
                    None => {
                        eprintln!("{}ui format: --field wants key=value", origin_now());
                        return 2;
                    }
                }
            }
            other if other.starts_with("--") => {
                crate::env::complain(
                    args,
                    other,
                    &format!("ui format: {other}: unknown option"),
                    "not an option here",
                    Some("`ui help` lists every widget and the options each one takes"),
                );
                return 2;
            }
            other => words.push(other.to_string()),
        }
        at += 1;
    }
    let text = if words.is_empty() {
        from_stdin().join("\n")
    } else {
        words.join(" ")
    };
    println!("{}", format(&text, kind, &values));
    0
}

pub(super) fn run_join(args: &[String]) -> i32 {
    let mut align = Align::Start;
    let mut side_by_side = true;
    let mut blocks = Vec::new();
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--horizontal" => side_by_side = true,
            "--vertical" => side_by_side = false,
            "--align" => match Align::parse(&take(args, &mut at)) {
                Some(parsed) => align = parsed,
                None => {
                    eprintln!(
                        "{}ui join: not an alignment; try top, center, bottom",
                        origin_now()
                    );
                    return 2;
                }
            },
            other if other.starts_with("--") => {
                crate::env::complain(
                    args,
                    other,
                    &format!("ui join: {other}: unknown option"),
                    "not an option here",
                    Some("`ui help` lists every widget and the options each one takes"),
                );
                return 2;
            }
            other => blocks.push(other.to_string()),
        }
        at += 1;
    }
    println!(
        "{}",
        if side_by_side {
            horizontal(&blocks, align)
        } else {
            vertical(&blocks, align)
        }
    );
    0
}

pub(super) fn run_log(args: &[String]) -> i32 {
    let mut entry = Entry {
        level: Level::Info,
        message: String::new(),
        time: None,
        fields: Vec::new(),
    };
    let mut words = Vec::new();
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--level" | "-l" => match Level::parse(&take(args, &mut at)) {
                Some(level) => entry.level = level,
                None => {
                    eprintln!(
                        "{}ui log: not a level; try debug, info, warn, error, fatal",
                        origin_now()
                    );
                    return 2;
                }
            },
            "--time" => entry.time = Some(take(args, &mut at)),
            "--field" => {
                let pair = take(args, &mut at);
                match pair.split_once('=') {
                    Some((key, value)) => entry.fields.push((key.to_string(), value.to_string())),
                    None => {
                        eprintln!("{}ui log: --field wants key=value", origin_now());
                        return 2;
                    }
                }
            }
            other if other.starts_with("--") => {
                crate::env::complain(
                    args,
                    other,
                    &format!("ui log: {other}: unknown option"),
                    "not an option here",
                    Some("`ui help` lists every widget and the options each one takes"),
                );
                return 2;
            }
            other => words.push(other.to_string()),
        }
        at += 1;
    }
    entry.message = words.join(" ");
    // stderr: a log line is not the script's output. `x=$(cmd)` must not capture it.
    eprintln!("{}", line(&entry));
    // `fatal` ends the script, which is the only thing distinguishing it from `error`.
    i32::from(entry.level == Level::Fatal)
}

/// `ui title` and `ui subtitle` — a heading, and the quieter line under it.
///
/// The same two shapes `oslo.ui.title` and `oslo.ui.subtitle` answer with, so a `.make.lua` recipe
/// and a bash script that calls `oslo userin title` head their output identically. Without them
/// each caller invents a bold line and a rule of its own width, and output from one shell reads as
/// output from three programs.
pub(super) fn run_heading(args: &[String], titled: bool) -> i32 {
    let mut style = theme::Style {
        bold: titled,
        dim: !titled,
        ..theme::Style::default()
    };
    let mut ruled = titled;
    let mut width: Option<usize> = None;
    let mut words = Vec::new();
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--fg" => {
                if let Some(colour) = theme::Color::parse(&take(args, &mut at)) {
                    style.fg = Some(colour);
                    // A colour asked for replaces the dimming, or a subtitle would be a dim green
                    // rather than a green one.
                    style.dim = false;
                }
            }
            "--no-rule" => ruled = false,
            "--width" => width = take(args, &mut at).parse().ok(),
            other if other.starts_with("--") => {
                crate::env::complain(
                    args,
                    other,
                    &format!("ui: {other}: unknown option"),
                    "not an option here",
                    Some("`ui help` lists every widget and the options each one takes"),
                );
                return 2;
            }
            other => words.push(other.to_string()),
        }
        at += 1;
    }
    let text = words.join(" ");
    println!("{}", oslo_ui::ink::ink(&text).styled(style));
    if ruled {
        // In cells, and never wider than the terminal — see `api::ui::heading`.
        let cells = width
            .unwrap_or_else(|| oslo_ui::dropdown::width::display_width(&text))
            .min(oslo_ui::dropdown::width::terminal_cols())
            .max(1);
        println!("{}", oslo_ui::ink::ink("─".repeat(cells)).dim());
    }
    0
}
