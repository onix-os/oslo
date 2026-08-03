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
//! [`crate::interactive::ask`] is where the widgets live; this is the command line onto them.
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
use crate::error::Result;
use crate::interactive::ask::{
    Answer, Border, Choice, Confirm, Input, Styling, choose, confirm, filter, input, style,
};
use crate::interactive::matching::Fuzzy;
use crate::interactive::theme;
use std::io::BufRead;

pub fn builtin_ui(env: &mut Environment, args: &[String]) -> Result<i32> {
    let _ = env;
    let Some(sub) = args.get(1) else {
        usage();
        return Ok(2);
    };
    let rest = &args[2..];
    Ok(match sub.as_str() {
        "input" | "write" => run_input(rest),
        "confirm" => run_confirm(rest),
        "choose" => run_choose(rest, false),
        "filter" => run_choose(rest, true),
        "style" => run_style(rest),
        "help" | "--help" | "-h" => {
            usage();
            0
        }
        other => {
            eprintln!("oslo: ui: {other}: not a widget");
            usage();
            2
        }
    })
}

fn usage() {
    eprintln!(
        "usage: ui input|confirm|choose|filter|style [options] [arguments]\n\
         \n\
         \x20 input   [--placeholder T] [--prompt T] [--value T] [--password] [--required]\n\
         \x20 confirm [--yes T] [--no T] [--default] [question]\n\
         \x20 choose  [--header T] [--multi] [--height N] [items…]\n\
         \x20 filter  [--header T] [--multi] [--height N] [items…]\n\
         \x20 style   [--border B] [--fg C] [--bg C] [--bold] [--padding \"Y X\"] [text…]\n\
         \n\
         The answer goes to stdout. Cancelling is status 1; no terminal is status 2.\n\
         Items come from stdin when none are given."
    );
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
                eprintln!("oslo: ui input: {other}: unknown option");
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
                eprintln!("oslo: ui confirm: {other}: unknown option");
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

fn run_choose(args: &[String], filtering: bool) -> i32 {
    let mut spec = Choice {
        filter: filtering,
        fuzzy: crate::interactive::settings::current().completion.fuzzy,
        ..Choice::default()
    };
    let mut items = Vec::new();
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--header" => spec.header = take(args, &mut at),
            "--multi" | "--no-limit" => spec.multi = true,
            "--height" => spec.height = take(args, &mut at).parse().unwrap_or(spec.height).max(1),
            "--exact" => spec.fuzzy = Fuzzy::Off,
            other if other.starts_with("--") => {
                eprintln!("oslo: ui: {other}: unknown option");
                return 2;
            }
            other => items.push(other.to_string()),
        }
        at += 1;
    }
    // Operands win; stdin is the fallback, so `ls | ui choose` and `ui choose a b` both work.
    spec.items = if items.is_empty() {
        from_stdin()
    } else {
        items
    };
    report(if filtering {
        filter(&spec)
    } else {
        choose(&spec)
    })
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
            "--bold" => spec.style.bold = true,
            "--width" => spec.width = take(args, &mut at).parse().ok(),
            // `--padding "1 2"` is gum's spelling: rows then columns, as CSS does it.
            "--padding" => {
                let value = take(args, &mut at);
                let mut parts = value.split_whitespace();
                spec.padding_y = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
                spec.padding_x = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            }
            other if other.starts_with("--") => {
                eprintln!("oslo: ui style: {other}: unknown option");
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
fn take(args: &[String], at: &mut usize) -> String {
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

impl<T> Answer<T> {
    /// Map the value, keeping the status. Lets `input`'s single string be reported by the same
    /// code that reports `choose`'s list.
    fn map<U>(self, f: impl FnOnce(T) -> U) -> Answer<U> {
        match self {
            Answer::Given(value) => Answer::Given(f(value)),
            Answer::Cancelled => Answer::Cancelled,
            Answer::NoTerminal => Answer::NoTerminal,
        }
    }
}

#[cfg(test)]
#[path = "ui/tests.rs"]
mod tests;
