//! Every builtin says where it is speaking from.
//!
//! # What this exists to catch
//!
//! The error-location work taught the shell to prefix a diagnostic with `script.sh: line 4: `
//! instead of `oslo: `, and wired it into `cd` and the executor. Forty other builtins kept a
//! hardcoded `oslo: `, so a script that failed on `read`, `printf`, `ulimit` or `trap` named the
//! shell and not the line — the exact complaint the work was written to answer, still true almost
//! everywhere.
//!
//! There is nothing to notice when this regresses: `oslo: read: -z: invalid option` is a
//! perfectly ordinary-looking message. It is only wrong next to the one bash prints.
//!
//! # The two rules
//!
//! 1. **No builtin hardcodes the prefix.** `crate::env::origin_now()` answers it, and answers
//!    `oslo: ` itself when there is no file to name — so the hardcoded form is never the only way
//!    to get the old behaviour, just the way to lose the new one.
//! 2. **A script gets the file and the line.** Checked by running one, because rule 1 is satisfied
//!    by a builtin nothing ever dispatches through `exec_custom_builtin`.

mod common;

use common::oslo_bin;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Every `.rs` file the builtins are made of, except the test modules beside them.
fn builtin_sources() -> Vec<(PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/oslo-shell/src/env/builtins");
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}")) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && !path.ends_with("tests.rs")
                && path.file_name().is_some_and(|n| n != "tests.rs")
            {
                let text =
                    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
                found.push((path, text));
            }
        }
    }
    assert!(
        found.len() > 30,
        "the builtin walk found almost nothing — has the directory moved?"
    );
    found
}

/// **Rule 1.** No builtin writes the prefix itself.
#[test]
fn no_builtin_hardcodes_the_shells_own_name() {
    let mut offenders = Vec::new();
    for (path, text) in builtin_sources() {
        for (number, line) in text.lines().enumerate() {
            if line.contains(r#"eprintln!("oslo: "#) || line.contains(r#"eprint!("oslo: "#) {
                offenders.push(format!("{}:{}", path.display(), number + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these print `oslo: ` directly instead of `origin_now()`, so a script that hits them \
         is told the shell's name rather than its own file and line: {offenders:#?}"
    );
}

/// Run `script` as a file and answer what reached stderr, with the temporary path shortened.
#[track_caller]
fn in_a_file(script: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("case.sh");
    std::fs::write(&path, script).expect("write script");
    let out = Command::new(oslo_bin())
        .arg(&path)
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    String::from_utf8_lossy(&out.stderr)
        .replace(&path.to_string_lossy().to_string(), "case.sh")
        .trim_end()
        .to_string()
}

/// **Rule 2**, one builtin per line, each on a line whose number is its own.
///
/// The failing operand is chosen so the diagnostic comes from the builtin rather than from the
/// executor around it — every one of these was `oslo: ` before.
#[test]
fn a_builtin_that_fails_in_a_script_names_the_file_and_the_line() {
    let cases: &[(&str, &str)] = &[
        ("rm /nonexistent-zz/x", "rm:"),
        ("cd /nonexistent-zz", "cd:"),
        ("ulimit -Z", "ulimit:"),
        ("read -z", "read:"),
        ("kill -NOSUCHSIG 1", "kill:"),
        ("hash -z", "hash:"),
        ("trap -z EXIT", "trap:"),
        ("printf -Z", "printf:"),
        ("times -Z", "times:"),
    ];

    for (number, (line, builtin)) in cases.iter().enumerate() {
        // A leading blank line per case, so the reported line number is not 1 for all of them —
        // a prefix built from a constant would pass otherwise.
        let padding = "\n".repeat(number);
        let seen = in_a_file(&format!("{padding}{line}\n"));
        let wanted = format!("case.sh: line {}: {builtin}", number + 1);
        assert!(
            seen.starts_with(&wanted),
            "`{line}` should begin {wanted:?}, said {seen:?}"
        );
    }
}

/// At a prompt and under `-c` the old prefix is still the right one: there is no file to name, and
/// `oslo: ` is what a reader already associates with the shell talking about itself.
#[test]
fn dash_c_keeps_the_shells_own_name() {
    let out = Command::new(oslo_bin())
        .arg("-c")
        .arg("ulimit -Z")
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    let seen = String::from_utf8_lossy(&out.stderr).trim_end().to_string();
    assert!(seen.starts_with("oslo: ulimit:"), "{seen}");
}

/// A sourced file names *itself*, and its own line, from inside a builtin's diagnostic.
#[test]
fn a_sourced_file_names_itself_in_a_builtins_diagnostic() {
    let dir = tempfile::tempdir().expect("tempdir");
    let inner = dir.path().join("inner.sh");
    std::fs::write(&inner, "echo inner\nulimit -Z\n").expect("write inner");
    let outer = dir.path().join("outer.sh");
    std::fs::write(&outer, format!("echo outer\n. {}\n", inner.display())).expect("write outer");

    let out = Command::new(oslo_bin())
        .arg(&outer)
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    let seen = String::from_utf8_lossy(&out.stderr)
        .replace(&inner.to_string_lossy().to_string(), "inner.sh")
        .trim_end()
        .to_string();
    assert!(seen.starts_with("inner.sh: line 2: ulimit:"), "{seen}");
}
