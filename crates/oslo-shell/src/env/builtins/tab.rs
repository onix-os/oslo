//! `tab` — the same door the key opens, for the things a key cannot do.
//!
//! The key is the interface. This exists because two of its answers are needed by something that is
//! not a person at a prompt:
//!
//! * **A prompt segment counting how many are running.** That is a subprocess reading lines, not a
//!   finder, and `tab -l` is the shape every multiplexer already answers to.
//! * **A name you already know.** `tab work` is fewer keystrokes than the key and a search, and a
//!   script or an alias can say it.
//!
//! Everything it does, the key does too. Nothing here is a second way of *deciding* anything.

use super::super::Environment;
use crate::tab::enter;
use oslo_base::error::Result;

/// A screenful of a tab's output, replayed so an attach lands on the screen it left.
const REPLAY: u64 = 8192;

pub fn builtin_tab(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let key = oslo_ui::settings::current().tab.key.clone();
    // **`args[0]` is the name the builtin was called by**, as it is for every builtin here. Reading
    // it as an operand made `tab -l` mean "go into a tab called tab", which created one and then
    // failed on a terminal it did not have.
    match args.get(1).map(String::as_str) {
        // The finder, exactly as the key opens it.
        None => Ok(open(&key)),
        Some("-l" | "--list" | "ls") => Ok(list()),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            Ok(0)
        }
        // **Not a flag, so it is a name.** Refusing anything unrecognised would make `tab -x` and
        // `tab my-tab` fail the same way, and only one of those is a mistake.
        Some(flag) if flag.starts_with('-') => {
            eprintln!("oslo: tab: {flag}: unknown option\n{USAGE}");
            Ok(2)
        }
        Some(name) => Ok(enter_named(&key, name)),
    }
}

/// Every tab there is, one per line, newest first.
///
/// **Plain lines and nothing else**, because the caller is `| wc -l` as often as it is a person.
fn list() -> i32 {
    match enter::names() {
        Ok(names) => {
            for name in names {
                println!("{name}");
            }
            0
        }
        Err(err) => {
            eprintln!("oslo: tab: {err}");
            1
        }
    }
}

fn open(key: &str) -> i32 {
    report(enter::open(key, REPLAY))
}

fn enter_named(key: &str, name: &str) -> i32 {
    report(enter::open_named(key, REPLAY, name))
}

fn report(outcome: std::io::Result<enter::Went>) -> i32 {
    match outcome {
        Ok(_) => 0,
        Err(err) => {
            eprintln!("oslo: tab: {err}");
            1
        }
    }
}

const USAGE: &str = "usage: tab [name | -l]

  tab          open the finder, the same one the key opens
  tab <name>   go into that tab, making it if it is not running
  tab -l       every tab, one per line";

#[cfg(test)]
mod tests;
