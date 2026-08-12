//! The Lua corpus: every case in `tests/lua_corpus/`, run through the real binary.
//!
//! The shell corpus in `tests/corpus/` compares against bash, which is why it can be trusted
//! without anyone reading it: the expected output comes from somewhere other than the code under
//! test. There is no second Lua shell to play that role here, so the oracle is a **recorded**
//! expectation living in the case itself.
//!
//! That difference is the whole risk, and it decides how these are written:
//!
//! * Expectations are **written by hand and reviewed**, never captured from a run. Generating them
//!   would record whatever the shell does today — bugs included — and then assert it forever,
//!   which is worse than having no test, because it looks like coverage.
//! * A case with no `--[[ expect ]]` block **fails**. A case that asserts nothing would otherwise
//!   sit in the corpus looking like it covers something.
//!
//! Format: an optional `-- status: N` header (default 0), the program, and a trailing block:
//!
//! ```lua
//! -- status: 3
//! print("hi")
//! --[[ expect
//! hi
//! ]]
//! ```
//!
//! Only stdout and the exit status are compared. stderr is checked for *shape* — a case that
//! expects a diagnostic says so with `-- stderr: yes` — for the same reason the shell corpus does
//! it: exact wording is not an interface.

mod common;

use common::oslo_bin;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Case {
    name: String,
    /// The `.lua` source, expectation block included — it is a Lua comment, so it runs fine.
    source: String,
    expected_stdout: String,
    expected_status: i32,
    expects_stderr: bool,
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lua_corpus")
}

fn load() -> Vec<Case> {
    let dir = corpus_dir();
    let mut cases: Vec<Case> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "lua"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let source = fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {name}: {e}"));
            let expected_stdout = expect_block(&name, &source);
            Case {
                expected_status: header_number(&source, "-- status:").unwrap_or(0),
                expects_stderr: source.contains("-- stderr: yes"),
                name,
                source,
                expected_stdout,
            }
        })
        .collect();
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}

/// The text between `--[[ expect` and the closing `]]`, which is what the case must print.
fn expect_block(name: &str, source: &str) -> String {
    let Some(start) = source.find("--[[ expect\n") else {
        panic!(
            "{name}: no `--[[ expect` block. A case must say what it expects, or it asserts \
             nothing while looking like coverage."
        );
    };
    let body = &source[start + "--[[ expect\n".len()..];
    let Some(end) = body.rfind("\n]]") else {
        panic!("{name}: `--[[ expect` block is not closed with `]]`");
    };
    let text = &body[..end];
    if text.is_empty() {
        String::new()
    } else {
        format!("{text}\n")
    }
}

fn header_number(source: &str, key: &str) -> Option<i32> {
    source
        .lines()
        .find_map(|l| l.trim().strip_prefix(key))
        .and_then(|v| v.trim().parse().ok())
}

#[test]
fn lua_corpus_matches_its_recorded_expectations() {
    let cases = load();
    assert!(
        cases.len() >= 8,
        "the Lua corpus is supposed to be substantial; found {}",
        cases.len()
    );

    let mut failures = Vec::new();
    for case in &cases {
        let dir = tempfile::tempdir().expect("tempdir");
        // The script is written into the scratch directory and named there, so a case can use
        // `arg[0]` and relative paths without depending on where the suite was run from.
        let script = dir.path().join(&case.name);
        fs::write(&script, &case.source).expect("write case");

        let output = Command::new(oslo_bin())
            .arg(&script)
            .arg("first-arg")
            .arg("second arg")
            .current_dir(dir.path())
            .env("HOME", dir.path())
            .env_remove("ENV")
            // **`HOME` alone does not isolate a case.** Anything resolving an XDG path prefers
            // `$XDG_DATA_HOME`, which is inherited and points at the *developer's* real one — so a
            // case that opened a database would write into their plugin directory rather than into
            // the temporary home two lines up.
            .env_remove("XDG_DATA_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_CACHE_HOME")
            .stdin(Stdio::null())
            .output()
            .expect("spawn oslo");

        let stdout = String::from_utf8_lossy(&output.stdout)
            .replace(script.to_str().unwrap(), "<SCRIPT>")
            .replace(dir.path().to_str().unwrap(), "<TMP>");
        let status = output.status.code().unwrap_or(-1);
        let stderr_seen = !output.stderr.is_empty();

        if stdout != case.expected_stdout {
            failures.push(format!(
                "  {}\n    stdout differs:\n{}",
                case.name,
                diff(&case.expected_stdout, &stdout)
            ));
        }
        if status != case.expected_status {
            failures.push(format!(
                "  {}\n    status: expected {}, got {}\n",
                case.name, case.expected_status, status
            ));
        }
        if stderr_seen != case.expects_stderr {
            failures.push(format!(
                "  {}\n    stderr: expected {}, got {} — {:?}\n",
                case.name,
                shape(case.expects_stderr),
                shape(stderr_seen),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "\n{} Lua corpus case(s) differ from what they record:\n\n{}\n\
         (expectations here are written by hand, so a difference means one of the two is wrong — \
         decide which before changing either.)\n",
        failures.len(),
        failures.join("\n")
    );

    eprintln!("lua corpus: {} cases matching", cases.len());
}

fn shape(present: bool) -> &'static str {
    if present { "non-empty" } else { "empty" }
}

fn diff(expected: &str, got: &str) -> String {
    let mut out = String::new();
    let (e, g): (Vec<&str>, Vec<&str>) = (expected.lines().collect(), got.lines().collect());
    for i in 0..e.len().max(g.len()) {
        let (a, b) = (e.get(i), g.get(i));
        if a != b {
            out.push_str(&format!(
                "      line {}: expected {:?}, got {:?}\n",
                i + 1,
                a.unwrap_or(&"<none>"),
                b.unwrap_or(&"<none>")
            ));
        }
    }
    out
}

/// Every case has to be reachable as a *program*, not only as text this harness reads.
///
/// The detection rules decide whether `oslo case.lua` runs as Lua at all, and a corpus that
/// bypassed them by forcing `--lua` would have passed happily while `#!/usr/bin/env oslo` sent
/// every real script to the shell parser — which is exactly the bug that shipped.
#[test]
fn every_case_is_detected_as_lua_without_being_told() {
    for case in load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join(&case.name);
        fs::write(&script, &case.source).expect("write case");
        let output = Command::new(oslo_bin())
            .arg(&script)
            .current_dir(dir.path())
            .stdin(Stdio::null())
            .output()
            .expect("spawn oslo");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("Syntax error"),
            "{} was read as shell, not Lua: {stderr}",
            case.name
        );
    }
}
