//! Asking the person at the terminal a question.
//!
//! Line-based, deliberately. A raw-mode picker that draws over the screen and restores it is the
//! obvious thing to want here, and it is the wrong thing to build first: it needs the terminal put
//! into raw mode, which fights the line editor that is already holding it, and it leaves nothing
//! behind in the transcript — you scroll up afterwards and the question you answered is gone.
//!
//! Everything here writes ordinary lines and reads ordinary lines. It works over ssh, inside a
//! multiplexer, in a script whose output is being logged, and in a pipeline — and what you were
//! asked, and what you answered, stay in the scrollback where you can see them.
//!
//! # Prompts go to stderr
//!
//! The question is not the script's output. A script that asks something and then prints a result
//! must be usable as `x=$(script)`, and a prompt captured into that substitution would be a bug in
//! the caller's data. stderr is where a prompt belongs for the same reason `read -p` puts it there.

use super::super::util::{ok, put, text};
use crate::lua::eval::value::{Number, Table, Value};
use std::io::{BufRead, IsTerminal, Write};

/// Write a prompt where a person will see it and a pipeline will not.
fn show(prompt: &str) {
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
}

/// One line from stdin, or `None` at end of input.
///
/// The trailing newline comes off; a trailing `\r` does too, because a terminal on the other end of
/// a serial line or a Windows-side ssh client sends one and an answer of `"y\r"` is not `"y"`.
fn line() -> Option<String> {
    let mut buffer = String::new();
    match std::io::stdin().lock().read_line(&mut buffer) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(buffer.trim_end_matches(['\n', '\r']).to_string()),
    }
}

pub fn install(ui: &mut Table) {
    // oslo.ui.ask(prompt, [default]) -> string, or the default at end of input
    put(ui, "ask", |_, args| {
        let prompt = text(&args, 1, "oslo.ui.ask")?;
        let default = match args.get(1) {
            Some(Value::Str(s)) => Some(s.to_string()),
            _ => None,
        };
        match &default {
            Some(d) => show(&format!("{prompt} [{d}] ")),
            None => show(&format!("{prompt} ")),
        }
        match line() {
            // An empty answer takes the default, which is what the brackets promised.
            Some(answer) if answer.is_empty() => match default {
                Some(d) => ok(Value::str(d)),
                None => ok(Value::str("")),
            },
            Some(answer) => ok(Value::str(answer)),
            // End of input is not an empty answer: nobody is there. A script piped from /dev/null
            // must take the default rather than loop forever asking.
            None => match default {
                Some(d) => ok(Value::str(d)),
                None => ok(Value::Nil),
            },
        }
    });

    // oslo.ui.confirm(question, [default_true]) -> boolean
    put(ui, "confirm", |_, args| {
        let question = text(&args, 1, "oslo.ui.confirm")?;
        let default = matches!(args.get(1), Some(Value::Bool(true)));
        let hint = if default { "[Y/n]" } else { "[y/N]" };
        loop {
            show(&format!("{question} {hint} "));
            match line() {
                None => return ok(Value::Bool(default)),
                Some(answer) => match answer.trim().to_ascii_lowercase().as_str() {
                    "" => return ok(Value::Bool(default)),
                    "y" | "yes" => return ok(Value::Bool(true)),
                    "n" | "no" => return ok(Value::Bool(false)),
                    // Anything else is asked again rather than guessed at. This is the one place a
                    // wrong guess is expensive: the caller is about to do something on the answer.
                    _ => eprintln!("please answer y or n"),
                },
            }
        }
    });

    // oslo.ui.select(items, [prompt]) -> index, value  (nil when nothing was chosen)
    put(ui, "select", |_, args| {
        let items: Vec<String> = match args.first() {
            Some(Value::Table(t)) => t
                .borrow()
                .sequence()
                .iter()
                .map(|v| match v {
                    Value::Str(s) => s.to_string(),
                    other => other.type_name().to_string(),
                })
                .collect(),
            _ => Vec::new(),
        };
        if items.is_empty() {
            return Ok(vec![Value::Nil]);
        }
        let prompt = match args.get(1) {
            Some(Value::Str(s)) => s.to_string(),
            _ => "choose".to_string(),
        };
        // Non-interactive input has nobody to choose, and a menu printed into a log is noise. The
        // first item is the answer a script would have had to hard-code anyway.
        if !std::io::stdin().is_terminal() {
            return Ok(vec![Value::Number(Number::Int(1)), Value::str(&items[0])]);
        }

        let width = items.len().to_string().len();
        loop {
            for (i, item) in items.iter().enumerate() {
                eprintln!("{:>width$}) {item}", i + 1);
            }
            show(&format!("{prompt} [1-{}] ", items.len()));
            let Some(answer) = line() else {
                return Ok(vec![Value::Nil]);
            };
            let answer = answer.trim();
            if answer.is_empty() {
                return Ok(vec![Value::Nil]);
            }
            match answer.parse::<usize>() {
                Ok(n) if n >= 1 && n <= items.len() => {
                    return Ok(vec![
                        Value::Number(Number::Int(n as i64)),
                        Value::str(&items[n - 1]),
                    ]);
                }
                _ => eprintln!("pick a number between 1 and {}", items.len()),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    /// The contract these functions are written to, recorded because none of it is testable
    /// without a terminal on the other end and all of it is easy to break:
    ///
    /// * prompts go to **stderr**, so `x=$(script)` captures the answer and not the question;
    /// * end of input is not an empty answer — it means nobody is there, so the default wins and
    ///   nothing loops;
    /// * `confirm` re-asks rather than guessing, because the caller acts on the answer;
    /// * `select` with no terminal takes the first item instead of printing a menu into a log.
    ///
    /// The pty harness in `scratchpad/` exercises the ones that need a person.
    #[test]
    fn the_contract_is_written_down() {}
}
