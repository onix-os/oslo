//! Startup, configuration and history (PLAN R9.9, R9.10, R9.11, R9.12).
//!
//! The interactive parts are reachable without a pty because `rush -i` forces the REPL and
//! rustyline falls back to reading lines directly when stdin is not a terminal — and with
//! `TERM=dumb` it writes the prompt to stdout, which is the only way to observe `PS1` and `PS2`
//! from outside. Everything here therefore drives the real binary the way a user does, rather
//! than asserting on internals that could be right while the shell is wrong.

mod common;

use common::rush_bin;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run(args: &[&str], vars: &[(&str, &str)], home: &Path) -> Output {
    let mut cmd = Command::new(rush_bin());
    cmd.args(args)
        .env("HOME", home)
        .env_remove("ENV")
        .env_remove("HISTFILE")
        .env_remove("HISTSIZE")
        .env_remove("PS1")
        .env_remove("PS2")
        .stdin(Stdio::null());
    for (k, v) in vars {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn rush")
}

/// Drive the REPL: `-i` forces it, and stdin is a pipe rather than a terminal.
fn repl(input: &str, vars: &[(&str, &str)], home: &Path) -> Output {
    let mut cmd = Command::new(rush_bin());
    cmd.arg("-i")
        .env("HOME", home)
        .env_remove("ENV")
        .env_remove("PS1")
        .env_remove("PS2")
        .env_remove("IGNOREEOF")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in vars {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn rush -i");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write to rush");
    child.wait_with_output().expect("rush output")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn bash(args: &[&str]) -> String {
    let o = Command::new("bash")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("spawn bash");
    String::from_utf8_lossy(&o.stdout).into_owned()
}

// ---------------------------------------------------------------- R9.12: `$0` and positionals

#[test]
fn dash_c_operands_match_bash() {
    let dir = tempfile::tempdir().unwrap();
    let script = "echo \"$0|$#|$1|$2|$*\"";
    let ours = run(&["-c", script, "myname", "a", "b"], &[], dir.path());
    let theirs = bash(&["-c", script, "myname", "a", "b"]);
    assert_eq!(out(&ours), theirs);
    assert_eq!(out(&ours).trim_end(), "myname|2|a|b|a b");
}

#[test]
fn a_script_gets_its_path_as_dollar_zero_like_bash() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("s.sh");
    std::fs::write(&script, "echo \"$0|$#|$1\"\n").unwrap();
    let path = script.to_str().unwrap();

    let ours = run(&[path, "one"], &[], dir.path());
    assert_eq!(out(&ours), bash(&[path, "one"]));
    assert_eq!(out(&ours).trim_end(), format!("{path}|1|one"));
}

#[test]
fn dash_c_without_operands_leaves_no_positionals() {
    let dir = tempfile::tempdir().unwrap();
    let ours = run(&["-c", "echo \"[$#][$1]\""], &[], dir.path());
    assert_eq!(out(&ours).trim_end(), "[0][]");
}

#[test]
fn the_find_exec_idiom_matches_bash() {
    // `find … -exec sh -c 'cmd "$@"' -- {} +`: the `--` is `$0`, not an end-of-options marker.
    let dir = tempfile::tempdir().unwrap();
    let script = "echo \"$0|$1|$2\"";
    let ours = run(&["-c", script, "--", "x", "y"], &[], dir.path());
    assert_eq!(out(&ours), bash(&["-c", script, "--", "x", "y"]));
    assert_eq!(out(&ours).trim_end(), "--|x|y");
}

// ------------------------------------------------------------------ R9.10: rc files, PS1, PS2

#[test]
fn env_is_read_by_a_non_interactive_shell() {
    let dir = tempfile::tempdir().unwrap();
    let rc = dir.path().join("shrc");
    std::fs::write(&rc, "GREET=hello\ngreet() { echo \"$GREET $1\"; }\n").unwrap();

    let o = run(
        &["-c", "greet world"],
        &[("ENV", rc.to_str().unwrap())],
        dir.path(),
    );
    assert_eq!(out(&o).trim_end(), "hello world");
}

#[test]
fn env_is_expanded_before_it_is_read() {
    // POSIX defines `$ENV` as a word that is expanded, so `ENV=$HOME/.shrc` has to work.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".shrc"), "MARK=expanded\n").unwrap();

    let o = run(&["-c", "echo $MARK"], &[("ENV", "$HOME/.shrc")], dir.path());
    assert_eq!(out(&o).trim_end(), "expanded");
}

#[test]
fn rushrc_is_read_by_an_interactive_shell_only() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".rushrc"), "MARK=from-rushrc\n").unwrap();

    let interactive = repl("echo $MARK\n", &[("HISTFILE", "")], dir.path());
    assert!(
        out(&interactive).contains("from-rushrc"),
        "{:?}",
        out(&interactive)
    );

    let batch = run(&["-c", "echo [$MARK]"], &[], dir.path());
    assert_eq!(
        out(&batch).trim_end(),
        "[]",
        "a script must not inherit the interactive rc file"
    );
}

#[test]
fn an_alias_from_rushrc_is_visible_at_the_prompt() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".rushrc"), "alias hi='echo aliased'\n").unwrap();

    let o = repl("hi\n", &[("HISTFILE", "")], dir.path());
    assert!(out(&o).contains("aliased"), "{:?}", out(&o));
}

#[test]
fn a_broken_rushrc_reports_and_leaves_the_shell_usable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".rushrc"), "if then fi\nMARK=late\n").unwrap();

    let o = repl("echo alive\n", &[("HISTFILE", "")], dir.path());
    assert!(out(&o).contains("alive"), "{:?}", out(&o));
    assert!(!err(&o).is_empty(), "a broken rc file must say so");
}

#[test]
fn ps1_and_ps2_are_honoured_with_defaults_as_fallback() {
    let dir = tempfile::tempdir().unwrap();
    // TERM=dumb makes rustyline print the prompt to stdout, which is the only way to see it
    // without a pty.
    let vars = [("HISTFILE", ""), ("TERM", "dumb")];

    let set = repl(
        "PS1='A> '\nPS2='B> '\nfor i in 1\ndo\necho x\ndone\n",
        &vars,
        dir.path(),
    );
    let text = out(&set);
    assert!(text.contains("A> "), "PS1 was ignored: {text:?}");
    assert!(
        text.contains("B> B> "),
        "PS2 was ignored on the continuation lines: {text:?}"
    );

    let unset = repl("for i in 1\ndo\necho x\ndone\n", &vars, dir.path());
    let text = out(&unset);
    assert!(
        text.contains("❯"),
        "the built-in prompt is the PS1 fallback: {text:?}"
    );
    assert!(text.contains("> > "), "`> ` is the PS2 fallback: {text:?}");
}

#[test]
fn ps1_is_expanded_every_time_it_is_shown() {
    let dir = tempfile::tempdir().unwrap();
    let o = repl(
        "PS1='[$N]'\nN=1\nN=2\n",
        &[("HISTFILE", ""), ("TERM", "dumb")],
        dir.path(),
    );
    let text = out(&o);
    assert!(text.contains("[1]"), "{text:?}");
    assert!(text.contains("[2]"), "{text:?}");
}

#[test]
fn an_unfinished_command_continues_instead_of_failing() {
    // R9.10's other half: PS2 only means something if there is a continuation to show it for.
    let dir = tempfile::tempdir().unwrap();
    let o = repl(
        "for i in 1 2\ndo\necho n=$i\ndone\n",
        &[("HISTFILE", "")],
        dir.path(),
    );
    assert!(out(&o).contains("n=1\nn=2"), "{:?}", out(&o));
    assert!(!err(&o).contains("Syntax error"), "{:?}", err(&o));
}

#[test]
fn a_here_document_body_keeps_its_indentation() {
    // Continuation lines are data, not commands: trimming them the way the first line is trimmed
    // would silently rewrite the file the user is writing.
    let dir = tempfile::tempdir().unwrap();
    let o = repl(
        "cat <<EOF\n    indented\nEOF\n",
        &[("HISTFILE", "")],
        dir.path(),
    );
    assert!(out(&o).contains("\n    indented\n"), "{:?}", out(&o));
}

#[test]
fn a_multi_line_command_is_one_history_entry() {
    let dir = tempfile::tempdir().unwrap();
    let hist = dir.path().join("hist");
    repl(
        "for i in 1\ndo\n:\ndone\n",
        &[("HISTFILE", hist.to_str().unwrap())],
        dir.path(),
    );
    let text = std::fs::read_to_string(&hist).unwrap();
    // rustyline escapes the newlines inside one entry, so the whole command is a single line.
    assert!(text.contains("for i in 1\\ndo\\n:\\ndone"), "{text:?}");
}

#[test]
fn ignoreeof_keeps_the_shell_alive_on_end_of_input() {
    let dir = tempfile::tempdir().unwrap();
    let o = repl("IGNOREEOF=1\n", &[("HISTFILE", "")], dir.path());
    assert!(
        out(&o).contains("Use \"exit\" to leave the shell."),
        "{:?}",
        out(&o)
    );
}

// ------------------------------------------------------------------------- R9.11: the history

fn history_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[test]
fn history_keeps_more_than_rustylines_default_of_a_hundred() {
    let dir = tempfile::tempdir().unwrap();
    let hist = dir.path().join("hist");
    let input: String = (1..=120).map(|i| format!(": {i}\n")).collect();

    repl(&input, &[("HISTFILE", hist.to_str().unwrap())], dir.path());
    assert_eq!(history_lines(&hist).len(), 120);
}

#[test]
fn histsize_caps_the_history() {
    let dir = tempfile::tempdir().unwrap();
    let hist = dir.path().join("hist");
    repl(
        ": a\n: b\n: c\n: d\n",
        &[("HISTFILE", hist.to_str().unwrap()), ("HISTSIZE", "2")],
        dir.path(),
    );
    assert_eq!(
        history_lines(&hist),
        vec![": c".to_string(), ": d".to_string()]
    );
}

#[test]
fn a_leading_space_keeps_a_command_out_of_the_history() {
    let dir = tempfile::tempdir().unwrap();
    let hist = dir.path().join("hist");
    repl(
        "echo one\n : secret\necho two\n",
        &[("HISTFILE", hist.to_str().unwrap())],
        dir.path(),
    );
    assert_eq!(
        history_lines(&hist),
        vec!["echo one".to_string(), "echo two".to_string()],
        "the leading-space convention must be honoured exactly once"
    );
}

#[test]
fn a_command_is_stored_once_not_twice() {
    let dir = tempfile::tempdir().unwrap();
    let hist = dir.path().join("hist");
    repl(
        "  echo hi  \n",
        &[("HISTFILE", hist.to_str().unwrap())],
        dir.path(),
    );
    // Leading whitespace also means "do not remember", so the file stays empty — what must not
    // happen is the line appearing twice.
    assert!(
        history_lines(&hist).is_empty(),
        "{:?}",
        history_lines(&hist)
    );

    let hist2 = dir.path().join("hist2");
    repl(
        "echo hi\n",
        &[("HISTFILE", hist2.to_str().unwrap())],
        dir.path(),
    );
    assert_eq!(history_lines(&hist2), vec!["echo hi".to_string()]);
}

#[test]
fn a_concurrent_sessions_entries_are_not_clobbered() {
    // The session appends to its own history file while it is running, standing in for a second
    // shell open at the same time. Rewriting the whole file on exit would lose that line.
    let dir = tempfile::tempdir().unwrap();
    let hist = dir.path().join("hist");
    let hist_str = hist.to_str().unwrap().to_string();

    repl(
        &format!("echo first\nprintf 'echo elsewhere\\n' >> {hist_str}\necho second\n"),
        &[("HISTFILE", &hist_str)],
        dir.path(),
    );

    let lines = history_lines(&hist);
    assert!(
        lines.contains(&"echo elsewhere".to_string()),
        "another session's entry was lost: {lines:?}"
    );
    assert!(lines.contains(&"echo second".to_string()), "{lines:?}");
}

#[test]
fn an_empty_histfile_disables_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let o = repl("echo hi\n", &[("HISTFILE", "")], dir.path());
    assert!(out(&o).contains("hi"));
    assert!(!dir.path().join(".rush_history").exists());
}

#[test]
fn the_history_builtin_numbers_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let o = repl(
        "echo one\necho two\nhistory\n",
        &[("HISTFILE", "")],
        dir.path(),
    );
    let text = out(&o);
    assert!(text.contains("    1  echo one"), "{text:?}");
    assert!(text.contains("    2  echo two"), "{text:?}");
    assert!(text.contains("    3  history"), "{text:?}");
}

#[test]
fn history_takes_a_count_and_minus_c() {
    let dir = tempfile::tempdir().unwrap();
    let o = repl(
        "echo one\necho two\nhistory 1\nhistory -c\nhistory\necho end\n",
        &[("HISTFILE", "")],
        dir.path(),
    );
    let text = out(&o);
    assert!(!text.contains("  1  echo one\n    2"), "{text:?}");
    // After `history -c` the only entry left is the `history` that reported it.
    assert!(text.contains("    1  history\n"), "{text:?}");
    assert!(text.contains("end"), "{text:?}");
}

#[test]
fn history_is_a_builtin_even_in_a_script() {
    let dir = tempfile::tempdir().unwrap();
    let o = run(&["-c", "type history; history; echo ok"], &[], dir.path());
    assert!(out(&o).contains("shell builtin"), "{:?}", out(&o));
    assert!(out(&o).contains("ok"));
    assert_eq!(o.status.code(), Some(0));
}

// ----------------------------------------------------------------------------- R9.9: the Lua layer

#[test]
fn a_broken_init_lua_is_reported_and_the_shell_still_starts() {
    let dir = tempfile::tempdir().unwrap();
    let init = dir.path().join(".config/rush/init.lua");
    std::fs::create_dir_all(init.parent().unwrap()).unwrap();
    std::fs::write(&init, "this is not lua(((\n").unwrap();

    let o = repl("echo alive\n", &[("HISTFILE", "")], dir.path());
    assert!(
        out(&o).contains("alive"),
        "a broken init.lua must not stop the shell: {:?}",
        out(&o)
    );
    let diagnostic = err(&o);
    assert!(
        diagnostic.contains("init.lua"),
        "the diagnostic must name the user's file: {diagnostic:?}"
    );
}

#[test]
fn a_working_init_lua_still_applies() {
    let dir = tempfile::tempdir().unwrap();
    let init = dir.path().join(".config/rush/init.lua");
    std::fs::create_dir_all(init.parent().unwrap()).unwrap();
    std::fs::write(&init, "rush.set_alias('hi', 'echo lua-alias')\n").unwrap();

    let o = repl("hi\n", &[("HISTFILE", "")], dir.path());
    assert!(out(&o).contains("lua-alias"), "{:?}", out(&o));
}

#[test]
fn lua_script_propagates_the_status_of_what_it_ran() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("s.lua");

    std::fs::write(&script, "rush.exec('false')\n").unwrap();
    let failed = run(&["--lua-script", script.to_str().unwrap()], &[], dir.path());
    assert_eq!(
        failed.status.code(),
        Some(1),
        "a failing rush.exec must show"
    );

    std::fs::write(&script, "rush.exec('true')\n").unwrap();
    let ok = run(&["--lua-script", script.to_str().unwrap()], &[], dir.path());
    assert_eq!(ok.status.code(), Some(0));

    std::fs::write(&script, "rush.exec('exit 7')\n").unwrap();
    let exited = run(&["--lua-script", script.to_str().unwrap()], &[], dir.path());
    assert_ne!(
        exited.status.code(),
        Some(0),
        "a script that asked to exit non-zero must not report success"
    );
}

#[test]
fn a_broken_lua_script_exits_non_zero_and_names_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("broken.lua");
    std::fs::write(&script, "not lua(((\n").unwrap();

    let o = run(&["--lua-script", script.to_str().unwrap()], &[], dir.path());
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("broken.lua"), "{:?}", err(&o));
}
