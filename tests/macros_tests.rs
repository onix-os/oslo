//! `oslo macros` through the real binary — the parts that are about where a name resolves.

mod common;

use common::oslo_bin;
use std::process::Command;

/// Run oslo with a store and a script directory of its own.
fn oslo(dirs: (&std::path::Path, &std::path::Path), args: &[&str], path_first: bool) -> String {
    let mut command = Command::new(oslo_bin());
    command
        .args(args)
        .env("XDG_DATA_HOME", dirs.0)
        .env("OSLO_MACROS_BIN", dirs.1)
        .stdin(std::process::Stdio::null());
    if path_first {
        let path = std::env::var("PATH").unwrap_or_default();
        command.env("PATH", format!("{}:{path}", dirs.1.display()));
    }
    let out = command.output().expect("spawn oslo");
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn store(dirs: (&std::path::Path, &std::path::Path), text: &str) {
    use std::io::Write;
    let mut child = Command::new(oslo_bin())
        .args(["macros", "import"])
        .env("XDG_DATA_HOME", dirs.0)
        .env("OSLO_MACROS_BIN", dirs.1)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("spawn oslo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(text.as_bytes())
        .expect("write");
    child.wait().expect("wait");
}

/// **A stored macro never beats a real program**, even with oslo's own copy of it early on `$PATH`.
///
/// This failed once, and quietly: the resolver skipped oslo's copy by *rejecting the answer* rather
/// than by leaving the directory out of the search, so the search ended there and a stored `date`
/// ran instead of `/usr/bin/date`.
#[test]
fn a_stored_script_does_not_shadow_a_real_program() {
    let data = tempfile::tempdir().expect("tempdir");
    let bin = tempfile::tempdir().expect("tempdir");
    let dirs = (data.path(), bin.path());
    store(dirs, "script date\n\t#!/bin/sh\n\techo I-AM-THE-MACRO\n");

    let out = oslo(dirs, &["-c", "date"], true);
    assert!(
        !out.contains("I-AM-THE-MACRO"),
        "the macro shadowed /usr/bin/date: {out:?}"
    );
}

/// A name no program answers to still reaches the database — from the database, not from the copy.
#[test]
fn a_stored_script_runs_when_nothing_else_answers() {
    let data = tempfile::tempdir().expect("tempdir");
    let bin = tempfile::tempdir().expect("tempdir");
    let dirs = (data.path(), bin.path());
    store(
        dirs,
        "script oslo-macro-probe\n\t#!/bin/sh\n\techo ran-the-macro\n",
    );

    assert_eq!(
        oslo(dirs, &["-c", "oslo-macro-probe"], true).trim(),
        "ran-the-macro"
    );
    // With the copies nowhere near `$PATH` it still runs, which is what makes the directory
    // optional for somebody who only uses oslo.
    assert_eq!(
        oslo(dirs, &["-c", "oslo-macro-probe"], false).trim(),
        "ran-the-macro"
    );
}

/// The copy is written for everything that is not oslo, and it is what bash finds.
#[test]
fn the_copy_is_what_another_shell_runs() {
    let data = tempfile::tempdir().expect("tempdir");
    let bin = tempfile::tempdir().expect("tempdir");
    let dirs = (data.path(), bin.path());
    store(
        dirs,
        "script greet-from-file\n\t#!/bin/sh\n\techo from-the-file\n",
    );

    let script = bin.path().join("greet-from-file");
    assert!(script.exists(), "no copy was written");

    let out = Command::new("sh")
        .arg("-c")
        .arg("greet-from-file")
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.path().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .expect("sh");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "from-the-file");
}
