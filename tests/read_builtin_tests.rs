//! End-to-end tests for the `read` builtin.
//!
//! `read` used to report success at end of input, so `while read l; do ...; done < file` spun
//! forever (measured at 314,453 blank iterations in three seconds). Every test here therefore
//! runs under a wall-clock bound and *fails* rather than wedging the suite if that regresses.
//!
//! Each case also asserts against bash when bash is on PATH: `read` is nothing but a pile of
//! edge cases (partial final line, remainder splitting, backslash handling), and hand-written
//! expectations drift from the shell they are supposed to imitate.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BOUND: Duration = Duration::from_secs(10);

mod common;
use common::rush_bin;

struct Outcome {
    stdout: String,
    status: i32,
}

/// Run `script` with `-c`, in a scratch directory pre-populated with `files`.
///
/// Output is captured to a file rather than a pipe: a runaway loop that outfills the pipe buffer
/// would otherwise block on a reader we do not have, and read as a hang in the shell under test.
fn execute(program: &Path, script: &str, files: &[(&str, &str)]) -> Outcome {
    let root = tempfile::tempdir().expect("tempdir");
    let cwd = root.path().join("cwd");
    fs::create_dir(&cwd).expect("mkdir");
    for (name, content) in files {
        fs::write(cwd.join(name), content).expect("fixture");
    }
    let out_path = root.path().join("stdout");

    let mut child = Command::new(program)
        .arg("-c")
        .arg(script)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(fs::File::create(&out_path).expect("stdout file"))
        .stderr(Stdio::null())
        .env_remove("ENV")
        .env_remove("BASH_ENV")
        .env("LC_ALL", "C")
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {}: {e}", program.display()));

    let deadline = Instant::now() + BOUND;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "{} did not finish within {BOUND:?} — `read` is looping at EOF again\nscript: {script}",
                    program.display()
                );
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };

    Outcome {
        stdout: String::from_utf8_lossy(&fs::read(&out_path).expect("read stdout")).into_owned(),
        status: status.code().unwrap_or(-1),
    }
}

fn bash() -> Option<std::path::PathBuf> {
    ["/bin/bash", "/usr/bin/bash"]
        .iter()
        .map(Path::new)
        .find(|p| p.exists())
        .map(Path::to_path_buf)
}

/// Assert rush's output, and that bash — the oracle — agrees with that same expectation.
fn assert_read(script: &str, files: &[(&str, &str)], expected: &str) {
    let rush = execute(&rush_bin(), script, files);
    assert_eq!(rush.stdout, expected, "rush output\nscript: {script}");

    if let Some(bash) = bash() {
        let oracle = execute(&bash, script, files);
        assert_eq!(
            oracle.stdout, expected,
            "bash disagrees with the expectation, so the expectation is wrong\nscript: {script}"
        );
        assert_eq!(
            rush.status, oracle.status,
            "exit status differs from bash\nscript: {script}"
        );
    }
}

const THREE_LINES: (&str, &str) = ("three.txt", "one\ntwo\nthree\n");

#[test]
fn while_read_over_a_three_line_file_runs_exactly_three_times() {
    assert_read(
        "n=0; while read l; do n=$((n+1)); echo \"$n:$l\"; done < three.txt; echo iters=$n",
        &[THREE_LINES],
        "1:one\n2:two\n3:three\niters=3\n",
    );
}

#[test]
fn read_past_the_last_line_reports_failure() {
    assert_read(
        "{ read a; read b; read c; read d; echo \"$?:[$a][$b][$c][$d]\"; } < three.txt",
        &[THREE_LINES],
        "1:[one][two][three][]\n",
    );
}

#[test]
fn read_succeeds_on_every_terminated_line() {
    assert_read(
        "{ read a; echo $?; read b; echo $?; read c; echo $?; } < three.txt",
        &[THREE_LINES],
        "0\n0\n0\n",
    );
}

#[test]
fn unterminated_final_line_is_assigned_but_fails() {
    // The value still lands in the variable; only the status says the delimiter was missing.
    // That combination is why the loop below stops at two iterations rather than three.
    assert_read(
        "{ read a; read b; echo \"$?:[$b]\"; } < partial.txt",
        &[("partial.txt", "a\nb")],
        "1:[b]\n",
    );
}

#[test]
fn while_read_drops_an_unterminated_final_line() {
    assert_read(
        "n=0; while read l; do n=$((n+1)); echo \"$n:$l\"; done < partial.txt; echo iters=$n",
        &[("partial.txt", "one\ntwo\nthree")],
        "1:one\n2:two\niters=2\n",
    );
}

#[test]
fn eof_clears_the_named_variables() {
    assert_read(
        "v=stale; read v < /dev/null; echo \"$?:[$v]\"",
        &[],
        "1:[]\n",
    );
}

#[test]
fn a_blank_line_is_data_not_eof() {
    assert_read(
        "{ read a; echo \"$?:[$a]\"; read b; echo \"$?:[$b]\"; } < blank.txt",
        &[("blank.txt", "\n")],
        "0:[]\n1:[]\n",
    );
}

#[test]
fn read_without_names_fills_reply() {
    // REPLY keeps the line verbatim: no field splitting, no trimming.
    assert_read(
        "read < spaced.txt; echo \"$?:[$REPLY]\"",
        &[("spaced.txt", "  hi   there \n")],
        "0:[  hi   there ]\n",
    );
}

#[test]
fn while_read_without_names_terminates() {
    assert_read(
        "n=0; while read; do n=$((n+1)); echo \"$n:$REPLY\"; done < three.txt; echo iters=$n",
        &[THREE_LINES],
        "1:one\n2:two\n3:three\niters=3\n",
    );
}

#[test]
fn extra_names_are_emptied_and_the_last_takes_the_remainder() {
    assert_read(
        "read a b c < fields.txt; echo \"[$a][$b][$c]\"",
        &[("fields.txt", "1 2\n")],
        "[1][2][]\n",
    );
}

#[test]
fn the_last_name_keeps_the_remainder_verbatim() {
    // Separators inside the remainder survive; only the trailing ones are stripped.
    assert_read(
        "read a b < fields.txt; echo \"[$a][$b]\"",
        &[("fields.txt", "1  2   3   \n")],
        "[1][2   3]\n",
    );
}

#[test]
fn leading_and_trailing_whitespace_is_stripped_from_a_single_name() {
    assert_read(
        "read a < fields.txt; echo \"[$a]\"",
        &[("fields.txt", "   x\ty   \n")],
        "[x\ty]\n",
    );
}

#[test]
fn a_backslash_escapes_a_field_separator() {
    assert_read(
        "read x y < esc.txt; echo \"[$x][$y]\"",
        &[("esc.txt", "a\\ b c\n")],
        "[a b][c]\n",
    );
}

#[test]
fn dash_r_keeps_backslashes_literal() {
    assert_read(
        "read -r x y < esc.txt; echo \"[$x][$y]\"",
        &[("esc.txt", "a\\ b c\n")],
        "[a\\][b c]\n",
    );
}

#[test]
fn dash_r_is_not_treated_as_a_variable_name() {
    // The pre-fix builtin assigned the line to a variable literally called `-r`.
    assert_read(
        "read -r v < three.txt; echo \"[$v]\"",
        &[THREE_LINES],
        "[one]\n",
    );
}

#[test]
fn backslash_newline_continues_onto_the_next_line() {
    assert_read(
        "{ read x; echo \"$?:[$x]\"; read y; echo \"[$y]\"; } < cont.txt",
        &[("cont.txt", "a\\\nb\nc\n")],
        "0:[ab]\n[c]\n",
    );
}

#[test]
fn dash_r_stops_at_the_first_newline() {
    assert_read(
        "{ read -r x; echo \"$?:[$x]\"; read -r y; echo \"[$y]\"; } < cont.txt",
        &[("cont.txt", "a\\\nb\nc\n")],
        "0:[a\\]\n[b]\n",
    );
}

#[test]
fn a_trailing_backslash_at_eof_is_dropped() {
    assert_read(
        "read x < dangling.txt; echo \"$?:[$x]\"",
        &[("dangling.txt", "a\\")],
        "1:[a]\n",
    );
}

#[test]
fn double_dash_ends_the_options() {
    assert_read(
        "read -- v < three.txt; echo \"[$v]\"",
        &[THREE_LINES],
        "[one]\n",
    );
}

#[test]
fn read_consumes_exactly_one_line_and_no_more() {
    // Buffered reads would swallow the rest of the file here and leave `cat` with nothing.
    assert_read(
        "{ read x; echo \"first=$x\"; cat; } < three.txt",
        &[THREE_LINES],
        "first=one\ntwo\nthree\n",
    );
}

#[test]
fn read_from_a_pipe_leaves_the_tail_for_the_next_command() {
    assert_read(
        "printf 'l1\\nl2\\nl3\\n' | { read x; echo \"got=$x\"; cat; }",
        &[],
        "got=l1\nl2\nl3\n",
    );
}
