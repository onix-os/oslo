//! **Nothing here runs the real nix.** CI has none, and a test that needs it is a test that fails
//! on somebody else's machine. Each of these builds a script that impersonates the one behaviour
//! under examination and hands it to [`run_program`].

use super::*;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

/// A fake `nix` that runs `body`, in a directory that lives as long as the returned handle.
fn fake(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("nix");
    let mut file = std::fs::File::create(&path).expect("create");
    write!(file, "#!/bin/sh\n{body}\n").expect("write");
    drop(file);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    (dir, path)
}

fn run_fake(body: &str, argv: &[&str]) -> Result<String, String> {
    let (_dir, path) = fake(body);
    let argv: Vec<String> = argv.iter().map(|a| (*a).to_string()).collect();
    // **Retried on `ETXTBSY`.** These tests run in parallel, and a script written by one thread
    // cannot be `exec`ed while another thread sits between `fork` and `exec` holding an inherited
    // write descriptor for it — `O_CLOEXEC` closes it at `exec`, which is after the kernel has
    // already refused. Nothing about it is oslo's, so it is absorbed here rather than in `run`.
    for _ in 0..100 {
        match run_program(path.as_os_str(), &argv, Duration::from_secs(10)) {
            Err(e) if e.contains("Text file busy") => {
                std::thread::sleep(Duration::from_millis(10));
            }
            answer => return answer,
        }
    }
    panic!("`{}` stayed busy", path.display())
}

#[test]
fn stdout_is_answered_verbatim() {
    let out = run_fake(r#"printf '{"a":1}'"#, &["flake", "metadata"]).expect("ran");
    assert_eq!(out, r#"{"a":1}"#);
}

#[test]
fn the_arguments_reach_nix_in_order_and_untouched() {
    // Including one that a shell would have taken apart. This is the whole reason the invocation
    // does not build a string: `--option` with a two-word value survives only as argv.
    let out = run_fake(
        r#"printf '%s\n' "$@""#,
        &["eval", "--option", "warn-dirty false"],
    )
    .expect("ran");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "--extra-experimental-features",
            "nix-command flakes",
            "eval",
            "--option",
            "warn-dirty false",
            "--json",
        ],
    );
}

#[test]
fn json_is_not_passed_twice_when_the_caller_wrote_it() {
    let out = run_fake(r#"printf '%s\n' "$@""#, &["flake", "show", "--json"]).expect("ran");
    assert_eq!(out.matches("--json").count(), 1, "{out}");
}

#[test]
fn a_failing_command_answers_its_message_rather_than_an_empty_document() {
    // What `nix registry list --json` really does: the flag is in its help and is refused.
    let err = run_fake(
        r#"echo "error: unrecognised flag '--json'" >&2; exit 1"#,
        &["registry", "list"],
    )
    .expect_err("should fail");
    assert_eq!(err, "error: unrecognised flag '--json'");
}

#[test]
fn a_failure_with_nothing_on_stderr_still_reports_the_status() {
    let err = run_fake("exit 3", &["build"]).expect_err("should fail");
    assert!(err.contains('3'), "{err}");
}

#[test]
fn a_document_larger_than_a_pipe_buffer_arrives_whole() {
    // **The deadlock test.** `nix config show --json` is 76 KB against a 64 KB pipe buffer, so a
    // version that waits before draining hangs here rather than failing — which is why this asks
    // for 256 KB and why the readers are on their own threads.
    let out = run_fake(
        r#"awk 'BEGIN { while (i++ < 4096) printf "%064d", i }'"#,
        &["config", "show"],
    )
    .expect("ran");
    assert_eq!(out.len(), 256 * 1024);
}

#[test]
fn a_command_that_overruns_is_killed_and_says_so() {
    let (_dir, path) = fake("sleep 30");
    let argv = vec!["flake".to_string(), "show".to_string()];
    // Retried on `ETXTBSY` like `run_fake`, and for the same reason — this one calls
    // `run_program` directly, so it was the one case the retry there did not cover, and it
    // failed under a loaded machine months after the rest stopped doing so.
    let started = Instant::now();
    let mut answer = run_program(path.as_os_str(), &argv, Duration::from_millis(150));
    for _ in 0..100 {
        match &answer {
            Err(problem) if problem.contains("Text file busy") => {
                std::thread::sleep(Duration::from_millis(10));
                answer = run_program(path.as_os_str(), &argv, Duration::from_millis(150));
            }
            _ => break,
        }
    }
    let err = answer.expect_err("should time out");
    assert!(err.contains("timed out"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "was not killed"
    );
}

#[test]
fn a_missing_nix_is_a_message_rather_than_a_panic() {
    let argv = vec!["flake".to_string(), "metadata".to_string()];
    let err = run_program("/nonexistent/nix".as_ref(), &argv, Duration::from_secs(1))
        .expect_err("should fail");
    assert_eq!(err, "nix is not installed");
}
