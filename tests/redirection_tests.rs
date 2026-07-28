//! Redirection: files, fds, heredocs and here-strings.
//!
//! Spawns the real binary; see `common/mod.rs` for why.

mod common;

use common::{assert_out, run, run_in};

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
