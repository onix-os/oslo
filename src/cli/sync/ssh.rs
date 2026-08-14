//! Running oslo on the other machine.
//!
//! **The far end is oslo, not `scp`.** Every store here is a live database or a directory a shell
//! may be writing to this instant; copying the bytes out from under it is how you get half a
//! transaction. So each part asks the far end for a snapshot, merges locally, and hands the merged
//! copy back to be merged again over there — which is also what makes a command typed on the other
//! machine mid-sync survive rather than be overwritten.

use std::process::{Command, Stdio};

/// Run `oslo …` on the far end, hand it `input`, and answer with what it printed.
///
/// **Standard error is inherited**, so ssh's own questions — a host key to accept, a passphrase, a
/// hardware key asking to be touched — reach the terminal. Captured, a sync would appear to hang on
/// the one prompt that needs a person.
pub fn over_ssh(remote: &str, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
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
        // On a thread, because a far end that answers before it has read everything would otherwise
        // fill the pipe while this waits on it.
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

/// Refuse unless both ends hold the same profile key.
///
/// **Before anything is copied.** The check is cheap, and a mismatch found after a transfer would
/// mean somebody else's data is already on this disk. The key never crosses the wire — only its
/// fingerprint, which is a hash and gives nothing away.
pub fn agree(remote: &str, name: &str) -> Result<(), String> {
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
