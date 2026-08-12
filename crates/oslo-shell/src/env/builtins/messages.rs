//! `messages` — what this session said, after it has scrolled away.
//!
//! ```text
//! messages                 everything, oldest last
//! messages -n 10           the last ten
//! messages plugin          only lines from a source whose name contains `plugin`
//! messages --errors        only the ones that failed
//! messages --clear         forget them
//! ```
//!
//! # Why a builtin and not `oslo messages`
//!
//! The plan called it a tool. A tool is a *new process*, and the buffer is this process's memory —
//! `oslo messages` would faithfully report that a shell which has just started has said nothing.
//! Only the session that produced them can be asked, which is what makes this a builtin, the same
//! way `:messages` is a command inside neovim rather than a flag to it.
//!
//! See `oslo_base::messages` for why the buffer is in memory rather than a file.

use crate::env::Environment;
use oslo_base::error::Result;
use oslo_base::messages::{self, Level, Message};

pub fn builtin_messages(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let mut want: Option<usize> = None;
    let mut only: Option<Level> = None;
    let mut matching: Option<&str> = None;
    let mut rest = args[1..].iter();

    while let Some(word) = rest.next() {
        match word.as_str() {
            "--help" | "-h" => {
                usage();
                return Ok(0);
            }
            "--clear" => {
                messages::clear();
                return Ok(0);
            }
            "--errors" => only = Some(Level::Error),
            "--warnings" => only = Some(Level::Warn),
            "-n" => match rest.next().and_then(|n| n.parse().ok()) {
                Some(n) => want = Some(n),
                None => {
                    eprintln!("messages: -n wants a number");
                    return Ok(2);
                }
            },
            // A bare word is a source, so the common case is `messages plugin` rather than a flag
            // nobody remembers. It matches on *contains* because sources are paths as often as
            // names, and nobody wants to type the whole of one.
            other if !other.starts_with('-') => matching = Some(other),
            other => {
                eprintln!("messages: {other}: unknown option");
                return Ok(2);
            }
        }
    }

    let said: Vec<Message> = messages::all()
        .into_iter()
        .filter(|line| only.is_none_or(|level| line.level == level))
        .filter(|line| matching.is_none_or(|what| line.source.contains(what)))
        .collect();

    // The tail, because the newest is what a question is usually about. `-n` with nothing to show is
    // not an error: an empty session said nothing, which is the answer.
    let from = want.map_or(0, |n| said.len().saturating_sub(n));
    for line in &said[from..] {
        // The count only appears when there is one, so the ordinary line is not made wider by a
        // `(x1)` that says nothing.
        let again = match line.times {
            0 | 1 => String::new(),
            n => format!("  (x{n})"),
        };
        println!(
            "{:>7.3}s  {:<5}  {}: {}{again}",
            line.at,
            line.level.word(),
            line.source,
            line.text
        );
    }
    // Nothing said is success, not failure: a script asking `messages --errors` wants to know
    // whether there were any, and the *count* is what answers that.
    Ok(0)
}

fn usage() {
    println!(
        "USAGE\n  \
         messages [-n COUNT] [--errors|--warnings] [SOURCE]\n  \
         messages --clear\n\n\
         What this session said — plugins, config files and hooks — oldest first.\n\
         Kept in memory only, so another shell has its own."
    );
}

#[cfg(test)]
#[path = "messages/tests.rs"]
mod tests;
