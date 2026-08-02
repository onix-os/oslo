//! Startup, configuration and history (PLAN R9.9, R9.10, R9.11, R9.12).
//!
//! The interactive parts are reachable without a pty because `oslo -i` forces the REPL and
//! rustyline falls back to reading lines directly when stdin is not a terminal — and with
//! `TERM=dumb` it writes the prompt to stdout, which is the only way to observe `PS1` and `PS2`
//! from outside. Everything here therefore drives the real binary the way a user does, rather
//! than asserting on internals that could be right while the shell is wrong.

mod common;

use common::oslo_bin;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Run the binary against a throwaway `$HOME`.
///
/// `XDG_CONFIG_HOME` goes with it: it outranks `$HOME/.config` when the shell looks for its
/// config, so an ambient one — GitHub's runners export it, as do most desktops — points the
/// shell at the machine's real config and the temporary home is never consulted at all.
fn run(args: &[&str], vars: &[(&str, &str)], home: &Path) -> Output {
    let mut cmd = Command::new(oslo_bin());
    cmd.args(args)
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("ENV")
        .env_remove("HISTFILE")
        .env_remove("HISTSIZE")
        .env_remove("PS1")
        .env_remove("PS2")
        .stdin(Stdio::null());
    for (k, v) in vars {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn oslo")
}

/// Drive the REPL: `-i` forces it, and stdin is a pipe rather than a terminal.
fn repl(input: &str, vars: &[(&str, &str)], home: &Path) -> Output {
    let mut cmd = Command::new(oslo_bin());
    cmd.arg("-i")
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
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
    let mut child = cmd.spawn().expect("spawn oslo -i");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write to oslo");
    child.wait_with_output().expect("oslo output")
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

/// `~/.oslorc` is **Lua** — it used to be shell syntax, and that is the change.
#[test]
fn oslorc_is_read_by_an_interactive_shell_only() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".oslorc"), "MARK = 'from-oslorc'\n").unwrap();

    let interactive = repl("echo $MARK\n", &[("HISTFILE", "")], dir.path());
    assert!(
        out(&interactive).contains("from-oslorc"),
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
fn an_alias_from_oslorc_is_visible_at_the_prompt() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".oslorc"),
        "oslo.set_alias('hi', 'echo aliased')\n",
    )
    .unwrap();

    let o = repl("hi\n", &[("HISTFILE", "")], dir.path());
    assert!(out(&o).contains("aliased"), "{:?}", out(&o));
}

/// An existing shell-syntax `.oslorc` fails loudly rather than half-working. That is the whole
/// migration story: a Lua syntax error names the line, where a file that silently applied its
/// first two commands and dropped the rest would be far harder to notice.
#[test]
fn a_shell_syntax_oslorc_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".oslorc"), "alias hi='echo aliased'\n").unwrap();

    let o = repl("echo alive\n", &[("HISTFILE", "")], dir.path());
    assert!(
        out(&o).contains("alive"),
        "the shell must still start: {:?}",
        out(&o)
    );
    let diagnostic = err(&o);
    assert!(
        diagnostic.contains(".oslorc"),
        "the diagnostic must name the file: {diagnostic:?}"
    );
}

/// The XDG location, which is the other name for the same file.
#[test]
fn a_config_under_xdg_is_read_too() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join(".config/oslo/config.lua");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "oslo.set_alias('hi', 'echo xdg-alias')\n").unwrap();

    let o = repl("hi\n", &[("HISTFILE", "")], dir.path());
    assert!(out(&o).contains("xdg-alias"), "{:?}", out(&o));
}

#[test]
fn a_broken_oslorc_reports_and_leaves_the_shell_usable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".oslorc"), "this is not lua(((\n").unwrap();

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
    assert!(!dir.path().join(".oslo_history").exists());
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
fn a_broken_config_is_reported_and_the_shell_still_starts() {
    let dir = tempfile::tempdir().unwrap();
    let init = dir.path().join(".config/oslo/config.lua");
    std::fs::create_dir_all(init.parent().unwrap()).unwrap();
    std::fs::write(&init, "this is not lua(((\n").unwrap();

    let o = repl("echo alive\n", &[("HISTFILE", "")], dir.path());
    assert!(
        out(&o).contains("alive"),
        "a broken config must not stop the shell: {:?}",
        out(&o)
    );
    let diagnostic = err(&o);
    assert!(
        diagnostic.contains("config.lua"),
        "the diagnostic must name the user's file: {diagnostic:?}"
    );
}

#[test]
fn a_working_config_still_applies() {
    let dir = tempfile::tempdir().unwrap();
    let init = dir.path().join(".config/oslo/config.lua");
    std::fs::create_dir_all(init.parent().unwrap()).unwrap();
    std::fs::write(&init, "oslo.set_alias('hi', 'echo lua-alias')\n").unwrap();

    let o = repl("hi\n", &[("HISTFILE", "")], dir.path());
    assert!(out(&o).contains("lua-alias"), "{:?}", out(&o));
}

/// The headline of the change: which language a script is written in is worked out, not declared.
///
/// One test per level of evidence, because each is a separate decision and a regression in any
/// one of them is invisible in the others.
#[test]
fn a_scripts_language_is_detected_rather_than_flagged() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, body: &str| {
        let p = dir.path().join(name);
        std::fs::write(&p, body).unwrap();
        p.to_str().unwrap().to_string()
    };

    // By extension.
    let lua = write("a.lua", "print('lua by extension')\n");
    assert!(out(&run(&[&lua], &[], dir.path())).contains("lua by extension"));

    let sh = write("a.sh", "echo 'shell by extension'\n");
    assert!(out(&run(&[&sh], &[], dir.path())).contains("shell by extension"));

    // By shebang, with no extension to help — and the shebang must not reach Lua's parser.
    let shebang = write("noext", "#!/usr/bin/env lua\nprint('lua by shebang')\n");
    let o = run(&[&shebang], &[], dir.path());
    assert!(out(&o).contains("lua by shebang"), "{:?}", err(&o));

    // By shebang, outranking a misleading extension.
    let mislabelled = write(
        "b.sh",
        "#!/usr/bin/env lua\nprint('shebang beats extension')\n",
    );
    let o = run(&[&mislabelled], &[], dir.path());
    assert!(out(&o).contains("shebang beats extension"), "{:?}", err(&o));

    // By syntax, with neither a shebang nor an extension.
    let sniffed = write("plain", "local t = {1, 2}\nprint('lua by syntax ' .. #t)\n");
    let o = run(&[&sniffed], &[], dir.path());
    assert!(out(&o).contains("lua by syntax 2"), "{:?}", err(&o));
}

/// `--lua` and `--sh` override every signal, for the file that genuinely cannot be told apart.
#[test]
fn the_language_can_be_forced_against_every_other_signal() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("looks_like_shell.sh");
    std::fs::write(&p, "print('forced to lua')\n").unwrap();
    let o = run(&["--lua", p.to_str().unwrap()], &[], dir.path());
    assert!(out(&o).contains("forced to lua"), "{:?}", err(&o));

    let p = dir.path().join("looks_like_lua.lua");
    std::fs::write(&p, "echo 'forced to shell'\n").unwrap();
    let o = run(&["--sh", p.to_str().unwrap()], &[], dir.path());
    assert!(out(&o).contains("forced to shell"), "{:?}", err(&o));
}

/// `-c` stays shell whatever the text looks like: every `sh -c` idiom depends on it, and the
/// differential corpus is 390 scripts' worth of that assumption.
#[test]
fn dash_c_is_shell_even_when_the_text_could_be_lua() {
    let dir = tempfile::tempdir().unwrap();
    // Valid Lua that prints `hi`. Read as shell the parentheses are a syntax error — which is
    // exactly what bash reports for the same string, also with status 2.
    let o = run(&["-c", "print('hi')"], &[], dir.path());
    assert_eq!(o.status.code(), Some(2), "stderr: {:?}", err(&o));
    assert!(!out(&o).contains("hi"), "-c ran as Lua: {:?}", out(&o));

    // And `--lua` is the way to ask for the other reading.
    let o = run(&["--lua", "-c", "print('hi')"], &[], dir.path());
    assert!(out(&o).contains("hi"), "{:?}", err(&o));
}
