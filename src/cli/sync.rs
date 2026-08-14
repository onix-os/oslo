//! `oslo sync user@host` — one command, everything this machine keeps.
//!
//! # Why this is not `oslo profile sync`
//!
//! A profile is a *history*: which commands this shell remembers. Macros are deliberately not part
//! of one — `oslo_base::macros` says why, and it is right — and a secret store is not either. So a
//! command that synced all three under the name `profile` would be lying about what a profile is.
//!
//! What the profile still provides is the **pairing**: its key is what says two machines are the
//! same person's, and every part refuses to move until both ends prove they hold it. One key, one
//! decision, everything else follows.
//!
//! # What travels, and what each part is
//!
//! ```text
//! history   per profile, event-sourced      merged event by event
//! macros    one per machine                 merged name by name
//! secrets   one store per name              merged name by name, never decrypted
//! ```
//!
//! Three storages that share nothing but [`oslo::track::stamp`] — the rule that decides which copy
//! of a record wins. That is the only thing they must agree about, and it is written once.
//!
//! # Deleting works in every one of them
//!
//! A removal is a tombstone in all three, so deleting a command, an alias or a secret on this
//! machine removes it on the other, and the machine that lost it does not hand it back.

use crate::cli::help::Paint;
use help::MENU;

pub(crate) mod help;
pub(crate) mod part;
mod ssh;

pub fn run(args: &[String]) -> i32 {
    if let Some(page) = MENU.asked(args, Paint::detect()) {
        print!("{page}");
        return 0;
    }
    match args.first().map(String::as_str) {
        None => MENU.missing("needs a USER@HOST to sync with"),
        // The far end's halves. Plumbing rather than something to type, and in the help because a
        // command that exists and is undocumented is worse than one that does not.
        Some("send") => part::send(&args[1..]),
        Some("receive") => part::receive(&args[1..]),
        Some(_) => here(args),
    }
}

/// `oslo sync USER@HOST [--only WHAT] [--dry-run]`.
fn here(args: &[String]) -> i32 {
    let mut remote = None;
    let mut wanted: Vec<part::Part> = Vec::new();
    let mut dry_run = false;
    let mut waiting_for_only = false;

    for argument in args {
        let word = argument.as_str();
        if waiting_for_only {
            waiting_for_only = false;
            match part::named(word) {
                Some(part) => wanted.push(part),
                None => return MENU.wrong("--only", &format!("{word:?}: no such part")),
            }
            continue;
        }
        match word {
            "--dry-run" | "-n" => dry_run = true,
            "--only" => waiting_for_only = true,
            flag if flag.starts_with("--only=") => match part::named(&flag[7..]) {
                Some(part) => wanted.push(part),
                None => return MENU.wrong("--only", &format!("{:?}: no such part", &flag[7..])),
            },
            flag if flag.starts_with('-') => {
                return MENU.wrong("sync", &format!("{flag:?}: no such option"));
            }
            word if remote.is_none() => remote = Some(word.to_string()),
            word => return MENU.wrong("sync", &format!("{word:?}: one machine at a time")),
        }
    }
    if waiting_for_only {
        return MENU.wrong("--only", "needs one of: history, macros, secrets");
    }
    let Some(remote) = remote else {
        return MENU.missing("needs a USER@HOST to sync with");
    };
    // Nothing named means everything, which is what somebody who typed `oslo sync` meant.
    if wanted.is_empty() {
        wanted = part::every();
    }

    // A sync that moved nothing is a sync that worked, so the status says nothing about how much
    // travelled — a login file running this must not see a failure because there was no news.
    match part::all_of(&remote, &wanted, dry_run) {
        Ok(()) => 0,
        Err(e) => fail(&e),
    }
}

pub(crate) fn fail(message: &str) -> i32 {
    eprintln!("oslo sync: {message}");
    1
}
