//! `=command`, the interactive shorthand that answers with a command's path.
//!
//! `ldd =oslo` becomes `ldd /usr/bin/oslo`. It is zsh's, and like `\cmd` and `@name` it is gated on
//! the shell being interactive, because `echo =foo` in a `/bin/sh` script has to print `=foo`.
//!
//! **The case worth a test file is the one that fails.** A name that resolves to nothing used to be
//! left exactly as it was, so `ldd =olso` — one transposed pair — handed `ldd` a word starting with
//! `=` and the answer came back as `ldd: ./=olso: No such file or directory`. That blames a file
//! nobody meant to name, says nothing about the shorthand, and is a long way from "you typed olso".
//!
//! No pty: `-i` sets the option `env.interactive()` reads, which is the whole gate.

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

fn run(script: &str, interactive: bool) -> (String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut command = Command::new(oslo_bin());
    if interactive {
        command.arg("-i");
    }
    let out = command
        .arg("-c")
        .arg(script)
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    (
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned(),
        String::from_utf8_lossy(&out.stderr).trim_end().to_owned(),
    )
}

/// A name that is a command becomes its path.
#[test]
fn a_command_becomes_where_it_lives() {
    let (out, err) = run("echo =sh", true);
    assert!(err.is_empty(), "{err}");
    assert!(out.starts_with('/'), "{out:?}");
    assert!(out.ends_with("/sh"), "{out:?}");
}

/// **A name that is not a command is named, and the command does not run.**
#[test]
fn a_name_that_is_not_a_command_says_so() {
    let (out, err) = run("echo =olso", true);
    assert!(
        err.contains("olso is not a command"),
        "the typo was not reported: {err:?}"
    );
    assert!(
        !out.contains("=olso"),
        "the literal reached the command anyway: {out:?}"
    );
}

/// A script sees none of it — the property that makes reporting safe at a prompt.
#[test]
fn a_script_still_prints_the_word_untouched() {
    for word in ["=sh", "=olso"] {
        let (out, err) = run(&format!("echo {word}"), false);
        assert_eq!(out, word, "a script must print {word} as written");
        assert!(err.is_empty(), "{err}");
    }
}

/// **Quoting makes it a literal**, which is what stops the error being a trap.
///
/// `echo "=ls"` expanded through its quotes before this — the same bug `@name` had and had fixed.
/// Harmless while an unresolved name was passed through silently; not harmless once it fails the
/// command, because a quoted `=whatever` in a script-like line would suddenly stop working.
#[test]
fn quoting_keeps_the_word_literal() {
    for word in ["=sh", "=olso"] {
        for quote in ['"', '\''] {
            let (out, err) = run(&format!("echo {quote}{word}{quote}"), true);
            assert_eq!(out, word, "{quote}{word}{quote} was not left alone");
            assert!(err.is_empty(), "{word}: {err}");
        }
    }
}

/// The shapes that only look like the shorthand are left alone, in both modes.
///
/// `FOO=bar` is the one that matters: an assignment is a word containing `=`, and reading it as the
/// shorthand would break every `VAR=value cmd` ever typed.
#[test]
fn a_word_that_merely_contains_an_equals_is_untouched() {
    for word in ["FOO=bar", "=", "==sh", "=/bin/sh"] {
        for interactive in [true, false] {
            let (out, err) = run(&format!("echo '{word}'"), interactive);
            assert_eq!(out, word, "{word} changed (interactive={interactive})");
            assert!(err.is_empty(), "{word}: {err}");
        }
    }
}
