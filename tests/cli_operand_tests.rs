//! A word an `oslo <tool>` subcommand cannot read is refused, not ignored.
//!
//! # What this exists to catch
//!
//! `oslo config files EXTRA` printed the file list and exited 0. So did `oslo profile list EXTRA`,
//! `oslo hook list EXTRA`, `oslo plugin list EXTRA`, `oslo secret list EXTRA`, `oslo direnv status
//! A B` and `oslo macros publish EXTRA` — seven tools answering as though the extra word had not
//! been typed. The dangerous shape is `oslo secret rm old new`, which forgot `old`, said nothing
//! about `new`, and looked exactly like success.
//!
//! It is the same silent acceptance `printf -Z`, `trap -z EXIT` and `ls | length extra` all had,
//! and there is nothing to notice when it regresses: the output is the ordinary output.
//!
//! # Read-only only
//!
//! Every case here either fails before doing anything or asks a question. `XDG_CONFIG_HOME` and
//! `XDG_DATA_HOME` point at a temporary directory regardless, so a case that did reach the store
//! would reach an empty one rather than the person's own.

mod common;

use common::oslo_bin;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Run `oslo <words…>` against an empty config and data directory.
fn oslo(home: &Path, words: &[&str]) -> Output {
    Command::new(oslo_bin())
        .args(words)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo")
}

fn sandbox() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("config/oslo")).expect("config dir");
    std::fs::create_dir_all(dir.path().join("data")).expect("data dir");
    dir
}

/// **The bug.** Each of these reads a fixed number of operands and ignored whatever came after.
#[test]
fn an_operand_a_subcommand_cannot_read_is_refused() {
    let dir = sandbox();
    let cases: &[&[&str]] = &[
        &["config", "files", "EXTRA_ZZ"],
        &["config", "timing", "EXTRA_ZZ"],
        &["config", "which", "vi.enabled", "EXTRA_ZZ"],
        &["profile", "list", "EXTRA_ZZ"],
        &["profile", "show", "default", "EXTRA_ZZ"],
        &["profile", "fingerprint", "default", "EXTRA_ZZ"],
        &["hook", "list", "EXTRA_ZZ"],
        &["hook", "show", "pre-cmd", "EXTRA_ZZ"],
        &["macros", "publish", "EXTRA_ZZ"],
    ];
    for words in cases {
        let out = oslo(dir.path(), words);
        assert!(
            !out.status.success(),
            "`oslo {}` was accepted: {}",
            words.join(" "),
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .unwrap_or("")
        );
    }
}

/// The same rule where it matters most: a second name on a command that destroys one.
#[cfg(feature = "secrets")]
#[test]
fn a_second_secret_name_is_refused_rather_than_dropped() {
    let dir = sandbox();
    for words in [
        ["secret", "rm", "one-zz", "two-zz"],
        ["secret", "get", "one-zz", "two-zz"],
    ] {
        let out = oslo(dir.path(), &words);
        assert!(
            !out.status.success(),
            "`oslo {}` was accepted",
            words.join(" ")
        );
    }
    let listed = oslo(dir.path(), &["secret", "list", "EXTRA_ZZ"]);
    assert!(!listed.status.success(), "`secret list EXTRA_ZZ` accepted");
}

/// `direnv` reads one path at most, so a second was a directory nobody acted on.
#[cfg(feature = "direnv")]
#[test]
fn a_direnv_subcommand_refuses_a_second_path() {
    let dir = sandbox();
    for words in [
        vec!["direnv", "status", "/tmp", "EXTRA_ZZ"],
        vec!["direnv", "prune", "EXTRA_ZZ"],
    ] {
        let out = oslo(dir.path(), &words);
        assert!(
            !out.status.success(),
            "`oslo {}` was accepted",
            words.join(" ")
        );
    }
    // And the legitimate shapes still answer.
    for words in [vec!["direnv", "status"], vec!["direnv", "status", "/tmp"]] {
        let out = oslo(dir.path(), &words);
        assert!(
            out.status.success(),
            "`oslo {}` was refused: {}",
            words.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[cfg(feature = "plugin")]
#[test]
fn a_plugin_subcommand_refuses_an_operand_it_cannot_read() {
    let dir = sandbox();
    let out = oslo(dir.path(), &["plugin", "list", "EXTRA_ZZ"]);
    assert!(!out.status.success(), "`plugin list EXTRA_ZZ` was accepted");
    // `doctor` names one plugin at most.
    let out = oslo(dir.path(), &["plugin", "doctor", "a-zz", "b-zz"]);
    assert!(!out.status.success(), "`plugin doctor a b` was accepted");
    // The listing itself still answers.
    let out = oslo(dir.path(), &["plugin", "list"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// **The half that must not break.** Every one of these is a legitimate call, and a check that
/// counted operands wrongly would refuse them — which is the way this fix could do more harm than
/// the bug did.
#[test]
fn the_calls_that_were_always_right_still_work() {
    let dir = sandbox();
    let cases: &[&[&str]] = &[
        &["config", "files"],
        &["config", "timing"],
        &["profile", "list"],
        &["profile"],
        &["profile", "show"],
        &["profile", "show", "default"],
        &["hook", "list"],
        &["hook", "list", "--attached"],
        &["hook", "show", "pre-cmd"],
        // `test` takes a name and any number of `field=value` pairs.
        &["hook", "test", "pre-prompt", "a=1", "b=2"],
        &["macros", "publish"],
        &["history", "path"],
    ];
    for words in cases {
        let out = oslo(dir.path(), words);
        assert!(
            out.status.success(),
            "`oslo {}` was refused: {}",
            words.join(" "),
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("")
        );
    }
}

/// An option no subcommand has is named as an option, not counted as an operand — the wording is
/// what sends somebody to check the spelling rather than the argument count.
#[test]
fn an_unknown_option_is_called_an_option() {
    let dir = sandbox();
    let out = oslo(dir.path(), &["config", "files", "--nosuchopt-zz"]);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("unknown option"), "{said}");
}
