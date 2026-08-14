//! The three things that travel, and the two halves each of them has.
//!
//! # One shape, three storages
//!
//! Every part does the same four steps: ask the far end for a snapshot, merge it against ours,
//! report what moved, hand the merged copy back. What differs is only what a snapshot *is* — a
//! database file for history and macros, a tar-less bundle of sealed files for secrets — and that
//! difference is confined to [`snapshot_of`] and [`absorb`].

use super::ssh::{agree, over_ssh};
use oslo::track::{Track, sync_files};
use std::path::PathBuf;

// Only a build with secrets has a directory of sealed files to carry.
#[cfg(feature = "secrets")]
mod bundle;
mod report;

pub use report::Moved;

/// One of the things a machine keeps.
///
/// **Secrets are here only in a build that has them.** A part that could be named and then refused
/// would be worse than one that is simply absent: `--only secrets` says *no such part* on a build
/// without the feature, and the help lists what that build can actually do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    History,
    Macros,
    #[cfg(feature = "secrets")]
    Secrets,
}

impl Part {
    pub fn word(self) -> &'static str {
        match self {
            Part::History => "history",
            Part::Macros => "macros",
            #[cfg(feature = "secrets")]
            Part::Secrets => "secrets",
        }
    }
}

/// Everything, in the order it is reported.
pub fn every() -> Vec<Part> {
    vec![
        Part::History,
        Part::Macros,
        #[cfg(feature = "secrets")]
        Part::Secrets,
    ]
}

pub fn named(word: &str) -> Option<Part> {
    match word {
        "history" => Some(Part::History),
        "macros" | "macro" => Some(Part::Macros),
        #[cfg(feature = "secrets")]
        "secrets" | "secret" => Some(Part::Secrets),
        _ => None,
    }
}

/// Sync every named part with `remote`.
///
/// **The fingerprints are checked once, for all of them.** The profile key is what says these two
/// machines are the same person's, and a per-part check would ask the same question three times.
pub fn all_of(remote: &str, wanted: &[Part], dry_run: bool) -> Result<(), String> {
    named_profile(remote, wanted, &oslo::track::profile::current(), dry_run)
}

/// The same, for a caller that names the profile rather than taking the one in use.
pub fn named_profile(
    remote: &str,
    wanted: &[Part],
    profile: &str,
    dry_run: bool,
) -> Result<(), String> {
    agree(remote, profile)?;

    for part in wanted {
        one(remote, *part, profile, dry_run)?;
    }
    if dry_run {
        println!("dry run — nothing was written on either side");
    }
    Ok(())
}

/// Ask, merge, report, hand back.
fn one(remote: &str, part: Part, profile: &str, dry_run: bool) -> Result<(), String> {
    let held = tempfile::Builder::new()
        .prefix("oslo-sync-")
        .tempdir()
        .map_err(|e| format!("no temporary directory: {e}"))?;
    let theirs = held.path().join("theirs");

    let snapshot = over_ssh(remote, &["sync", "send", part.word(), profile], None)?;
    absorb(part, &theirs, &snapshot)?;

    let moved = merge_into(part, &theirs, profile, dry_run)?;
    // Said before the far end is written to, so the line a person reads is in the order the work
    // happened rather than after a round trip that may take a while.
    report::say(part, &moved);

    if dry_run {
        return Ok(());
    }
    // Merged again over there rather than dropped on top, so anything recorded on that machine
    // while this was running survives.
    let merged = snapshot_of(part, &theirs)?;
    // What it prints is deliberately nothing — see [`report::say_far`].
    over_ssh(
        remote,
        &["sync", "receive", part.word(), profile],
        Some(&merged),
    )?;
    Ok(())
}

/// Merge the far end's copy into ours, and theirs into it, so both hold the union.
fn merge_into(
    part: Part,
    theirs: &std::path::Path,
    profile: &str,
    dry: bool,
) -> Result<Moved, String> {
    match part {
        Part::History => {
            let mine = history_path(profile)?;
            if !mine.exists() {
                return Err(format!(
                    "{profile}: no history at {} — nothing to sync yet",
                    mine.display()
                ));
            }
            let report = sync_files(&mine, &theirs.join("hist.db"), dry)?;
            Ok(Moved::from_history(&report))
        }
        Part::Macros => {
            let mine = oslo::macros::open()?;
            let other = oslo::macros::Store::open(&theirs.join("macros.db"))
                .ok_or_else(|| "the macros that arrived cannot be opened".to_string())?;
            let report = oslo::macros::sync::merge(&mine, &other, dry)?;
            // The flat file a starting shell reads, or every arriving alias is invisible until the
            // next `oslo macros` command happens to rewrite it.
            if !dry && !report.quiet() {
                let _ = oslo::macros::snapshot::write(&oslo::macros::all(&mine));
            }
            Ok(Moved::from_macros(&report))
        }
        #[cfg(feature = "secrets")]
        Part::Secrets => {
            let mine = oslo::secrets::Store::selected(None)?;
            let other = oslo::secrets::Store {
                name: mine.name.clone(),
                directory: theirs.join("secrets"),
                keys: mine.keys.clone(),
                recipients: mine.recipients.clone(),
                crypto: mine.crypto.clone(),
            };
            let report = oslo::secrets::sync::merge(&mine, &other, dry)?;
            Ok(Moved::from_secrets(&report))
        }
    }
}

/// Where this profile's history lives.
pub fn history_path(name: &str) -> Result<PathBuf, String> {
    if !oslo::track::profile::valid(name) {
        return Err(format!("{name:?} is not a usable profile name"));
    }
    let data = std::env::var("XDG_DATA_HOME").ok();
    let home = std::env::var("HOME").ok();
    oslo::track::profile::profile_dir(data.as_deref(), home.as_deref(), name)
        .map(|dir| dir.join("hist.db"))
        .ok_or_else(|| "no $XDG_DATA_HOME and no $HOME, so there is no store".to_string())
}

/// One part of this machine, as bytes to send.
fn snapshot_of(part: Part, from: &std::path::Path) -> Result<Vec<u8>, String> {
    match part {
        Part::History => std::fs::read(from.join("hist.db"))
            .map_err(|e| format!("{}: {e}", from.join("hist.db").display())),
        Part::Macros => std::fs::read(from.join("macros.db"))
            .map_err(|e| format!("{}: {e}", from.join("macros.db").display())),
        #[cfg(feature = "secrets")]
        Part::Secrets => bundle::pack(&from.join("secrets")),
    }
}

/// The other way: bytes that arrived, laid out where a merge can read them.
fn absorb(part: Part, into: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(into).map_err(|e| format!("{}: {e}", into.display()))?;
    match part {
        Part::History => write_at(&into.join("hist.db"), bytes),
        Part::Macros => write_at(&into.join("macros.db"), bytes),
        #[cfg(feature = "secrets")]
        Part::Secrets => bundle::unpack(&into.join("secrets"), bytes),
    }
}

fn write_at(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// `oslo sync send WHAT PROFILE` — this machine's copy, on standard output.
pub fn send(args: &[String]) -> i32 {
    let Some(part) = args.first().and_then(|word| named(word)) else {
        return super::fail("send needs one of: history, macros, secrets");
    };
    let profile = args
        .get(1)
        .cloned()
        .unwrap_or_else(oslo::track::profile::current);
    match mine(part, &profile).and_then(|bytes| write_out(&bytes)) {
        Ok(()) => 0,
        Err(e) => super::fail(&e),
    }
}

/// A consistent copy of one part of this machine.
///
/// **A snapshot rather than the file.** Both databases are live and a shell may be writing to one
/// this instant; copying the bytes would give the far end a file that half-opens.
fn mine(part: Part, profile: &str) -> Result<Vec<u8>, String> {
    match part {
        Part::History => {
            let path = history_path(profile)?;
            let Some(held) = scratch(&path)? else {
                return Ok(Vec::new());
            };
            Track::open_existing(&path, true)?.backup_to(&held.path())?;
            read_back(held.path())
        }
        Part::Macros => {
            let path = oslo::macros::database().ok_or("nowhere to read macros from")?;
            let Some(held) = scratch(&path)? else {
                return Ok(Vec::new());
            };
            oslo::macros::Store::open(&path)
                .ok_or_else(|| format!("{}: cannot be opened", path.display()))?
                .backup_to(&held.path())?;
            read_back(held.path())
        }
        #[cfg(feature = "secrets")]
        Part::Secrets => bundle::pack(&oslo::secrets::Store::selected(None)?.directory),
    }
}

/// Somewhere for a database to write its own consistent copy.
///
/// `None` when there is nothing to copy, which is not an error: a machine with no macros yet has
/// none to send, and the far end merging an empty snapshot is exactly right.
///
/// A directory rather than a temporary *file*, because `backup_to` writes the database itself and
/// refuses a destination that already exists — which is precisely what `tempfile()` hands back.
fn scratch(source: &std::path::Path) -> Result<Option<Copy>, String> {
    if !source.exists() {
        return Ok(None);
    }
    let held = tempfile::Builder::new()
        .prefix("oslo-send-")
        .tempdir()
        .map_err(|e| format!("no temporary directory: {e}"))?;
    Ok(Some(Copy { held }))
}

/// A temporary directory and the file inside it a copy goes to.
struct Copy {
    held: tempfile::TempDir,
}

impl Copy {
    fn path(&self) -> PathBuf {
        self.held.path().join("copy.db")
    }
}

fn read_back(path: PathBuf) -> Result<Vec<u8>, String> {
    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_out(bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut out = std::io::stdout();
    if out.write_all(bytes).is_err() || out.flush().is_err() {
        return Err("cannot write the snapshot".to_string());
    }
    Ok(())
}

/// `oslo sync receive WHAT PROFILE` — merge what arrives on standard input.
///
/// **Merged, never dropped on top.** Between the moment the other end asked for a copy and the
/// moment it hands one back, something here may have changed.
pub fn receive(args: &[String]) -> i32 {
    use std::io::Read;
    let Some(part) = args.first().and_then(|word| named(word)) else {
        return super::fail("receive needs one of: history, macros, secrets");
    };
    let profile = args
        .get(1)
        .cloned()
        .unwrap_or_else(oslo::track::profile::current);

    let mut incoming = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut incoming) {
        return super::fail(&format!("cannot read standard input: {e}"));
    }
    if incoming.is_empty() {
        // An empty snapshot is a machine with nothing of this kind, which is not a failure.
        return 0;
    }

    let held = match tempfile::Builder::new().prefix("oslo-receive-").tempdir() {
        Ok(dir) => dir,
        Err(e) => return super::fail(&format!("no temporary directory: {e}")),
    };
    let theirs = held.path().join("theirs");
    if let Err(e) = absorb(part, &theirs, &incoming) {
        return super::fail(&e);
    }
    match merge_into(part, &theirs, &profile, false) {
        Ok(moved) => {
            report::say_far(part, &moved);
            0
        }
        Err(e) => super::fail(&e),
    }
}
