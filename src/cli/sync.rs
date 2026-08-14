//! What `oslo profile sync` does: everything this machine keeps, carried to another.
//!
//! # There is no `oslo sync`
//!
//! There was, briefly, on the argument that macros and secrets are not part of a profile — which is
//! true about the *word* and bought nothing. What it produced was two commands doing one job under
//! two names, which is one more name than the job has. **`oslo profile sync` is the only spelling**;
//! this module is where the work lives, not a command of its own.
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
//! The profile provides the **pairing**: its key is what says two machines are the same person's,
//! and every part refuses to move until both ends prove they hold it.
//!
//! # Deleting works in every one of them
//!
//! A removal is a tombstone in all three, so deleting a command, an alias or a secret on this
//! machine removes it on the other, and the machine that lost it does not hand it back.

use crate::cli::profile::help::MENU;

pub(crate) mod part;
mod ssh;

/// `oslo profile sync USER@HOST [NAME] [--only WHAT] [--dry-run]`.
pub(crate) fn here(args: &[String]) -> i32 {
    let mut remote = None;
    let mut profile = None;
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
            // A second word is the profile, which is what `oslo profile sync HOST NAME` means and
            // what this accepts so that the two spellings take the same words.
            word if profile.is_none() => profile = Some(word.to_string()),
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
    let profile = profile.unwrap_or_else(oslo::track::profile::current);

    // A sync that moved nothing is a sync that worked, so the status says nothing about how much
    // travelled — a login file running this must not see a failure because there was no news.
    match part::named_profile(&remote, &wanted, &profile, dry_run) {
        Ok(()) => 0,
        Err(e) => fail(&e),
    }
}

pub(crate) fn fail(message: &str) -> i32 {
    eprintln!("oslo profile sync: {message}");
    1
}
