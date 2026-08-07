//! `\command` and `\\command`, at a prompt and in a script.
//!
//! | written | alias | function | builtin | runs |
//! |---|---|---|---|---|
//! | `cmd` | expanded | used | used | the shell's |
//! | `\cmd` | skipped | skipped | skipped | the program on `$PATH` |
//! | `\\cmd` | expanded | used | skipped | the alias's target, unbuiltin |
//!
//! The reason to want it: `rm` is a builtin here, and a shell whose `rm` moves things to `/tmp`
//! needs a one-keystroke way to get the `rm` that does not. `command rm` is not it — `command`
//! bypasses *functions*, and the builtin still wins.
//!
//! **The half that matters most is the script half.** oslo is meant to be `/bin/sh`, so `\ls` in a
//! POSIX script has to go on meaning what POSIX says: suppress the alias, then ordinary command
//! search. Changing that would silently reinterpret every escaped command word already written on
//! the machine, with nothing to notice. Every case below is run twice for that reason.

mod common;

use common::oslo_bin;
use std::process::{Command, Output, Stdio};

/// Run `script` with the shell told it is interactive, or not.
///
/// `-i` sets the option `env.interactive()` reads, which is the gate on all of this — the same one
/// `=command` and `rm`'s extensions use. No pty is involved: the flag is what is being tested, not
/// the line editor.
fn run(script: &str, interactive: bool) -> (String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut command = Command::new(oslo_bin());
    if interactive {
        command.arg("-i");
    }
    let output: Output = command
        .arg("-c")
        .arg(script)
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    (
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned(),
        String::from_utf8_lossy(&output.stderr)
            .trim_end()
            .to_owned(),
    )
}

/// A program named `probe` on `$PATH` that says so, plus a function and an alias of the same name.
///
/// Three things called one name is the only way to tell which step of the command search answered.
/// The directory is kept alive by the caller, since dropping it deletes the program.
fn three_of_a_name(dir: &std::path::Path) -> String {
    let bin = dir.join("probe");
    std::fs::write(&bin, "#!/bin/sh\necho PROGRAM\n").expect("write probe");
    #[allow(clippy::permissions_set_readonly_false)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    format!("PATH={}:$PATH\n", dir.display())
}

/// Run `script` with a `probe` program on `$PATH`, in both modes.
fn probed(body: &str, interactive: bool) -> (String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let prelude = three_of_a_name(dir.path());
    let script = format!("{prelude}{body}");
    let mut command = Command::new(oslo_bin());
    if interactive {
        command.arg("-i");
    }
    let output: Output = command
        .arg("-c")
        .arg(&script)
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    (
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned(),
        String::from_utf8_lossy(&output.stderr)
            .trim_end()
            .to_owned(),
    )
}

/// `\cmd` reaches past a function to the program on `$PATH`.
///
/// A function is the sharpest test of the "everything" half: `command probe` would still call the
/// function's *builtin* peers, and quoting the name would not skip anything at all.
#[test]
fn a_single_backslash_reaches_the_program() {
    let body = "probe() { echo FUNCTION; }\nprobe\n\\probe\n";
    let (out, err) = probed(body, true);
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        vec!["FUNCTION", "PROGRAM"],
        "stderr: {err}"
    );
}

/// `\\cmd` keeps the function and the alias — it only refuses the builtin.
///
/// The narrow form, for `alias rm='rm -i'`, where the alias is the whole point and the builtin is
/// not. Nothing about a function is being escaped past.
#[test]
fn a_double_backslash_keeps_the_function() {
    let body = "probe() { echo FUNCTION; }\n\\\\probe\n";
    let (out, err) = probed(body, true);
    assert_eq!(out, "FUNCTION", "stderr: {err}");
}

/// `\\cmd` expands the alias and then declines the builtin the alias named.
///
/// `echo` is the pair to test it with: there is a builtin and a `/usr/bin/echo`, and they disagree
/// about `--help` — the builtin prints it as an argument, coreutils prints usage.
#[test]
fn a_double_backslash_expands_the_alias_and_skips_the_builtin() {
    let script = "alias e=\"echo\"\ne --help\n\\\\e --help\n";
    let (out, err) = run(script, true);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines.first(),
        Some(&"--help"),
        "the builtin echo prints its argument; stderr: {err}"
    );
    assert!(
        lines.iter().skip(1).any(|line| line.starts_with("Usage:")),
        "\\\\e should have reached /usr/bin/echo, got {lines:?}; stderr: {err}"
    );
}

/// `\cmd` skips the alias, which is the one thing it already did and must go on doing.
#[test]
fn a_single_backslash_still_skips_the_alias() {
    let script = "alias e=\"echo aliased\"\n\\e nothing-here\n";
    let (_, err) = run(script, true);
    assert!(
        err.contains("e: command not found"),
        "the alias must not have expanded, got stderr: {err}"
    );
}

/// **A script gets POSIX and nothing else.** `\cmd` suppresses the alias and then finds the
/// function, exactly as bash and dash do.
///
/// This is the case that decides whether oslo can be `/bin/sh`. If it ever fails, the gate in
/// `exec::simple::escape` has gone.
#[test]
fn a_script_keeps_the_posix_reading() {
    let body = "probe() { echo FUNCTION; }\n\\probe\n";
    let (out, err) = probed(body, false);
    assert_eq!(
        out, "FUNCTION",
        "a script's \\cmd must still find the function; stderr: {err}"
    );
}

/// And a script's `\\cmd` is a command whose name begins with a backslash — not found, as in bash.
#[test]
fn a_script_reads_a_double_backslash_as_a_literal_name() {
    let body = "probe() { echo FUNCTION; }\n\\\\probe\n";
    let (out, err) = probed(body, false);
    assert_eq!(out, "", "nothing should have run: {out:?}");
    assert!(
        err.contains("command not found"),
        "expected a not-found diagnostic, got: {err}"
    );
}

/// Quoting a command name is not escaping it.
///
/// `"echo"` runs the builtin in bash, dash and zsh, and dispatch tables written as `"$cmd" "$@"`
/// depend on it. A shell that read quoting as "skip the builtin" would send every one of them to
/// `$PATH`.
#[test]
fn quoting_the_command_name_still_finds_the_builtin() {
    let (out, err) = run("\"echo\" --help\n'echo' --help\n", true);
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        vec!["--help", "--help"],
        "quoting must not skip the builtin; stderr: {err}"
    );
}

/// An escape that is not at the front of the word means nothing at all: `r\m` is `rm`.
#[test]
fn an_escape_inside_the_word_is_an_ordinary_name() {
    let (out, err) = run("e\\cho --help\n", true);
    assert_eq!(out, "--help", "e\\cho is the builtin echo; stderr: {err}");
}
