//! `kill` and `umask` against the only witnesses that can tell the truth about them: a live
//! victim process, and a file created on disk.
//!
//! Both builtins used to pass their tests while doing nothing (or the wrong thing) — `umask`
//! returned 0 with the mask unchanged, and `kill -0` returned 0 having *terminated* the process
//! it was asked to probe. An assertion on the exit status alone cannot see either. So the kill
//! tests watch what happens to a victim that traps its signals, and the umask tests read the
//! permission bits off a file the shell created.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// A victim that survives the default disposition of nothing: it exits with a distinct status
/// per signal, so the *identity* of what arrived is visible in its wait status.
///
/// The busy loop is `sleep` in small slices rather than one long sleep because a shell only runs
/// a trap between commands — the signal interrupts the sleep, and the handler runs immediately.
fn spawn_victim() -> Child {
    Command::new("sh")
        .arg("-c")
        .arg(
            "trap 'exit 41' HUP; trap 'exit 42' USR1; trap 'exit 43' TERM; \
             while :; do sleep 0.05; done",
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn victim")
}

/// Give the victim time to install its traps. Without this the signal can land on a shell that
/// has not reached the `trap` builtin yet and the default disposition kills it.
fn settle() {
    sleep(Duration::from_millis(200));
}

/// Wait for the victim to exit, with a budget: a signal that never arrives must fail the test
/// rather than hang the suite.
fn wait_for_exit(child: &mut Child) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match child.try_wait().expect("try_wait") {
            Some(status) => return Some(status.code().unwrap_or(-1)),
            None => sleep(Duration::from_millis(20)),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

fn still_running(child: &mut Child) -> bool {
    child.try_wait().expect("try_wait").is_none()
}

/// The finding, stated as a test: `kill -0` is a probe. It reports whether the process exists
/// and it leaves it running.
#[test]
fn signal_zero_probes_without_signalling() {
    let mut victim = spawn_victim();
    settle();

    let r = common::run(&format!(
        "kill -0 {pid}; echo \"probe=$?\"",
        pid = victim.id()
    ));
    assert_eq!(r.out(), "probe=0", "stderr: {}", r.stderr);

    settle();
    let alive = still_running(&mut victim);
    let _ = victim.kill();
    let _ = victim.wait();
    assert!(
        alive,
        "kill -0 terminated the process it was asked to probe"
    );
}

/// Every spelling of a signal name has to reach the same signal. The victim's exit status says
/// which one actually arrived — the old code sent TERM for all of them.
#[test]
fn named_signals_are_delivered_by_name() {
    // Taken from the name rather than written as `-10`: a literal here asserts the kernel's
    // numbering as much as oslo's parsing, and only one of those is under test.
    let usr1 = format!("-{}", nix::sys::signal::Signal::SIGUSR1 as i32);

    for (spec, expected) in [
        ("-HUP", 41),
        ("-USR1", 42),
        ("-usr1", 42),
        ("-SIGHUP", 41),
        ("-s USR1", 42),
        ("-n 1", 41),
        (usr1.as_str(), 42),
    ] {
        let mut victim = spawn_victim();
        settle();
        let r = common::run(&format!("kill {spec} {pid}", pid = victim.id()));
        assert_eq!(r.status, 0, "kill {spec} failed: {}", r.stderr);
        assert_eq!(
            wait_for_exit(&mut victim),
            Some(expected),
            "kill {spec} delivered the wrong signal"
        );
    }
}

/// An unresolvable spec must send nothing at all. This is the same defect as `kill -0`: the
/// signal defaulted to TERM and stayed there when the parse failed.
#[test]
fn an_invalid_signal_spec_signals_nothing() {
    let mut victim = spawn_victim();
    settle();

    for bad in ["-NOSUCHSIG", "-99", "-s NOPE", "-n 999"] {
        let r = common::run(&format!("kill {bad} {pid}", pid = victim.id()));
        assert_eq!(r.status, 1, "kill {bad} should fail: {}", r.stdout);
        assert!(!r.stderr.is_empty(), "kill {bad} should diagnose");
    }

    settle();
    let alive = still_running(&mut victim);
    let _ = victim.kill();
    let _ = victim.wait();
    assert!(alive, "an invalid signal spec still signalled the process");
}

#[test]
fn a_non_numeric_operand_is_reported() {
    let r = common::run("kill -0 abc; echo \"s=$?\"");
    assert_eq!(r.out(), "s=1");
    assert!(!r.stderr.is_empty());
}

/// `kill -l` in both directions, plus the full listing.
#[test]
fn kill_dash_l_lists_and_translates() {
    let r = common::run("kill -l 9; kill -l KILL; kill -l SIGKILL; kill -l 0");
    assert_eq!(r.out(), "KILL\n9\n9\nEXIT", "stderr: {}", r.stderr);

    let r = common::run("kill -l");
    assert!(r.stdout.contains("HUP"), "listing was {:?}", r.stdout);
    assert!(r.stdout.contains("TERM"));
    assert!(r.stdout.contains("KILL"));
    assert_eq!(r.status, 0);
}

/// The assertion the old umask tests were missing: the mask has to actually move, and the only
/// proof of that is a file created afterwards.
#[test]
fn umask_symbolic_mode_changes_the_real_mask() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = common::run_in(dir.path(), "umask u=rwx,g=,o=; umask; echo x > created");
    assert_eq!(r.out(), "0077", "stderr: {}", r.stderr);

    let mode = fs::metadata(dir.path().join("created"))
        .expect("created file")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "file created under a mask that never moved");
}

#[test]
fn umask_octal_mode_changes_the_real_mask() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = common::run_in(dir.path(), "umask 027; umask; echo x > created");
    assert_eq!(r.out(), "0027", "stderr: {}", r.stderr);

    let mode = fs::metadata(dir.path().join("created"))
        .expect("created file")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o640);
}

/// A refused operand must leave the mask exactly where it was — the diagnostic is only half the
/// fix if the mask has already been half-written.
#[test]
fn a_rejected_mode_leaves_the_mask_alone() {
    for bad in ["999", "8", "77777", "abc", "u=q"] {
        let r = common::run(&format!("umask 022; umask {bad}; echo \"s=$?\"; umask"));
        assert_eq!(r.out(), "s=1\n0022", "umask {bad}: stderr {}", r.stderr);
        assert!(!r.stderr.is_empty(), "umask {bad} should diagnose");
    }
}

#[test]
fn umask_print_forms() {
    assert_eq!(common::run("umask 022; umask -S").out(), "u=rwx,g=rx,o=rx");
    assert_eq!(common::run("umask 022; umask -p").out(), "umask 0022");
    assert_eq!(
        common::run("umask 077; umask -p -S").out(),
        "umask -S u=rwx,g=,o="
    );
    let r = common::run("umask -w");
    assert_eq!(r.status, 2);
    assert!(!r.stderr.is_empty());
}
