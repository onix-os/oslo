//! `scratch` — the same door the key opens, for the things a key cannot do.
//!
//! The key is the interface. This exists because two of its answers are needed by something that is
//! not a person at a prompt:
//!
//! * **A prompt segment counting how many are running.** That is a subprocess reading lines, not a
//!   finder, and `scratch -l` is the shape every multiplexer already answers to.
//! * **A name you already know.** `scratch work` is fewer keystrokes than the key and a search, and a
//!   script or an alias can say it.
//!
//! Everything it does, the key does too. Nothing here is a second way of *deciding* anything.

use super::super::Environment;
use crate::env::origin_now;
use crate::scratch::enter;
use oslo_base::error::Result;

/// A screenful of a scratch's output, replayed so an attach lands on the screen it left.
const REPLAY: u64 = 8192;

pub fn builtin_scratch(_env: &mut Environment, args: &[String]) -> Result<i32> {
    // **`args[0]` is the name the builtin was called by**, as it is for every builtin here. Reading
    // it as an operand made `scratch -l` mean "go into a scratch called scratch", which created one and then
    // failed on a terminal it did not have.
    Ok(run(args.get(1..).unwrap_or_default()))
}

/// `oslo scratch …`, for everything that is not an oslo prompt.
///
/// **A builtin cannot answer a prompt segment.** A status bar, a `io.popen`, an `sh -c` — all of
/// them reach a *program*, and inside `/bin/sh` the word `scratch` finds whatever is on `$PATH`, which
/// on a machine with scratch-rs installed is scratch-rs. One body, two doors, so the two can never disagree
/// about which sessions are running.
pub fn tool(args: &[String]) -> i32 {
    run(args)
}

/// The operands, whichever door they came through.
fn run(args: &[String]) -> i32 {
    let key = oslo_ui::settings::current().scratch.key.clone();
    match args.first().map(String::as_str) {
        // The finder, exactly as the key opens it.
        None => open(&key),
        Some("-l" | "--list" | "ls") => list(),
        Some("-k" | "--kill") => kill(args.get(1..).unwrap_or_default()),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            0
        }
        // **Not a flag, so it is a name.** Refusing anything unrecognised would make `scratch -x` and
        // `scratch my-scratch` fail the same way, and only one of those is a mistake.
        Some(flag) if flag.starts_with('-') => {
            crate::env::complain_with_usage(
                args,
                flag,
                &format!("scratch: {flag}: unknown option"),
                "not an option here",
                USAGE,
            );
            2
        }
        Some(name) => enter_named(&key, name),
    }
}

/// Every scratch there is, one per line, newest first.
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
            eprintln!("{}scratch: {err}", origin_now());
            1
        }
    }
}

/// End the named scratches, and say so for each that would not go.
///
/// **Every name is tried**, rather than stopping at the first failure: `scratch -k a b c` is one
/// request to be rid of three things, and leaving two of them running because the first is stuck
/// would be the wrong half done.
fn kill(names: &[String]) -> i32 {
    if names.is_empty() {
        eprintln!("{}scratch: -k takes a name\n{USAGE}", origin_now());
        return 2;
    }
    let mut status = 0;
    for name in names {
        if let Err(err) = enter::kill(name) {
            eprintln!("{}scratch: {err}", origin_now());
            status = 1;
        }
    }
    status
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
            eprintln!("{}scratch: {err}", origin_now());
            1
        }
    }
}

const USAGE: &str = "usage: scratch [name | -l | -k name...]

  scratch          open the finder, the same one the key opens
  scratch <name>   go into that scratch, making it if it is not running
  scratch -l       every scratch, one per line
  scratch -k NAME  end it — the finder's delete key, for a name you know";

#[cfg(test)]
mod tests;
