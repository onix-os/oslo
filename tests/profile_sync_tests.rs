//! `oslo profile` — the key that says two machines mean one profile, and the sync that follows.
//!
//! Driven through the real binary with a stand-in for `ssh` (`$OSLO_SSH`), because what is under
//! test is two *machines*: two data directories, two stores, two keys. An in-process test could
//! exercise the merge and would never catch the half that matters — that the far end is asked, and
//! refuses.

mod common;

use common::oslo_bin;
use std::path::Path;
use std::process::Command;

/// One machine: its own data, state and home.
struct Machine {
    home: tempfile::TempDir,
}

impl Machine {
    fn new() -> Machine {
        Machine {
            home: tempfile::tempdir().expect("tempdir"),
        }
    }

    fn path(&self) -> &Path {
        self.home.path()
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        self.run_with(args, &[])
    }

    fn run_with(&self, args: &[&str], extra: &[(&str, String)]) -> (String, String, i32) {
        let mut command = Command::new(oslo_bin());
        command
            .args(args)
            .env("HOME", self.path())
            .env("XDG_DATA_HOME", self.path())
            .env("XDG_STATE_HOME", self.path().join("state"))
            .env_remove("OSLO_PROFILE");
        for (name, value) in extra {
            command.env(name, value);
        }
        let out = command.output().expect("spawn oslo");
        (
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
            out.status.code().unwrap_or(-1),
        )
    }

    /// Fill this machine's history with `lines`, through the door a person would use.
    fn remember(&self, lines: &[&str]) {
        let file = self.path().join("seed.txt");
        std::fs::write(&file, format!("{}\n", lines.join("\n"))).expect("write");
        let (_, err, status) = self.run(&["history", "import", &file.to_string_lossy()]);
        assert_eq!(status, 0, "{err}");
    }

    fn lines(&self) -> Vec<String> {
        let (out, err, status) = self.run(&["history", "list"]);
        assert_eq!(status, 0, "{err}");
        let mut found: Vec<String> = out
            .lines()
            .filter_map(|row| row.rsplit('\t').next().map(str::to_string))
            .collect();
        found.sort();
        found
    }
}

/// A stand-in for `ssh`: ignores the destination and runs the command against `machine`.
fn fake_ssh(at: &Path, machine: &Machine) -> String {
    let script = at.join("fake-ssh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nshift\nexec env HOME={home} XDG_DATA_HOME={home} \
             XDG_STATE_HOME={home}/state \"$@\"\n",
            home = machine.path().display()
        ),
    )
    .expect("write");
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("chmod");
    script.to_string_lossy().to_string()
}

fn syncing(here: &Machine, there: &Machine, extra_args: &[&str]) -> (String, String, i32) {
    let ssh = fake_ssh(here.path(), there);
    let mut args = vec!["profile", "sync", "buildbox"];
    args.extend_from_slice(extra_args);
    here.run_with(
        &args,
        &[
            ("OSLO_SSH", ssh),
            (
                "OSLO_SSH_REMOTE_BIN",
                oslo_bin().to_string_lossy().to_string(),
            ),
        ],
    )
}

/// **Without a shared key there is no sync.** `default` here and `default` on a machine you have an
/// account on are two histories that share a word; syncing on the strength of the word would merge
/// somebody else's commands into yours.
#[test]
fn a_profile_with_no_key_refuses_to_sync() {
    let (here, there) = (Machine::new(), Machine::new());
    here.remember(&["cargo build"]);
    there.remember(&["kubectl get pods"]);

    let (_, err, status) = syncing(&here, &there, &[]);
    assert_ne!(status, 0, "it synced without a key");
    assert!(err.contains("has no key here"), "{err:?}");

    // A key on one side only is still not a shared key.
    here.run(&["profile", "key", "init"]);
    let (_, err, status) = syncing(&here, &there, &[]);
    assert_ne!(status, 0, "it synced against a machine with no key");
    assert!(
        err.contains("not the same profile") || err.contains("exited"),
        "{err:?}"
    );
}

/// The whole of it: carry the key across once, then both ends end up with the union.
#[test]
fn two_machines_that_share_a_key_end_up_with_both_histories() {
    let (here, there) = (Machine::new(), Machine::new());
    here.remember(&["cargo build", "vim src/main.rs", "git push"]);
    there.remember(&["cargo build", "kubectl get pods", "docker compose up"]);

    // The one manual step: this machine's key, carried to the other.
    let (key, err, status) = here.run(&["profile", "key", "init"]);
    assert_eq!(status, 0, "{err}");
    assert_eq!(
        key.trim().len(),
        16,
        "a fingerprint is 16 hex characters: {key:?}"
    );

    let (exported, err, status) = here.run(&["profile", "export"]);
    assert_eq!(status, 0, "{err}");
    let carried = there.path().join("carried.key");
    std::fs::write(&carried, &exported).expect("write");
    let (_, err, status) = there.run(&[
        "profile",
        "import",
        // `import` reads standard input, so the file goes in through a shell redirect below.
    ]);
    assert_ne!(status, 0, "import with no input should fail: {err}");

    // Import for real, through stdin.
    let imported = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "HOME={home} XDG_DATA_HOME={home} XDG_STATE_HOME={home}/state {oslo} profile import < {file}",
            home = there.path().display(),
            oslo = oslo_bin().display(),
            file = carried.display(),
        ))
        .output()
        .expect("spawn");
    assert!(imported.status.success(), "{:?}", imported);

    // Both fingerprints now agree.
    let (mine, _, _) = here.run(&["profile", "fingerprint"]);
    let (theirs, _, _) = there.run(&["profile", "fingerprint"]);
    assert_eq!(mine.trim(), theirs.trim(), "the key did not arrive");

    // A dry run says what would move and moves nothing.
    let (said, err, status) = syncing(&here, &there, &["--dry-run"]);
    assert_eq!(status, 0, "{err}");
    assert!(said.contains("dry run"), "{said}");
    assert_eq!(here.lines().len(), 3, "a dry run wrote something");

    let (said, err, status) = syncing(&here, &there, &[]);
    assert_eq!(status, 0, "{err}\n{said}");

    // **Both `cargo build`s survive.** Every event carries the host that ran it, so the same command
    // on two machines is two events rather than a conflict — which is the property that makes this
    // worth doing at all.
    let expected = vec![
        "cargo build".to_string(),
        "cargo build".to_string(),
        "docker compose up".to_string(),
        "git push".to_string(),
        "kubectl get pods".to_string(),
        "vim src/main.rs".to_string(),
    ];
    assert_eq!(here.lines(), expected, "this machine");
    assert_eq!(there.lines(), expected, "the other machine");
}

/// Running it twice changes nothing the second time, which is what makes it safe to put in a cron
/// line or a login file.
#[test]
fn syncing_twice_moves_nothing_the_second_time() {
    let (here, there) = (Machine::new(), Machine::new());
    here.remember(&["cargo build"]);
    there.remember(&["kubectl get pods"]);

    here.run(&["profile", "key", "init"]);
    let (exported, _, _) = here.run(&["profile", "export"]);
    let carried = there.path().join("carried.key");
    std::fs::write(&carried, &exported).expect("write");
    Command::new("sh")
        .arg("-c")
        .arg(format!(
            "HOME={home} XDG_DATA_HOME={home} XDG_STATE_HOME={home}/state {oslo} profile import < {file}",
            home = there.path().display(),
            oslo = oslo_bin().display(),
            file = carried.display(),
        ))
        .output()
        .expect("spawn");

    let (_, err, status) = syncing(&here, &there, &[]);
    assert_eq!(status, 0, "{err}");

    let (said, err, status) = syncing(&here, &there, &[]);
    assert_eq!(status, 0, "{err}");
    assert!(said.contains("here     +0 ~0 -0"), "{said}");
    assert!(said.contains("unchanged 2"), "{said}");
}
