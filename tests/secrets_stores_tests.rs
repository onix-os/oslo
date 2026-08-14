//! Stores, keys and recipients through the real binary: several of each, and the fences on them.
#![cfg(feature = "secrets")]

mod common;

use common::oslo_bin;
use std::io::Write;
use std::process::{Command, Stdio};

/// Run `oslo secret …` against a store of its own, with `input` on standard input.
fn secret(home: &std::path::Path, args: &[&str], input: &[u8]) -> (String, String, i32) {
    let mut child = Command::new(oslo_bin())
        .arg("secret")
        .args(args)
        .env("XDG_DATA_HOME", home)
        .env("XDG_STATE_HOME", home.join("state"))
        .env_remove("OSLO_SECRET_IDENTITY")
        .env_remove("OSLO_SECRET_STORE")
        .env_remove("OSLO_SECRET_NO_EXEC")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn oslo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input)
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// **The thing a second recipient is for.** A colleague's key is added, everything is rotated to
/// it, and then *their key alone* opens the file — which is the property that makes an encrypted
/// store worth committing rather than a private one worth hiding.
#[test]
fn a_second_recipient_can_read_what_was_rotated_to_them() {
    let home = tempfile::tempdir().expect("tempdir");
    secret(home.path(), &["set", "token"], b"shared-value");

    // Their key, made in a store of its own so it is a different identity.
    let their_key = home.path().join("friend-key");
    let (made, err, status) = secret(
        home.path(),
        &[
            "--store",
            "friend",
            "key",
            "add",
            "file",
            &their_key.to_string_lossy(),
        ],
        b"",
    );
    assert_eq!(status, 0, "{err}{made}");
    let (out, err, status) = secret(home.path(), &["--store", "friend", "key", "init"], b"");
    assert_eq!(status, 0, "{err}");
    let public = out.lines().last().expect("a public half").to_string();
    assert!(public.starts_with("age1"), "{out:?}");

    let (_, err, status) = secret(home.path(), &["recipient", "add", &public], b"");
    assert_eq!(status, 0, "{err}");
    let (out, err, status) = secret(home.path(), &["rotate"], b"");
    assert_eq!(status, 0, "{err}");
    assert!(out.contains("2 recipients"), "{out:?}");

    // Mine still opens it…
    assert_eq!(
        secret(home.path(), &["get", "token"], b"").0,
        "shared-value"
    );

    // …and so does theirs, with nothing of mine in the environment.
    let out = Command::new(oslo_bin())
        .args(["secret", "get", "token"])
        .env("XDG_DATA_HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("OSLO_SECRET_IDENTITY", &their_key)
        .output()
        .expect("spawn oslo");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "shared-value",
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A recipient this binary cannot use is refused when it is written, not when the next `set` fails.
#[test]
fn a_recipient_oslo_cannot_use_is_refused_at_the_time() {
    let home = tempfile::tempdir().expect("tempdir");
    secret(home.path(), &["set", "token"], b"v");

    let (_, err, status) = secret(
        home.path(),
        &["recipient", "add", "age1yubikey1qdefinitelynotmine"],
        b"",
    );
    assert_ne!(status, 0, "it was accepted");
    assert!(err.contains("plugin recipient"), "{err:?}");
    assert!(err.contains("key command"), "a way out is named: {err:?}");
}

/// **A key can be a program**, which is what reaches a password manager, a smartcard, or anything
/// else oslo will never compile in — and `$OSLO_SECRET_NO_EXEC` is the switch that says this
/// invocation will not fork, for a cron job or a container to export once.
#[test]
fn a_key_can_come_from_a_program_and_can_be_refused() {
    let home = tempfile::tempdir().expect("tempdir");
    secret(home.path(), &["set", "unrelated"], b"v");
    let identity = home.path().join("state/oslo/identity");

    let giver = home.path().join("give-key");
    std::fs::write(
        &giver,
        format!("#!/bin/sh\ncat {}\n", identity.to_string_lossy()),
    )
    .expect("write");
    std::fs::set_permissions(&giver, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("chmod");

    let (_, err, status) = secret(
        home.path(),
        &[
            "--store",
            "byhand",
            "key",
            "add",
            "command",
            &giver.to_string_lossy(),
        ],
        b"",
    );
    assert_eq!(status, 0, "{err}");

    let (_, err, status) = secret(home.path(), &["--store", "byhand", "set", "thing"], b"kept");
    assert_eq!(status, 0, "{err}");
    assert_eq!(
        secret(home.path(), &["--store", "byhand", "get", "thing"], b"").0,
        "kept"
    );

    // The same read, with the switch on: refused, and it says which one it did not run.
    let out = Command::new(oslo_bin())
        .args(["secret", "--store", "byhand", "get", "thing"])
        .env("XDG_DATA_HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("OSLO_SECRET_NO_EXEC", "1")
        .output()
        .expect("spawn oslo");
    assert!(!out.status.success(), "it ran the program anyway");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("OSLO_SECRET_NO_EXEC"), "{said:?}");
}

/// **A plugin's store never forks**, whichever door the command comes through.
#[test]
fn a_plugin_store_may_not_run_a_key_command() {
    let home = tempfile::tempdir().expect("tempdir");
    let (_, err, status) = secret(
        home.path(),
        &[
            "--store",
            "plugin.notes",
            "key",
            "add",
            "command",
            "/bin/cat",
        ],
        b"",
    );
    assert_ne!(status, 0, "it was accepted");
    assert!(err.contains("may not run a key command"), "{err:?}");

    // And by hand, in the file the command would have written.
    let conf = home.path().join("state/oslo/secrets.conf");
    std::fs::create_dir_all(conf.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&conf, "[plugin.notes]\nkey command /bin/cat\n").expect("write");
    let (_, err, status) = secret(home.path(), &["--store", "plugin.notes", "list"], b"");
    assert_ne!(status, 0, "the hand-written one was accepted");
    assert!(err.contains("may not run a key command"), "{err:?}");
}

/// Which store an invocation means: the argument, then the environment, then the file, then `user`.
#[test]
fn the_store_is_chosen_in_one_order() {
    let home = tempfile::tempdir().expect("tempdir");
    secret(home.path(), &["set", "name"], b"in-user");
    secret(home.path(), &["--store", "work", "set", "name"], b"in-work");

    assert_eq!(secret(home.path(), &["get", "name"], b"").0, "in-user");
    assert_eq!(
        secret(home.path(), &["--store", "work", "get", "name"], b"").0,
        "in-work"
    );

    // The environment, which is what a Makefile or a cron line sets once for everything under it.
    let from_environment = |store: &str| {
        let out = Command::new(oslo_bin())
            .args(["secret", "get", "name"])
            .env("XDG_DATA_HOME", home.path())
            .env("XDG_STATE_HOME", home.path().join("state"))
            .env("OSLO_SECRET_STORE", store)
            .output()
            .expect("spawn oslo");
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    assert_eq!(from_environment("work"), "in-work");

    // And the file, which is what the machine says when nothing else does.
    let conf = home.path().join("state/oslo/secrets.conf");
    let text = std::fs::read_to_string(&conf).unwrap_or_default();
    std::fs::write(&conf, format!("default work\n{text}")).expect("write");
    assert_eq!(secret(home.path(), &["get", "name"], b"").0, "in-work");

    // The argument still wins over both.
    assert_eq!(
        secret(home.path(), &["--store", "user", "get", "name"], b"").0,
        "in-user"
    );
}

/// `run` puts the value in one child and nowhere else — no shell in between to keep a copy.
#[test]
fn run_hands_the_value_to_one_child() {
    let home = tempfile::tempdir().expect("tempdir");
    secret(home.path(), &["set", "gh-token"], b"tok");

    let (out, err, status) = secret(
        home.path(),
        &["run", "TOKEN=gh-token", "--", "sh", "-c", "echo \"$TOKEN\""],
        b"",
    );
    assert_eq!(status, 0, "{err}");
    assert_eq!(out, "tok\n");

    // The name may be left off, and is then the variable, lowercased and hyphenated.
    let (out, err, status) = secret(
        home.path(),
        &["run", "GH_TOKEN=", "--", "sh", "-c", "echo \"$GH_TOKEN\""],
        b"",
    );
    assert_eq!(status, 0, "{err}");
    assert_eq!(out, "tok\n");

    // A secret that is not there is a failure before the command runs, not an empty variable.
    let (_, _, status) = secret(
        home.path(),
        &["run", "X=missing", "--", "sh", "-c", "exit 0"],
        b"",
    );
    assert_ne!(status, 0);
}
