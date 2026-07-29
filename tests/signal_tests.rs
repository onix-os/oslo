//! What a command rush launches inherits, and what it must not.
//!
//! These go through the real binary because the defect they cover lives in the window between
//! `fork` and `execv`: an in-process test can inspect `Environment` all it likes and never see
//! that the program on the other side of `execv` started life with SIGPIPE ignored.

mod common;

use common::run;
use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The kernel reports the ignored-signal set as a hex bitmask in `/proc/self/status`.
///
/// Anything non-zero here is a disposition the child did not ask for and cannot see: the Rust
/// runtime's `SIG_IGN` for SIGPIPE (bit 13, `0x1000`), and — in the REPL — the shell's own
/// `SIG_IGN` for SIGTSTP/SIGTTIN/SIGTTOU. bash leaves this field at zero.
fn sig_ign_mask(script: &str) -> u64 {
    let r = run(script);
    let line = r
        .stdout
        .lines()
        .find(|l| l.starts_with("SigIgn:"))
        .unwrap_or_else(|| panic!("no SigIgn line in:\n{}\nstderr: {}", r.stdout, r.stderr))
        .to_string();
    let hex = line.split_whitespace().nth(1).expect("SigIgn value");
    u64::from_str_radix(hex, 16).expect("hex mask")
}

#[test]
fn an_exec_ed_child_ignores_nothing() {
    assert_eq!(sig_ign_mask("grep SigIgn /proc/self/status"), 0);
}

/// A pipeline stage forks twice — once for the stage, once for the command — so it is the path
/// most likely to lose the reset.
#[test]
fn a_pipeline_stage_ignores_nothing() {
    assert_eq!(sig_ign_mask("cat /proc/self/status | grep SigIgn"), 0);
}

/// The point of restoring SIG_DFL for SIGPIPE: `yes` must be killed by the closed pipe rather
/// than surviving the failed write and complaining about it.
#[test]
fn a_writer_to_a_closed_pipe_dies_quietly() {
    let r = run("yes | head -1");
    assert_eq!(r.out(), "y");
    assert_eq!(r.stderr, "", "SIGPIPE was still ignored in the child");
    assert_eq!(r.status, 0);
}

/// The other half of restoring SIG_DFL for SIGTSTP: a child that *does* stop must not take the
/// shell down with it.
///
/// `waitpid` without `WUNTRACED` only reports termination, so a suspended job would leave the
/// shell blocked forever on a process nothing can resume — a suspend that turns into a hang is
/// worse than a Ctrl-Z that does nothing. The command is run under a watchdog so a regression
/// fails this test instead of wedging the whole test run.
#[test]
fn a_stopped_child_does_not_wedge_the_shell() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = "sh -c 'echo $$ > pid; kill -STOP $$'\necho \"s=$?\"\necho CONTINUE";

    let mut child = Command::new(common::rush_bin())
        .arg("-c")
        .arg(script)
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rush");

    let deadline = Instant::now() + Duration::from_secs(10);
    let wedged = loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break false,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                break true;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };

    // The stopped process is deliberately abandoned by the shell, so this test owns it: without
    // this it survives as a stopped orphan for as long as the machine is up.
    if let Ok(pid) = fs::read_to_string(dir.path().join("pid")) {
        let _ = Command::new("kill").arg("-KILL").arg(pid.trim()).status();
    }

    let mut stdout = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }

    assert!(!wedged, "the shell blocked on a stopped child");
    // 128 + SIGSTOP, the status a shell reports for a job it left suspended.
    let stopped = 128 + nix::sys::signal::Signal::SIGSTOP as i32;
    assert!(
        stdout.contains(&format!("s={stopped}")),
        "expected the stop to be reported as {stopped}, got:\n{stdout}"
    );
    assert!(stdout.contains("CONTINUE"), "script did not continue");
}

/// A blocked signal survives `exec` too, so the mask has to be cleared as well as the handlers.
#[test]
fn an_exec_ed_child_blocks_nothing() {
    let r = run("grep SigBlk /proc/self/status");
    let hex = r
        .stdout
        .split_whitespace()
        .nth(1)
        .expect("SigBlk value")
        .to_string();
    assert_eq!(u64::from_str_radix(&hex, 16).expect("hex mask"), 0);
}
