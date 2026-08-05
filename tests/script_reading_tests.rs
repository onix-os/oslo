//! How a program's text reaches the evaluator.
//!
//! A file and a piped stream are read **a command at a time**, as every shell reads them: the
//! lines before a syntax error run, and a `#!/bin/sh` polyglot reaches its `exec` before the other
//! language below it is ever parsed. A `-c` string is parsed whole, because bash and dash both do.
//!
//! Everything here drives the real binary, since the distinction is only observable from outside.

mod common;

use common::oslo_bin;
use std::path::Path;
use std::process::{Command, Output};

fn run(args: &[&str], env: &[(&str, &str)], cwd: &Path) -> Output {
    let mut cmd = Command::new(oslo_bin());
    cmd.args(args).current_dir(cwd).env("TERM", "dumb");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("run oslo")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

/// **A script runs up to its syntax error, not instead of it.** bash, dash and busybox all print
/// what came before; oslo used to parse the whole file first and print nothing at all.
#[test]
fn a_script_runs_the_lines_before_a_syntax_error() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("half.sh");
    std::fs::write(&p, "echo BEFORE\nif true; then\n  echo inside\n").unwrap();
    let o = run(&[p.to_str().unwrap()], &[], dir.path());
    assert!(out(&o).contains("BEFORE"), "stdout: {:?}", out(&o));
    assert!(
        err(&o).to_lowercase().contains("syntax"),
        "the error is still reported: {:?}",
        err(&o)
    );
}

/// **A `#!/bin/sh` polyglot works.** `/usr/bin/pnmflip` and nine netpbm siblings are Perl below an
/// `exec perl -x -S -- "$0"`; reading a command at a time reaches the `exec` and the process is
/// replaced before the Perl is looked at. Parsing the file first was a syntax error in Perl that
/// stopped the shell part from ever running.
#[test]
fn a_polyglot_execs_away_before_its_other_language_is_parsed() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("poly.sh");
    // `exec cat` stands in for `exec perl`: the trailing text is not shell, and must never be
    // parsed as any.
    std::fs::write(
        &p,
        "#!/bin/sh\necho SHELL-PART\nexec echo replaced\nsub f($) { this is not shell ((( }\n",
    )
    .unwrap();
    let o = run(&[p.to_str().unwrap()], &[], dir.path());
    assert!(out(&o).contains("SHELL-PART"), "{:?}", err(&o));
    assert!(out(&o).contains("replaced"), "{:?}", err(&o));
    assert!(o.status.success(), "exited {:?}: {:?}", o.status, err(&o));
}

/// **A `-c` command is not recorded unless `$OSLO_ALLHIST` asks.**
///
/// The default is what makes oslo safe as `/bin/sh`: `sh -c` is how every `system()` call, git
/// hook, build tool and cron line runs, and recording those would bury what a person typed.
#[test]
fn a_dash_c_command_is_not_recorded_by_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let hist = dir.path().join("hist");
    run(
        &["-c", "echo one"],
        &[
            ("HISTFILE", hist.to_str().expect("path")),
            ("XDG_DATA_HOME", dir.path().to_str().expect("path")),
        ],
        dir.path(),
    );
    assert!(!hist.exists(), "a -c command was recorded without asking");
    assert!(
        !dir.path().join("oslo").exists(),
        "a tracking store was opened without asking"
    );
}

/// With it set, the command lands in **both** stores: `$HISTFILE` for the Up arrow, and the
/// tracking store for the finder. A command you ran is a command you ran.
#[test]
fn allhist_records_a_dash_c_command_in_both_stores() {
    let dir = tempfile::tempdir().expect("tempdir");
    let hist = dir.path().join("hist");
    let vars = [
        ("HISTFILE", hist.to_str().expect("path")),
        ("XDG_DATA_HOME", dir.path().to_str().expect("path")),
        ("OSLO_ALLHIST", "1"),
    ];
    run(&["-c", "echo recorded"], &vars, dir.path());

    let text = std::fs::read_to_string(&hist).expect("the history file");
    assert!(text.contains("echo recorded"), "$HISTFILE: {text:?}");
    assert!(
        dir.path().join("oslo").join("default.kv").exists(),
        "the finder's store was not written"
    );

    // A leading space keeps a command out, exactly as it does at a prompt — that is how a secret
    // on a command line stays out of the log.
    run(&["-c", " echo private"], &vars, dir.path());
    let text = std::fs::read_to_string(&hist).expect("the history file");
    assert!(
        !text.contains("private"),
        "a space-prefixed command was recorded: {text:?}"
    );
}

/// A *script* is never recorded, however `$OSLO_ALLHIST` is set: its contents are not commands
/// anybody typed, and every `#!/bin/sh` script on the machine would otherwise land in the history.
#[test]
fn a_script_is_never_recorded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let hist = dir.path().join("hist");
    let script = dir.path().join("s.sh");
    std::fs::write(&script, "echo from-a-script\n").expect("write");
    run(
        &[script.to_str().expect("path")],
        &[
            ("HISTFILE", hist.to_str().expect("path")),
            ("XDG_DATA_HOME", dir.path().to_str().expect("path")),
            ("OSLO_ALLHIST", "1"),
        ],
        dir.path(),
    );
    assert!(!hist.exists(), "a script's contents were recorded");
}

/// `-c` is *not* streamed: bash and dash both parse a `-c` string whole, so
/// `sh -c 'echo RAN; if true; then'` prints nothing in either.
#[test]
fn dash_c_is_parsed_whole() {
    let dir = tempfile::tempdir().unwrap();
    let o = run(&["-c", "echo RAN; if true; then"], &[], dir.path());
    assert!(
        !out(&o).contains("RAN"),
        "-c ran part of a program that does not parse: {:?}",
        out(&o)
    );
}
