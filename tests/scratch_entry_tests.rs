//! Going into a named scratch, from somewhere that cannot go anywhere.
//!
//! # What this exists to catch
//!
//! `scratch work` is documented as something "a script or an alias can say". From a script there is
//! no terminal, and a scratch **is** a terminal you go into — so the answer has to be no. It was:
//! the command reported a failure and exited 1.
//!
//! What it did on the way there is the defect. The session was built first — the keeper forked, a
//! shell exec'd inside it — and only then did the attach discover there was no terminal to attach
//! to. The failure left that shell running, with nobody attached and no way to attach: one
//! unreachable process per invocation, for the life of the machine, from a command that said it
//! had failed.

#![cfg(feature = "scratch")]

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

/// The refusal comes first, and nothing is built for a session that could never be entered.
#[test]
fn a_scratch_without_a_terminal_leaves_nothing_behind() {
    let home = tempfile::tempdir().expect("tempdir");
    let name = "oslo-test-no-tty";

    let out = Command::new(oslo_bin())
        .args(["-c", &format!("scratch {name}")])
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", home.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn oslo");

    let said = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{said}");
    assert!(
        said.contains("terminal"),
        "the refusal does not say why: {said}"
    );

    // The runtime directory is named for the uid. `id -u` rather than a crate: this test binary
    // links nothing that would answer it, and one process is cheaper than a dependency.
    let uid =
        String::from_utf8_lossy(&Command::new("id").arg("-u").output().expect("id -u").stdout)
            .trim()
            .to_string();
    let runtime = std::path::PathBuf::from(format!("/tmp/oslo-{uid}/scratch"));
    for suffix in ["meta", "log", "lock"] {
        let left = runtime.join(format!("{name}.{suffix}"));
        assert!(
            !left.exists(),
            "{} was left behind by a scratch that was refused",
            left.display()
        );
    }
}

/// Listing needs no terminal, and still answers.
///
/// The check is about *going into* one. A status question from a prompt segment or a script is the
/// case the tool exists for, and refusing that too would have been the wrong cure.
#[test]
fn listing_scratches_works_without_a_terminal() {
    let home = tempfile::tempdir().expect("tempdir");
    let out = Command::new(oslo_bin())
        .args(["-c", "scratch -l"])
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", home.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
