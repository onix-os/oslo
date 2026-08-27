//! Redirection: files, fds, heredocs and here-strings.
//!
//! Spawns the real binary; see `common/mod.rs` for why.

mod common;

use common::{assert_out, run, run_in};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// --- redirection ---

#[test]
fn output_redirection_writes_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let r = run_in(dir.path(), "echo hello > out.txt");
    assert_eq!(r.out(), "", "output should go to the file, not stdout");

    let written = std::fs::read_to_string(dir.path().join("out.txt")).expect("out.txt");
    assert_eq!(written, "hello\n");
}

#[test]
fn append_redirection_does_not_truncate() {
    assert_out("echo a > f; echo b >> f; cat f", "a\nb");
}

#[test]
fn output_redirection_truncates_by_default() {
    assert_out("echo first > f; echo second > f; cat f", "second");
}

#[test]
fn input_redirection_reads_the_file() {
    assert_out("printf 'x\\ny\\n' > f; cat < f", "x\ny");
}

#[test]
fn stderr_can_be_redirected_separately() {
    let dir = tempfile::tempdir().unwrap();
    let r = run_in(dir.path(), "ls /nonexistent-path-xyz 2> err.txt");
    assert_eq!(r.out(), "");
    let err = std::fs::read_to_string(dir.path().join("err.txt")).expect("err.txt");
    assert!(!err.is_empty(), "stderr should have been captured");
}

#[test]
fn stderr_can_be_merged_into_stdout() {
    let r = run("ls /nonexistent-path-xyz 2>&1");
    assert!(
        r.stdout.contains("nonexistent-path-xyz"),
        "expected the error on stdout, got stdout={:?} stderr={:?}",
        r.stdout,
        r.stderr
    );
}

#[test]
fn ampersand_redirect_captures_both_streams() {
    let dir = tempfile::tempdir().unwrap();
    run_in(dir.path(), "sh -c 'echo out; echo err >&2' &> both.txt");
    let both = std::fs::read_to_string(dir.path().join("both.txt")).expect("both.txt");
    assert!(both.contains("out"), "stdout missing: {:?}", both);
    assert!(both.contains("err"), "stderr missing: {:?}", both);
}

#[test]
fn redirection_applies_to_a_compound_command() {
    assert_out(
        "for i in 1 2; do echo $i; done > loop.txt; cat loop.txt",
        "1\n2",
    );
}

#[test]
fn redirection_works_inside_a_pipeline() {
    assert_out("printf 'a\\nb\\n' > f; cat < f | tr a-z A-Z", "A\nB");
}

#[test]
fn heredoc_feeds_stdin() {
    assert_out("cat <<EOF\nline1\nline2\nEOF", "line1\nline2");
}

#[test]
fn dash_heredoc_strips_leading_tabs() {
    assert_out("cat <<-EOF\n\tindented\nEOF", "indented");
}

#[test]
fn here_string_feeds_stdin() {
    assert_out("cat <<< hello", "hello");
    assert_out(r#"cat <<< "a b""#, "a b");
}

/// A heredoc body used to be written into a pipe with no reader attached, so anything past the
/// 64 KB pipe buffer wedged the shell forever. This cannot use `run()`: a regression here hangs
/// instead of failing, and `Command::output` would block the whole test run.
#[test]
fn a_large_heredoc_does_not_deadlock() {
    const LINE: &str = "0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmno";
    const LINES: usize = 10_486; // just over 1 MB at 100 bytes a line

    let mut script = String::from("cat <<EOF\n");
    for _ in 0..LINES {
        script.push_str(LINE);
        script.push('\n');
    }
    script.push_str("EOF\n");

    let dir = tempfile::tempdir().unwrap();
    // A script file, not `-c`: Linux caps a single argv string at MAX_ARG_STRLEN (128 KB), well
    // under the size that used to deadlock.
    let script_path = dir.path().join("heredoc.sh");
    std::fs::write(&script_path, &script).unwrap();

    let out_path = dir.path().join("heredoc.out");
    // stdout goes to a file rather than a pipe so the deadline below measures the shell, not a
    // test harness that forgot to drain 1 MB.
    let mut child = Command::new(common::oslo_bin())
        .arg(&script_path)
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&out_path).unwrap())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn oslo");

    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("a {}-byte heredoc deadlocked the shell", LINES * 100);
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };

    assert!(status.success(), "oslo exited with {:?}", status.code());
    let written = std::fs::metadata(&out_path).unwrap().len();
    assert_eq!(
        written,
        (LINES * (LINE.len() + 1)) as u64,
        "the whole heredoc body should reach the command's stdin"
    );
}

/// **A number too big to be a file descriptor is a command word.** Ninety-six threes in front of a
/// `>` used to reach `w.parse::<IoFd>().unwrap()` in the parser and panic — `PosOverflow`, found by
/// the parse fuzzer and now a seed in `fuzz/seeds/fuzz_parse/`. bash reads it as a word and runs it,
/// which is what this now does.
#[test]
fn an_oversized_fd_is_a_word_rather_than_a_panic() {
    let huge = "3".repeat(96);
    let r = run(&format!("{huge}> out.txt"));
    assert!(
        r.stderr.contains("command not found"),
        "stderr was {:?}",
        r.stderr
    );
    assert!(
        !r.stderr.contains("panicked"),
        "the parser panicked: {:?}",
        r.stderr
    );

    // And an fd that does fit is still a redirection.
    let r = run("echo hi 2>/dev/null");
    assert_eq!(r.out(), "hi");
}

/// **`>&file` names a file and means both streams**, the same as `&>file`.
///
/// With no descriptor in front of it and a target that is not a number, `>&` is the widespread
/// spelling in `cmd >&/dev/null`. It used to be refused as an invalid descriptor, so the command
/// ran anyway and neither stream was captured.
#[test]
fn a_bare_dup_to_a_name_takes_both_streams() {
    let dir = tempfile::tempdir().unwrap();
    let r = run_in(dir.path(), "sh -c 'echo out; echo err >&2' >& both.txt");
    assert_eq!(r.out(), "", "both streams went to the file");

    let written = std::fs::read_to_string(dir.path().join("both.txt")).expect("both.txt");
    let mut lines: Vec<&str> = written.lines().collect();
    lines.sort_unstable();
    assert_eq!(lines, vec!["err", "out"]);
}

/// **A redirection target is one filename or it is a mistake.**
///
/// `f="a b"; echo hi > $f` used to join the fields with a space, create a file literally called
/// `a b` and report success — so `> $LOGFILES` wrote garbage somewhere and the intended file was
/// never written. bash refuses, naming the word rather than its expansion, because the expansion of
/// an empty target is empty and would name nothing at all.
#[test]
fn a_target_that_is_not_one_word_is_refused() {
    let dir = tempfile::tempdir().unwrap();

    let several = run_in(dir.path(), r#"f="a b"; echo hi > $f"#);
    assert!(
        several.err().contains("$f: ambiguous redirect"),
        "several fields: {:?}",
        several.err()
    );
    assert!(
        !dir.path().join("a b").exists(),
        "and no file was created for it"
    );

    let empty = run_in(dir.path(), "f=; echo hi > $f");
    assert!(
        empty.err().contains("$f: ambiguous redirect"),
        "no fields at all: {:?}",
        empty.err()
    );

    // Quoted, an empty target *is* one word — a filename that happens to be empty, which the open
    // refuses for a different and more accurate reason.
    let quoted = run_in(dir.path(), r#"f=; echo hi > "$f""#);
    assert!(
        quoted.err().contains("No such file or directory"),
        "quoted empty: {:?}",
        quoted.err()
    );
}
