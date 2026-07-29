//! Expansion, pipelines, substitution and script invocation.
//!
//! Spawns the real binary; see `common/mod.rs` for why.

mod common;

use common::{assert_out, oslo_bin, run};
use std::io::Write;
use std::process::{Command, Stdio};

// --- general ---

#[test]
fn pipelines_pass_data_between_stages() {
    assert_out("echo hello | tr a-z A-Z", "HELLO");
    assert_out("printf 'a\\nb\\nc\\n' | grep b", "b");
}

/// A pipeline's status is its last stage's. Moved here from the in-process suite: forking a
/// pipeline stage out of libtest's thread pool deadlocked the child (see `posix_shell_tests.rs`).
#[test]
fn pipeline_status_is_the_last_stage() {
    assert_eq!(run("echo hello | grep -q hello").status, 0);
    assert_eq!(run("echo hello | grep -q nope").status, 1);
}

#[test]
fn and_or_short_circuits() {
    assert_out("false && echo NO || echo YES", "YES");
    assert_out("true && echo YES || echo NO", "YES");
}

#[test]
fn command_substitution_captures_output() {
    assert_out("X=$(echo sub); echo got:$X", "got:sub");
}

/// R2.9: an assignment RHS gets command substitution but no field splitting, so a value that
/// contains newlines keeps them instead of collapsing to `a b`. Lives here rather than beside
/// the other R2.9 unit tests in `src/exec/simple.rs` because command substitution forks, and
/// forking out of libtest's thread pool deadlocks the child in the allocator.
#[test]
fn assignment_rhs_keeps_embedded_newlines() {
    assert_out(
        "oslo_n1=$(printf 'a\\nb'); printf '[%s]' \"$oslo_n1\"",
        "[a\nb]",
    );
    // Trailing newlines are still stripped: that is command substitution's own rule.
    assert_out(
        "oslo_n2=$(printf 'a\\n\\n\\n'); printf '[%s]' \"$oslo_n2\"",
        "[a]",
    );
}

#[test]
fn arithmetic_expansion_evaluates() {
    assert_out("echo $((2 + 3 * 4))", "14");
    assert_out("X=10; echo $((X + 5))", "15");
}

#[test]
fn quoted_arguments_survive_as_one_word() {
    assert_out(r#"echo "a   b""#, "a   b");
    assert_out("echo 'a   b'", "a   b");
}

#[test]
fn positional_parameters_in_functions() {
    assert_out(r#"f() { echo "$#:$1:$2"; }; f a b"#, "2:a:b");
}

#[test]
fn subshell_does_not_leak_variables() {
    assert_out("V=1; (V=2); echo $V", "1");
}

#[test]
fn glob_expansion_still_works() {
    assert_out("touch f1 f2; echo f*", "f1 f2");
}

#[test]
fn unmatched_glob_is_left_alone() {
    assert_out("echo nomatch*xyz", "nomatch*xyz");
}

#[test]
fn script_files_run_with_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("s.sh");
    let mut f = std::fs::File::create(&script).unwrap();
    writeln!(f, "echo \"arg1=$1\"").unwrap();
    writeln!(f, "exit 4").unwrap();
    drop(f);

    let output = Command::new(oslo_bin())
        .arg(&script)
        .arg("hello")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");

    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim_end(),
        "arg1=hello"
    );
    assert_eq!(output.status.code(), Some(4));
}

#[test]
fn a_syntax_error_is_reported_and_fails() {
    let r = run("if ; then");
    assert_ne!(r.status, 0, "a syntax error must not exit 0");
    assert!(!r.stderr.is_empty(), "expected a diagnostic on stderr");
}
