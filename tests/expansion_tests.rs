//! Expansion, pipelines, substitution and script invocation.
//!
//! Spawns the real binary; see `common/mod.rs` for why.

mod common;

use common::{assert_out, run, rush_bin};
use std::io::Write;
use std::process::{Command, Stdio};

// --- general ---

#[test]
fn pipelines_pass_data_between_stages() {
    assert_out("echo hello | tr a-z A-Z", "HELLO");
    assert_out("printf 'a\\nb\\nc\\n' | grep b", "b");
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

    let output = Command::new(rush_bin())
        .arg(&script)
        .arg("hello")
        .stdin(Stdio::null())
        .output()
        .expect("spawn rush");

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
