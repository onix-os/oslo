//! Exit-status truthfulness in the cases the differential corpus cannot arbitrate.
//!
//! Two kinds of case live here rather than in `tests/corpus`:
//!
//! * **a child killed by a signal.** Reproducing one means having the victim kill its own parent
//!   (`kill -9 $PPID`), and which process that is depends on how many times the shell forked —
//!   bash execs the last stage of a pipeline in place and would kill *itself*. The number a shell
//!   must report is not in dispute (128 + signo), so it is asserted directly.
//! * **stdin.** Every corpus case runs with stdin on `/dev/null` precisely so a dropped
//!   redirection cannot hang the suite, which is also what makes it blind to a background job
//!   stealing the terminal's input.

mod common;

use common::oslo_bin;
use std::io::Write;
use std::process::{Command, Stdio};

/// Run `script` with `input` on stdin, in `dir`.
fn run_with_stdin(dir: &std::path::Path, script: &str, input: &str) -> (String, i32) {
    let mut child = Command::new(oslo_bin())
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn oslo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait oslo");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// The last stage of a pipeline dies on SIGKILL: the pipeline is 137, not 0.
///
/// `waitpid` used to be called once per `if let` arm, so the `Signaled` arm tested a *second*
/// call that could only fail — and a killed stage was reported as a clean exit.
#[test]
fn a_signalled_last_stage_reports_128_plus_the_signal() {
    // `sh` kills its parent, which is the process oslo forked for the stage — oslo runs an
    // external command in a further child, so `$PPID` here is the stage and not the shell.
    let r = common::run(r#"echo x | sh -c 'kill -9 $PPID; sleep 5'; echo "status=$?""#);
    assert_eq!(r.out(), "status=137", "stderr: {}", r.stderr);
}

/// A background job started by a non-interactive shell announces nothing, on either stream.
///
/// The notice used to go to stdout unconditionally, so every `$( )` containing a `&` captured
/// `[bg] <pid>` as part of its value.
#[test]
fn a_non_interactive_shell_prints_no_job_notice() {
    let r = common::run("sleep 0 & wait");
    assert_eq!(r.stdout, "");
    assert_eq!(r.stderr, "");

    let captured = common::run(r#"x=$(sleep 0 & echo done); echo "[$x]""#);
    assert_eq!(captured.out(), "[done]", "stderr: {}", captured.stderr);
}

/// A background job does not read the input the shell was given.
///
/// Without job control there is no separate process group keeping the job off the terminal, so
/// POSIX has the shell give it `/dev/null` instead. Before that, `cmd &` and the shell raced for
/// every keystroke and the job usually won.
#[test]
fn a_background_job_does_not_steal_stdin() {
    let dir = tempfile::tempdir().expect("tempdir");
    // `wait` makes this deterministic: `cat` has finished before the shell reads anything, so if
    // it had the shell's stdin it would have drained the lot.
    let script = r#"cat > drained & wait
read a
read b
echo "[$a][$b]"
echo "drained=[$(cat drained)]""#;
    let (stdout, status) = run_with_stdin(dir.path(), script, "hello\nworld\n");
    assert_eq!(status, 0);
    assert_eq!(stdout, "[hello][world]\ndrained=[]\n");
}
