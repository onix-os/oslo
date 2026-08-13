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

/// **`type` and `command -v` answer for a stored macro**, because they exist to answer "what would
/// run?" and it runs.
///
/// Both were wrong once, in opposite directions: `command -v` said nothing at all, while `type`
/// printed the path of oslo's own copy — a file dispatch skips. Anything that probes with
/// `command -v foo` before calling `foo` got the wrong answer for every stored script.
#[test]
fn what_would_run_is_what_type_reports() {
    let data = tempfile::tempdir().expect("tempdir");
    let bin = tempfile::tempdir().expect("tempdir");
    let dirs = (data.path(), bin.path());
    store(dirs, "script oslo-probe\n\t#!/bin/sh\n\techo hi\n");

    // The word `command -v` prints is one this shell can run, as it is for a function.
    assert_eq!(
        oslo(dirs, &["-c", "command -v oslo-probe"], true).trim(),
        "oslo-probe"
    );
    assert_eq!(
        oslo(dirs, &["-c", "command -V oslo-probe"], true).trim(),
        "oslo-probe is a stored script"
    );
    assert_eq!(
        oslo(dirs, &["-c", "type oslo-probe"], true).trim(),
        "oslo-probe is a stored script"
    );
    // `-t` is read by scripts, and it behaves as a file does.
    assert_eq!(
        oslo(dirs, &["-c", "type -t oslo-probe"], true).trim(),
        "file"
    );
    // The probe every configure script writes.
    assert_eq!(
        oslo(
            dirs,
            &["-c", "command -v oslo-probe >/dev/null && echo yes"],
            true
        )
        .trim(),
        "yes"
    );
}

/// Type `line` at an interactive oslo and collect what it prints.
fn at_a_prompt(dirs: (&std::path::Path, &std::path::Path), line: &str) -> String {
    use std::io::Write;
    let mut command = Command::new(oslo_bin());
    command
        .arg("-i")
        .env("XDG_DATA_HOME", dirs.0)
        // Not the machine's own config: a `oslo.alias` in it would answer some of these.
        .env("XDG_CONFIG_HOME", dirs.0)
        .env("OSLO_MACROS_BIN", dirs.1)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = command.spawn().expect("spawn oslo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(line.as_bytes())
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// **`which` and `whereis` are builtins so that they can answer this at all.** The programs by
/// those names read `$PATH`, and a stored macro is a database row dispatch reaches after it.
#[test]
fn which_and_whereis_know_about_a_stored_macro() {
    let data = tempfile::tempdir().expect("tempdir");
    let bin = tempfile::tempdir().expect("tempdir");
    let dirs = (data.path(), bin.path());
    store(dirs, "script oslo-probe\n\t#!/bin/sh\n\techo hi\n");

    let out = at_a_prompt(dirs, "which oslo-probe\n");
    assert!(out.contains("oslo-probe: stored script"), "{out:?}");
    // **One entry per thing.** The copy `oslo macros` writes for other shells is the same macro
    // written down twice, so it is not a second place this name lives — even with the directory
    // right there on `$PATH`. The `export` is typed rather than inherited because a config of the
    // machine's own may rebuild `$PATH` at startup.
    let out = at_a_prompt(
        dirs,
        &format!(
            "export PATH={}:$PATH\nwhereis oslo-probe\n",
            bin.path().display()
        ),
    );
    assert!(out.contains("oslo-probe: stored script"), "{out:?}");
    assert!(
        !out.contains(&format!("{}/oslo-probe", bin.path().display())),
        "the generated copy was reported as a second place: {out:?}"
    );
    // A builtin is the case no program can answer.
    let out = at_a_prompt(dirs, "which cd\n");
    assert!(out.contains("cd: shell built-in command"), "{out:?}");
}

/// **A script gets the program**, because `which` is not POSIX and `$(which echo)` in somebody's
/// configure script has to stay a path. `command -v` is the one that answers about this shell.
#[test]
fn a_script_is_answered_by_the_program() {
    let data = tempfile::tempdir().expect("tempdir");
    let bin = tempfile::tempdir().expect("tempdir");
    let dirs = (data.path(), bin.path());

    let out = oslo(dirs, &["-c", "which echo"], false);
    // Nothing to assert if the machine has no `which` at all — then the builtin answers, which is
    // the documented fallback.
    if which::which("which").is_ok() {
        assert!(out.trim().ends_with("/echo"), "not a path: {out:?}");
    }
}

/// One turned off is not there, for `type` as for dispatch.
#[test]
fn one_turned_off_is_reported_by_neither() {
    let data = tempfile::tempdir().expect("tempdir");
    let bin = tempfile::tempdir().expect("tempdir");
    let dirs = (data.path(), bin.path());
    store(dirs, "script oslo-probe\n\t#!/bin/sh\n\techo hi\n");
    oslo(dirs, &["macros", "off", "oslo-probe"], false);

    assert_eq!(
        oslo(dirs, &["-c", "command -v oslo-probe"], false).trim(),
        ""
    );
    assert_eq!(oslo(dirs, &["-c", "type -t oslo-probe"], false).trim(), "");
}

/// **A file somebody put in that directory by hand is theirs**, and runs like a file anywhere else.
///
/// oslo passes over the copies it wrote, and for a while it did that by taking the whole directory
/// out of `$PATH` — which made a hand-written script there run from bash, from tmux and from a
/// `.desktop` entry, and be "command not found" in the one shell that owns the directory. The
/// manifest says which files are oslo's; nothing else in there is.
#[test]
fn a_file_oslo_never_wrote_is_not_oslos_to_hide() {
    let data = tempfile::tempdir().expect("tempdir");
    let bin = tempfile::tempdir().expect("tempdir");
    let dirs = (data.path(), bin.path());
    store(dirs, "script oslo-probe\n\t#!/bin/sh\n\techo hi\n");

    let by_hand = bin.path().join("oslo-hand-written");
    std::fs::write(&by_hand, "#!/bin/sh\necho by-hand\n").expect("write");
    let mut mode = std::fs::metadata(&by_hand).expect("stat").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    std::fs::set_permissions(&by_hand, mode).expect("chmod");

    assert_eq!(
        oslo(dirs, &["-c", "oslo-hand-written"], true).trim(),
        "by-hand"
    );
    assert_eq!(
        oslo(dirs, &["-c", "command -v oslo-hand-written"], true).trim(),
        by_hand.display().to_string()
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
