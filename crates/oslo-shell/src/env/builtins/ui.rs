//! `ui` — the input widgets, for shell scripts.
//!
//! ```sh
//! name=$(ui input --placeholder "your name") || exit
//! ui confirm "delete $target?" && rm -rf "$target"
//! branch=$(git branch --format='%(refname:short)' | ui filter --header "check out")
//! ui style --border rounded --padding "1 2" "done"
//! ```
//!
//! A builtin rather than a program on `$PATH`, because it needs the terminal the shell already
//! owns and because a shell that ships its own prompts should not need one installed beside it.
//! [`oslo_ui::ask`] is where the widgets live; this is the command line onto them.
//!
//! # The three rules a script depends on
//!
//! * **the answer is stdout, everything else is stderr** — so `$(ui input)` captures the answer
//!   and nothing else;
//! * **cancelling is status 1 with no output** — so `x=$(ui input) || exit` is correct, where a
//!   widget returning "" on Esc would make cancelled and empty the same thing;
//! * **no terminal is status 2** — distinct from cancelled, so a script can tell "there was nobody
//!   to ask" from "they said no".
//!
//! Items come from the operands, or from stdin when there are none — `ls | ui choose` and
//! `ui choose a b c` are both the obvious thing.

use crate::env::Environment;
use crate::env::origin_now;
use oslo_base::error::Result;
mod chrome;
mod lists;
mod look;
use chrome::{Chromed, chrome_flag};
use lists::{from_stdin_raw, run_choose, run_file, run_table};
use oslo_ui::ask::{
    Align, Answer, As, Border, Confirm, Entry, Input, Level, Pager, Spin, Styling, Write, confirm,
    format, horizontal, input, line, pager, spin, style, vertical, write,
};
use oslo_ui::theme;
use std::io::BufRead;

pub fn builtin_ui(env: &mut Environment, args: &[String]) -> Result<i32> {
    let _ = env;
    Ok(run("ui", args.get(1..).unwrap_or_default()))
}

/// `oslo userin …`, for every shell that is not oslo.
///
/// **A builtin cannot be reached from bash.** A script, a `sh -c`, a Makefile recipe — all of them
/// reach a *program*, and the widgets are the part of oslo most worth having from outside: a shell
/// that ships its own prompts should be able to lend them out. One body, two doors, so the two can
/// never disagree about what a widget does.
pub fn tool(args: &[String]) -> i32 {
    run("oslo userin", args)
}

/// The widget and its options, whichever door they came through.
fn run(called: &str, args: &[String]) -> i32 {
    let Some(sub) = args.first() else {
        usage(called, Where::Error);
        return 2;
    };
    let rest = &args[1..];
    match sub.as_str() {
        "input" => run_input(rest),
        "write" => run_write(rest),
        "confirm" => run_confirm(rest),
        "choose" => run_choose(rest, false),
        "filter" => run_choose(rest, true),
        "style" => run_style(rest),
        "title" => run_heading(rest, true),
        "subtitle" => run_heading(rest, false),
        "file" => run_file(rest),
        "format" => run_format(rest),
        "join" => run_join(rest),
        "log" => run_log(rest),
        "pager" => run_pager(rest),
        "spin" => run_spin(rest),
        "table" => run_table(rest),
        "help" | "--help" | "-h" => {
            // **Asked for, so it is output.** A `--help` on stderr cannot be paged or grepped,
            // which is the first thing anyone does with a list of thirteen widgets.
            usage(called, Where::Output);
            0
        }
        other => {
            eprintln!("{}{called}: {other}: not a widget", origin_now());
            usage(called, Where::Error);
            2
        }
    }
}

/// Where the usage goes: what was asked for is output, what went wrong is not.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Where {
    Output,
    Error,
}

fn usage(called: &str, out: Where) {
    let text = format!(
        "usage: {called} WIDGET [options] [arguments]\n\
         \n\
         ask for something\n\
         \x20 input   [--placeholder T] [--prompt T] [--value T] [--password] [--required]\n\
         \x20 write   [--header T] [--placeholder T] [--value T]\n\
         \x20 confirm [--yes T] [--no T] [--default] [question]\n\
         \x20 choose  [--header T] [--multi] [--height N] [items…]\n\
         \x20 filter  [--header T] [--multi] [--height N] [items…]\n\
         \x20 table   [--separator C] [--header-row] [--height N]\n\
         \x20 file    [--all] [--directory] [--height N] [path]\n\
         \n\
         show something\n\
         \x20 style   [--border B] [--fg C] [--bg C] [--bold] [--padding \"Y X\"] [text…]\n\
         \x20 format  [--type markdown|template|code|text] [--field K=V] [text…]\n\
         \x20 join    [--horizontal|--vertical] [--align A] [blocks…]\n\
         \x20 pager   [--title T] [--wrap] [text…]\n\
         \x20 log     [--level L] [--time T] [--field K=V] message…\n\
         \x20 spin    [--title T] [--quiet] -- command [args…]\n\
         \n\
         The answer goes to stdout. Cancelling is status 1; no terminal is status 2.\n\
         Items come from stdin when none are given."
    );
    match out {
        Where::Output => println!("{text}"),
        Where::Error => eprintln!("{text}"),
    }
}

/// Report an answer the way a script reads it: the value on stdout, the status as the status.
fn report(answer: Answer<Vec<String>>) -> i32 {
    match answer {
        Answer::Given(values) => {
            for value in values {
                println!("{value}");
            }
            0
        }
        other => other.status(),
    }
}

/// A flag the widget itself did not claim, offered to the chrome and then to the look.
///
/// One helper rather than the same two `match`es in each of five option loops — which is how
/// `ui file` came to accept `--border` and not `--stripe`.
pub(super) enum Shared {
    Took,
    NotMine,
    Bad(i32),
}

pub(super) fn shared_flag(
    chrome: &mut oslo_ui::ask::chrome::Chrome,
    look: &mut oslo_ui::ask::look::Look,
    args: &[String],
    at: &mut usize,
) -> Shared {
    match chrome_flag(chrome, args, at) {
        Chromed::Took => return Shared::Took,
        Chromed::Bad(status) => return Shared::Bad(status),
        Chromed::NotMine => {}
    }
    match look::look_flag(look, args, at) {
        look::Looked::Took => Shared::Took,
        look::Looked::Bad(status) => Shared::Bad(status),
        look::Looked::NotMine => Shared::NotMine,
    }
}

fn run_input(args: &[String]) -> i32 {
    let mut spec = Input::default();
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--placeholder" => spec.placeholder = take(args, &mut at),
            "--prompt" => spec.prompt = take(args, &mut at),
            "--value" | "--default" => spec.default = Some(take(args, &mut at)),
            "--password" => spec.password = true,
            "--required" => spec.required = true,
            other => {
                // The widget's own options are matched first, so `input`'s `--prompt` keeps its
                // meaning and the look's identically named field is never reached from here.
                match shared_flag(&mut spec.chrome, &mut spec.look, args, &mut at) {
                    Shared::Took => {
                        at += 1;
                        continue;
                    }
                    Shared::Bad(status) => return status,
                    Shared::NotMine => {}
                }
                eprintln!("{}ui input: {other}: unknown option", origin_now());
                return 2;
            }
        }
        at += 1;
    }
    report(input(&spec).map(|line| vec![line]))
}

fn run_confirm(args: &[String]) -> i32 {
    let mut spec = Confirm::default();
    let mut at = 0;
    let mut question = None;
    while at < args.len() {
        match args[at].as_str() {
            "--yes" => spec.yes = take(args, &mut at),
            "--no" => spec.no = take(args, &mut at),
            "--default" => spec.default = true,
            other if other.starts_with("--") => {
                // The shared options first — see `chrome_flag`.
                match chrome_flag(&mut spec.chrome, args, &mut at) {
                    Chromed::Took => {
                        at += 1;
                        continue;
                    }
                    Chromed::Bad(status) => return status,
                    Chromed::NotMine => {}
                }
                eprintln!("{}ui confirm: {other}: unknown option", origin_now());
                return 2;
            }
            other => question = Some(other.to_string()),
        }
        at += 1;
    }
    if let Some(question) = question {
        spec.question = question;
    }
    match confirm(&spec) {
        // The answer *is* the status: `ui confirm && …` is the whole point, and printing `yes` for
        // the caller to compare against would be a worse interface wearing the same clothes.
        Answer::Given(true) => 0,
        Answer::Given(false) => 1,
        other => other.status(),
    }
}

fn run_style(args: &[String]) -> i32 {
    let mut spec = Styling::default();
    let mut words = Vec::new();
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--border" => match Border::parse(&take(args, &mut at)) {
                Some(border) => spec.border = border,
                None => {
                    eprintln!(
                        "oslo: ui style: not a border; try none, rounded, square, double, thick"
                    );
                    return 2;
                }
            },
            "--fg" => spec.style.fg = theme::Color::parse(&take(args, &mut at)),
            "--bg" => spec.style.bg = theme::Color::parse(&take(args, &mut at)),
            "--border-fg" => spec.border_style.fg = theme::Color::parse(&take(args, &mut at)),
            // Every attribute the style carries, so `ui style` and `oslo.ui.style` accept the same
            // set — `--bold` alone was the odd one out.
            "--bold" => spec.style.bold = true,
            "--dim" => spec.style.dim = true,
            "--italic" => spec.style.italic = true,
            "--underline" => spec.style.underline = true,
            "--reverse" => spec.style.reverse = true,
            "--blink" => spec.style.blink = true,
            "--hidden" => spec.style.hidden = true,
            "--strike" => spec.style.strike = true,
            "--width" => spec.width = take(args, &mut at).parse().ok(),
            // `--padding "1 2"` is gum's spelling: rows then columns, as CSS does it.
            "--padding" => {
                let value = take(args, &mut at);
                let mut parts = value.split_whitespace();
                spec.padding_y = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
                spec.padding_x = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            }
            other if other.starts_with("--") => {
                eprintln!("{}ui style: {other}: unknown option", origin_now());
                return 2;
            }
            other => words.push(other.to_string()),
        }
        at += 1;
    }
    spec.text = if words.is_empty() {
        from_stdin().join("\n")
    } else {
        words.join(" ")
    };
    // stdout, not stderr: this is the script's own output, not a prompt.
    println!("{}", style(&spec));
    0
}

/// The option's argument, leaving `at` on it so the caller's `+= 1` lands past it.
pub(super) fn take(args: &[String], at: &mut usize) -> String {
    *at += 1;
    args.get(*at).cloned().unwrap_or_default()
}

/// Lines from stdin, for `… | ui choose`.
///
/// **Only when stdin is a pipe.** With stdin still attached to the terminal there is nothing
/// coming and reading would block for ever on input the person cannot give — they are trying to
/// answer a question that has not been drawn yet. The first version of this did exactly that and
/// hung the test suite; `ui choose` with no operands and no pipe now has no items, which the
/// widget reports as a cancel.
fn from_stdin() -> Vec<String> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return Vec::new();
    }
    std::io::stdin()
        .lock()
        .lines()
        .map_while(std::result::Result::ok)
        .filter(|line| !line.is_empty())
        .collect()
}

#[cfg(test)]
#[path = "ui/tests.rs"]
mod tests;

fn run_write(args: &[String]) -> i32 {
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
                eprintln!("{}ui write: {other}: unknown option", origin_now());
                return 2;
            }
        }
        at += 1;
    }
    report(write(&spec).map(|text| vec![text]))
}

fn run_format(args: &[String]) -> i32 {
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
                eprintln!("{}ui format: {other}: unknown option", origin_now());
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

fn run_join(args: &[String]) -> i32 {
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
                eprintln!("{}ui join: {other}: unknown option", origin_now());
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

fn run_log(args: &[String]) -> i32 {
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
                eprintln!("{}ui log: {other}: unknown option", origin_now());
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

fn run_pager(args: &[String]) -> i32 {
    let mut spec = Pager::default();
    let mut words = Vec::new();
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--title" => spec.title = take(args, &mut at),
            "--wrap" => spec.wrap = true,
            other if other.starts_with("--") => {
                // The shared options first — see `chrome_flag`.
                match chrome_flag(&mut spec.chrome, args, &mut at) {
                    Chromed::Took => {
                        at += 1;
                        continue;
                    }
                    Chromed::Bad(status) => return status,
                    Chromed::NotMine => {}
                }
                eprintln!("{}ui pager: {other}: unknown option", origin_now());
                return 2;
            }
            other => words.push(other.to_string()),
        }
        at += 1;
    }
    spec.text = if words.is_empty() {
        from_stdin_raw()
    } else {
        words.join(" ")
    };
    match pager(&spec) {
        Answer::Given(()) => 0,
        other => other.status(),
    }
}

fn run_spin(args: &[String]) -> i32 {
    let mut spec = Spin {
        title: "working".to_string(),
        command: Vec::new(),
        quiet: false,
    };
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--title" => spec.title = take(args, &mut at),
            "--quiet" => spec.quiet = true,
            // Everything after `--` is the command, so its own options cannot be read as ours.
            "--" => {
                spec.command = args[at + 1..].to_vec();
                break;
            }
            other if other.starts_with("--") => {
                eprintln!("{}ui spin: {other}: unknown option", origin_now());
                return 2;
            }
            _ => {
                spec.command = args[at..].to_vec();
                break;
            }
        }
        at += 1;
    }
    spin(&spec)
}

/// `ui title` and `ui subtitle` — a heading, and the quieter line under it.
///
/// The same two shapes `oslo.ui.title` and `oslo.ui.subtitle` answer with, so a `.make.lua` recipe
/// and a bash script that calls `oslo userin title` head their output identically. Without them
/// each caller invents a bold line and a rule of its own width, and output from one shell reads as
/// output from three programs.
fn run_heading(args: &[String], titled: bool) -> i32 {
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
                eprintln!("{}ui: {other}: unknown option", origin_now());
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
