//! The prompt's two languages, driven through the real binary.
//!
//! `-i` forces the interactive loop even though stdin is a pipe, which is what lets these run
//! without a pty. Two things are out of reach that way and are *not* covered here: the toggle
//! key, because no key is ever pressed down a pipe, and the prompt string, because rustyline
//! does not draw one for a terminal it considers unsupported. Both need the pty harness in
//! `scripts/alpine-vm-*.sh`.
//!
//! What is pinned here is everything else: which language a line is read as, the one-line
//! escapes, per-mode completeness, and how a failure in each is reported.

mod common;

use common::oslo_bin;
use std::io::Write;
use std::process::{Command, Stdio};

/// Type `input` at an interactive prompt and return everything it printed.
#[track_caller]
fn typed(input: &str, env: &[(&str, &str)]) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut child = Command::new(oslo_bin())
        .arg("-i")
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("ENV")
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn oslo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write");
    let output = child.wait_with_output().expect("wait");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

#[test]
fn a_session_starts_in_shell_mode() {
    let out = typed("echo hello\n", &[]);
    assert!(out.contains("hello"), "{out}");
}

#[test]
fn the_equals_prefix_runs_one_lua_line_from_shell_mode() {
    let out = typed("echo one\n=print(1 + 1)\necho two\n", &[]);
    // The prefix does not change the mode: the shell line after it is still shell.
    let lines: Vec<&str> = out
        .lines()
        .filter(|l| *l == "one" || *l == "2" || *l == "two")
        .collect();
    assert_eq!(lines, vec!["one", "2", "two"], "{out}");
}

#[test]
fn the_default_mode_is_configurable() {
    let out = typed("print('from lua')\n", &[("OSLO_DEFAULT_MODE", "lua")]);
    assert!(out.contains("from lua"), "{out}");
}

#[test]
fn the_bang_prefix_runs_one_shell_line_from_lua_mode() {
    let out = typed(
        "print('lua one')\n!echo shell\nprint('lua two')\n",
        &[("OSLO_DEFAULT_MODE", "lua")],
    );
    let lines: Vec<&str> = out
        .lines()
        .filter(|l| ["lua one", "shell", "lua two"].contains(l))
        .collect();
    assert_eq!(lines, vec!["lua one", "shell", "lua two"], "{out}");
}

/// The two languages share one namespace, so a variable set in one is readable in the other on
/// the next line. That is the whole reason switching mid-session is useful.
#[test]
fn a_variable_crosses_between_the_modes() {
    let out = typed("export greeting=hello\n=print(greeting)\n", &[]);
    assert!(out.contains("hello"), "{out}");

    let back = typed("=name = 'world'\necho $name\n", &[]);
    assert!(back.contains("world"), "{back}");
}

/// An unfinished Lua chunk asks for another line rather than reporting a syntax error.
#[test]
fn lua_mode_continues_an_unfinished_chunk() {
    let out = typed(
        "if true then\n  print('inside')\nend\n",
        &[("OSLO_DEFAULT_MODE", "lua")],
    );
    assert!(out.contains("inside"), "{out}");
}

/// And a genuine mistake is reported instead of wedging the prompt waiting for more.
#[test]
fn a_lua_syntax_error_comes_back_rather_than_hanging() {
    let out = typed(
        "x = = 2\necho still here\n",
        &[("OSLO_DEFAULT_MODE", "lua")],
    );
    assert!(out.contains("syntax error"), "{out}");
}

#[test]
fn the_current_mode_is_published_for_the_prompt_to_read() {
    let out = typed("echo mode is $OSLO_MODE\n", &[]);
    assert!(out.contains("mode is sh"), "{out}");

    let lua = typed(
        "!echo mode is $OSLO_MODE\n",
        &[("OSLO_DEFAULT_MODE", "lua")],
    );
    assert!(lua.contains("mode is lua"), "{lua}");
}

/// A Lua line that fails must not take the shell down — an interactive shell survives what would
/// end a script.
#[test]
fn a_failing_lua_line_leaves_the_prompt_up() {
    let out = typed(
        "error('boom')\nprint('still here')\n",
        &[("OSLO_DEFAULT_MODE", "lua")],
    );
    assert!(out.contains("boom"), "{out}");
    assert!(out.contains("still here"), "{out}");
}

/// `oslo.exit` from the prompt ends the shell with the status it names.
#[test]
fn oslo_exit_from_lua_mode_ends_the_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut child = Command::new(oslo_bin())
        .arg("-i")
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("OSLO_DEFAULT_MODE", "lua")
        .env_remove("ENV")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn oslo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"oslo.exit(7)\nprint('not reached')\n")
        .expect("write");
    let output = child.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(7));
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("not reached"),
        "the shell kept reading after oslo.exit"
    );
}

/// History expansion is shell syntax, and rewriting a Lua line against the history would corrupt
/// it — a `!` in Lua is half of `~=` and can appear inside any string.
#[test]
fn a_lua_line_is_not_rewritten_by_history_expansion() {
    let out = typed("print('a!b')\n", &[("OSLO_DEFAULT_MODE", "lua")]);
    assert!(out.contains("a!b"), "{out}");
}
