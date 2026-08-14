//! `oslo profile sync user@host` — two histories, one profile, both ends ending up with the union.
//!
//! # The shape
//!
//! ```text
//! oslo profile fingerprint NAME          ssh host oslo profile fingerprint NAME
//!            └──────────────── must be equal ──────────────────┘
//!
//! ssh host oslo profile send NAME  ─────────────────►  a snapshot of theirs
//!            sync_files(mine, theirs)                  merges *both* files
//! ssh host oslo profile receive NAME  ◄─────────────   the merged copy
//!            (which merges again over there, in case
//!             they recorded something meanwhile)
//! ```
//!
//! **The far end is oslo, not `scp`.** A store is a live database and copying the file under a
//! shell that is writing to it is how you get half a transaction; `send` takes a proper snapshot
//! and `receive` merges rather than replaces, so a command typed over there between the two steps
//! survives instead of being overwritten.
//!
//! # Why the fingerprints have to match
//!
//! `default` here and `default` on a machine you have an account on are two histories that share a
//! word. The key is what says they are one profile, and refusing without it is the difference
//! between syncing and merging a stranger's commands into your own.
//!
//! # When both sides have the same command
//!
//! Nothing here decides that: [`oslo::track::HistoryEvent::preferred`] already does, and it does it
//! the same way on both machines. The higher revision wins; a tie goes to the deleted one; a tie
//! there goes to a random 16-byte tie-breaker *stored in the event itself*. Both ends therefore
//! reach the same answer without asking each other, which is what makes the sync order-independent
//! — run it twice, run it backwards, and the result is the same.
//!
//! A command run on both machines is not a conflict at all, and this is the part worth knowing:
//! every event carries the **host** that ran it, so `cargo build` here and `cargo build` there are
//! two events that both survive. The counts add up rather than one overwriting the other.

use oslo::track::{Track, sync_files};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::fail;

const USAGE: &str = "usage: oslo profile sync USER@HOST [NAME] [--dry-run]";

pub fn run(args: &[String]) -> i32 {
    let mut remote = None;
    let mut name = None;
    let mut dry_run = false;
    for argument in args {
        match argument.as_str() {
            "--dry-run" | "-n" => dry_run = true,
            flag if flag.starts_with('-') => {
                eprintln!("oslo profile sync: {flag:?}: no such option");
                eprintln!("{USAGE}");
                return 2;
            }
            word if remote.is_none() => remote = Some(word.to_string()),
            word => name = Some(word.to_string()),
        }
    }
    let Some(remote) = remote else {
        eprintln!("{USAGE}");
        return 2;
    };
    let name = name.unwrap_or_else(oslo::track::profile::current);

    match two_ways(&remote, &name, dry_run) {
        Ok(()) => 0,
        Err(e) => fail(&e),
    }
}

fn two_ways(remote: &str, name: &str, dry_run: bool) -> Result<(), String> {
    let mine = store_of(name)?;
    if !mine.exists() {
        return Err(format!(
            "{name}: no store at {} — nothing to sync yet",
            mine.display()
        ));
    }

    // **Before anything is copied.** The check is cheap, and a mismatch found after a transfer
    // would mean somebody else's history is already on this disk.
    agree(remote, name)?;

    let theirs = tempfile::Builder::new()
        .prefix("oslo-sync-")
        .suffix(".db")
        .tempfile()
        .map_err(|e| format!("no temporary file: {e}"))?;
    let theirs = theirs.into_temp_path();

    let snapshot = over_ssh(remote, &["profile", "send", name], None)?;
    std::fs::write(&theirs, &snapshot).map_err(|e| format!("{}: {e}", theirs.display()))?;
    println!("fetched  {} bytes from {remote}", snapshot.len());

    // Merges *both* files: mine gains theirs, and the copy gains mine, which is what goes back.
    let report = sync_files(&mine, &theirs, dry_run)?;
    println!(
        "here     +{} ~{} -{}\nthere    +{} ~{} -{}\nunchanged {}",
        report.added_left,
        report.updated_left,
        report.deleted_left,
        report.added_right,
        report.updated_right,
        report.deleted_right,
        report.unchanged,
    );

    if dry_run {
        println!("dry run — nothing was written on either side");
        return Ok(());
    }

    // Merged again over there rather than dropped on top, so anything they recorded while this was
    // running survives.
    let merged = std::fs::read(&theirs).map_err(|e| format!("{}: {e}", theirs.display()))?;
    let said = over_ssh(remote, &["profile", "receive", name], Some(&merged))?;
    print!("{}", String::from_utf8_lossy(&said));
    Ok(())
}

/// Refuse unless both ends hold the same profile key.
fn agree(remote: &str, name: &str) -> Result<(), String> {
    let key = oslo::track::profile::key::read(name)?.ok_or_else(|| {
        format!(
            "{name}: has no key here — `oslo profile key init {name}`, then export it to {remote}"
        )
    })?;
    let here = oslo::track::profile::key::fingerprint(&key);

    let answered = over_ssh(remote, &["profile", "fingerprint", name], None)?;
    let there = String::from_utf8_lossy(&answered).trim().to_string();

    if there.is_empty() {
        return Err(format!(
            "{remote}: said nothing when asked for its fingerprint"
        ));
    }
    if here != there {
        return Err(format!(
            "{name}: this machine is {here} and {remote} is {there} — they are not the same \
             profile.\n  If they should be: `oslo profile export {name} | ssh {remote} oslo \
             profile import {name}`"
        ));
    }
    Ok(())
}

/// Run `oslo …` on the far end, hand it `input`, and answer with what it printed.
///
/// **Standard error is inherited**, so ssh's own questions — a host key to accept, a passphrase —
/// reach the terminal. Captured, the sync would appear to hang on the one prompt that needs a
/// person.
fn over_ssh(remote: &str, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    // `$OSLO_SSH` for anyone whose way in is not a bare `ssh` — a wrapper, an alternate config, a
    // jump host, `mosh`. Split on spaces rather than handed to a shell, so nothing in it is
    // re-interpreted.
    let how = std::env::var("OSLO_SSH").unwrap_or_else(|_| "ssh".to_string());
    let mut words = how.split_whitespace();
    let program = words.next().unwrap_or("ssh");

    let mut command = Command::new(program);
    command
        .args(words)
        .arg(remote)
        .arg(std::env::var("OSLO_SSH_REMOTE_BIN").unwrap_or_else(|_| "oslo".to_string()))
        .args(args)
        .stdin(match input {
            Some(_) => Stdio::piped(),
            None => Stdio::null(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot run {program}: {e}"))?;

    if let Some(bytes) = input {
        let mut stdin = child.stdin.take().ok_or("no standard input to write to")?;
        let written = bytes.to_vec();
        // On a thread, because a far end that answers before it has read everything would
        // otherwise fill the pipe while this waits on it.
        let writer = std::thread::spawn(move || {
            use std::io::Write;
            stdin.write_all(&written)
        });
        let out = child.wait_with_output().map_err(|e| format!("ssh: {e}"))?;
        let _ = writer.join();
        return finished(remote, args, out);
    }

    let out = child.wait_with_output().map_err(|e| format!("ssh: {e}"))?;
    finished(remote, args, out)
}

fn finished(remote: &str, args: &[&str], out: std::process::Output) -> Result<Vec<u8>, String> {
    if out.status.success() {
        return Ok(out.stdout);
    }
    // 127 is the shell over there saying it could not find the command, which is the failure
    // somebody hits first and the one a generic "exited 127" would not explain.
    let why = match out.status.code() {
        Some(127) => format!("{remote}: no `oslo` on its $PATH"),
        Some(code) => format!("{remote}: `oslo {}` exited {code}", args.join(" ")),
        None => format!("{remote}: `oslo {}` was killed", args.join(" ")),
    };
    Err(why)
}

/// Where a named profile keeps its store.
pub(super) fn store_of(name: &str) -> Result<PathBuf, String> {
    if !oslo::track::profile::valid(name) {
        return Err(format!("{name:?} is not a usable profile name"));
    }
    let data = std::env::var("XDG_DATA_HOME").ok();
    let home = std::env::var("HOME").ok();
    oslo::track::profile::profile_dir(data.as_deref(), home.as_deref(), name)
        .map(|dir| dir.join("hist.db"))
        .ok_or_else(|| "no $XDG_DATA_HOME and no $HOME, so there is no store".to_string())
}

/// `oslo profile send NAME` — a consistent snapshot of this profile's store, on standard output.
///
/// A snapshot rather than the file: the store is a live database and a shell may be writing to it
/// this instant.
pub(super) fn send(name: &str) -> i32 {
    let path = match store_of(name) {
        Ok(path) => path,
        Err(e) => return fail(&e),
    };
    // Read-only: `send` must never be the thing that writes to a store somebody else is using.
    let track = match Track::open_existing(&path, true) {
        Ok(track) => track,
        Err(e) => return fail(&e),
    };
    // A directory rather than a temporary *file*: `backup_to` writes the store itself and refuses
    // a destination that already exists, which is exactly what `tempfile()` hands back.
    let held = match tempfile::Builder::new().prefix("oslo-send-").tempdir() {
        Ok(dir) => dir,
        Err(e) => return fail(&format!("no temporary directory: {e}")),
    };
    let snapshot = held.path().join("hist.db");
    if let Err(e) = track.backup_to(&snapshot) {
        return fail(&e);
    }
    match std::fs::read(&snapshot) {
        Ok(bytes) => {
            use std::io::Write;
            let mut out = std::io::stdout();
            if out.write_all(&bytes).is_err() || out.flush().is_err() {
                return fail("cannot write the snapshot");
            }
            0
        }
        Err(e) => fail(&format!("{}: {e}", snapshot.display())),
    }
}

/// `oslo profile receive NAME` — merge a snapshot arriving on standard input into this profile.
///
/// **Merged, never dropped on top.** Between the moment the other end asked for a copy and the
/// moment it hands one back, a shell here may have recorded something; replacing the store would
/// lose it.
pub(super) fn receive(name: &str) -> i32 {
    use std::io::Read;
    let mine = match store_of(name) {
        Ok(path) => path,
        Err(e) => return fail(&e),
    };
    let mut incoming = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut incoming) {
        return fail(&format!("cannot read standard input: {e}"));
    }
    if incoming.is_empty() {
        return fail("nothing arrived on standard input");
    }

    let theirs = match tempfile::Builder::new()
        .prefix("oslo-receive-")
        .suffix(".db")
        .tempfile()
    {
        Ok(file) => file.into_temp_path(),
        Err(e) => return fail(&format!("no temporary file: {e}")),
    };
    if let Err(e) = std::fs::write(&theirs, &incoming) {
        return fail(&format!("{}: {e}", theirs.display()));
    }
    match sync_files(&mine, &theirs, false) {
        Ok(report) => {
            println!(
                "there    +{} ~{} -{}",
                report.added_left, report.updated_left, report.deleted_left
            );
            0
        }
        Err(e) => fail(&e),
    }
}
