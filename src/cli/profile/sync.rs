//! `oslo profile sync user@host` — the history half of [`crate::cli::sync`], under the name of the
//! thing it syncs.
//!
//! # Why this is a wrapper and not an implementation
//!
//! A profile *is* the history, so syncing one is a real thing to want and this is where somebody
//! will look for it. But there is nothing here that `oslo sync --only history` does not do, and two
//! implementations of a merge are two chances to disagree about who wins — so this parses the words
//! and hands over.
//!
//! `oslo sync` is the one to reach for: it carries macros and secrets as well, over the same ssh
//! and behind the same profile-key check.

use crate::cli::sync::part::{self, Part};

use super::help::MENU;

pub fn run(args: &[String]) -> i32 {
    let mut remote = None;
    let mut name = None;
    let mut dry_run = false;
    for argument in args {
        match argument.as_str() {
            "--dry-run" | "-n" => dry_run = true,
            flag if flag.starts_with('-') => {
                return MENU.wrong("sync", &format!("{flag:?}: no such option"));
            }
            word if remote.is_none() => remote = Some(word.to_string()),
            word => name = Some(word.to_string()),
        }
    }
    let Some(remote) = remote else {
        return MENU.wrong("sync", "needs a USER@HOST to sync with");
    };
    let name = name.unwrap_or_else(oslo::track::profile::current);

    match part::named_profile(&remote, &[Part::History], &name, dry_run) {
        Ok(()) => 0,
        Err(e) => super::fail(&e),
    }
}
