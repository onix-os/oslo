//! `oslo profile sync user@host` — another name for [`crate::cli::sync`], where somebody looking for
//! it will find it.
//!
//! # It carries everything, and used not to
//!
//! This synced the history alone for a while, argued from the fact that macros and secrets are not
//! part of a profile. That is true about the word and useless to the person typing it: the command
//! moved a third of the machine, printed one line about history, and said nothing about the two
//! parts it had left behind — so they were discovered missing on the other machine, later.
//!
//! A name is not worth that. Both spellings now carry all three, and `NAME` still decides only which
//! *history* travels, because macros and secrets are one per machine either way.
//!
//! The merge itself is not here. Two implementations of it would be two chances to disagree about
//! which copy of a record wins, so this parses the words and hands them to `oslo sync`.

use super::help::MENU;

/// **The same words and the same work as `oslo sync`**, down to `--only` and `--dry-run`.
///
/// Handed over rather than re-parsed: a second parser would be a second set of flags to keep in
/// step, and the last time these two differed the difference was invisible until data failed to
/// arrive on another machine.
pub fn run(args: &[String]) -> i32 {
    if args.is_empty() {
        return MENU.wrong("sync", "needs a USER@HOST to sync with");
    }
    crate::cli::sync::here(args)
}
