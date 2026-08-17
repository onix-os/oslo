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
    let expected = ["one", "2", "two"];
    let lines: Vec<&str> = out
        .lines()
        .filter_map(|line| expected.iter().find(|want| line.ends_with(**want)).copied())
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
    let expected = ["lua one", "shell", "lua two"];
    let lines: Vec<&str> = out
        .lines()
        .filter_map(|line| expected.iter().find(|want| line.ends_with(**want)).copied())
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

/// `oslo.proc.exit` from the prompt ends the shell with the status it names.
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
        .write_all(b"oslo.proc.exit(7)\nprint('not reached')\n")
        .expect("write");
    let output = child.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(7));
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("not reached"),
        "the shell kept reading after oslo.proc.exit"
    );
}

/// History expansion is shell syntax, and rewriting a Lua line against the history would corrupt
/// it — a `!` in Lua is half of `~=` and can appear inside any string.
#[test]
fn a_lua_line_is_not_rewritten_by_history_expansion() {
    let out = typed("print('a!b')\n", &[("OSLO_DEFAULT_MODE", "lua")]);
    assert!(out.contains("a!b"), "{out}");
}

#[test]
fn a_precmd_hook_sees_each_command_as_typed() {
    let out = typed(
        "=oslo.on.precmd(function(c) print('PRE ' .. c.text .. ' in ' .. c.cwd) end)\necho hi\n",
        &[],
    );
    assert!(out.contains("PRE echo hi in /"), "{out}");
    // The command still runs; a hook observes, it does not intercept.
    assert!(out.contains("\nhi"), "{out}");
}

/// The status, and the rest of what a hook needs to report a command: how long it took, where it
/// ended, and whether it worked. The status alone was all this used to be handed.
#[test]
fn a_postcmd_hook_is_handed_the_status() {
    let out = typed(
        "=oslo.on.postcmd(function(c) print('POST ' .. c.text .. ' ' .. c.status \
         .. ' ' .. tostring(c.ok) .. ' ' .. type(c.duration_ms)) end)\nfalse\n",
        &[],
    );
    assert!(out.contains("POST false 1 false number"), "{out}");
}

/// **A command that failed still fires the hook.** It used to fire only on success, which left the
/// hook silent for exactly the commands one is usually installed to notice.
#[test]
fn a_postcmd_hook_fires_for_a_command_that_failed() {
    let out = typed(
        "=oslo.on.postcmd(function(c) print('POST ' .. c.status) end)\nno-such-command-anywhere\n",
        &[],
    );
    assert!(out.contains("POST 127"), "{out}");
}

#[test]
fn a_cd_hook_fires_only_when_the_directory_changed() {
    let out = typed(
        "=oslo.on.cd(function(d) print('CD ' .. d.to .. ' from ' .. d.from) end)\n\
         echo not a cd\ncd /tmp\n",
        &[],
    );
    assert!(out.contains("CD /tmp"), "{out}");
    assert_eq!(out.matches("CD ").count(), 1, "{out}");
}

/// A handler written inline still has to be removable, which is the whole reason `oslo.on.*`
/// returns a handle rather than taking a name.
#[test]
fn a_hook_handle_removes_the_handler_it_stands_for() {
    let out = typed(
        "=h = oslo.on.precmd(function(c) print('PRE ' .. c.text) end)\necho one\n=h:remove()\necho two\n",
        &[],
    );
    assert!(out.contains("PRE echo one"), "{out}");
    assert!(!out.contains("PRE echo two"), "{out}");
}

/// One broken hook must not disable the others, or silently stop the command from running.
#[test]
fn a_failing_hook_is_reported_and_the_rest_still_run() {
    let out = typed(
        "=oslo.on.precmd(function() error('broken') end)\n\
         =oslo.on.precmd(function() print('SECOND') end)\n\
         echo ran\n",
        &[],
    );
    assert!(out.contains("broken"), "{out}");
    assert!(out.contains("SECOND"), "{out}");
    assert!(out.contains("\nran"), "{out}");
}

/// **The Lua prompt evaluates expressions and shows what they came to.**
///
/// It could not. A Lua chunk is a sequence of *statements*, so every one of `1 + 1`, `x * 2`,
/// `os.time`, `"a string"` and `{1,2}` was a syntax error — "expression is not a statement" — and
/// the one thing a prompt is mostly used for was the one thing it refused. Every Lua REPL solves
/// it the same way: try the line as the tail of a `return` first.
#[test]
fn the_lua_prompt_evaluates_an_expression_and_prints_it() {
    let lua = &[("OSLO_DEFAULT_MODE", "lua")];
    for (typed_line, wanted) in [
        ("1 + 1", "2"),
        ("2 ^ 10", "1024.0"),
        ("\"a \" .. \"string\"", "a string"),
        ("#(\"abc\")", "3"),
        ("math.max(3, 9)", "9"),
        ("(\"x\"):rep(3)", "xxx"),
    ] {
        let out = typed(&format!("{typed_line}\n"), lua);
        assert!(
            out.lines().any(|line| line.trim() == wanted),
            "`{typed_line}` should print {wanted:?}, printed {out:?}"
        );
    }
}

/// Several values print the way Lua prints several values, and none prints nothing.
#[test]
fn the_lua_prompt_shows_every_value_and_stays_quiet_for_none() {
    let lua = &[("OSLO_DEFAULT_MODE", "lua")];

    let several = typed("1, 2, 3\n", lua);
    assert!(
        several.lines().any(|line| line.trim() == "1\t2\t3"),
        "{several}"
    );

    // A statement is not an expression and must not grow an answer: `x = 5` prints nothing, and
    // `print` already printed, so wrapping it must not add a `nil` under what it said.
    let assigned = typed("x = 5\nx * 2\n", lua);
    assert!(
        assigned.lines().any(|line| line.trim() == "10"),
        "{assigned}"
    );
    assert!(
        !assigned.lines().any(|line| line.trim() == "nil"),
        "an assignment answered something: {assigned}"
    );

    // `print` already printed, so wrapping the line must not put a `nil` under what it said.
    let printed = typed("print(\"said\")\n", lua);
    assert_eq!(
        printed.lines().filter(|line| line.trim() == "said").count(),
        1,
        "print said it more than once: {printed}"
    );
    assert!(
        !printed.lines().any(|line| line.trim() == "nil"),
        "print grew an answer: {printed}"
    );
}

/// `nil` and `false` are answers, not silence — a prompt that hid them would be lying about what
/// the expression came to.
#[test]
fn the_lua_prompt_shows_a_falsey_answer() {
    let lua = &[("OSLO_DEFAULT_MODE", "lua")];
    for (typed_line, wanted) in [("nil", "nil"), ("false", "false"), ("1 == 2", "false")] {
        let out = typed(&format!("{typed_line}\n"), lua);
        assert!(
            out.lines().any(|line| line.trim() == wanted),
            "`{typed_line}` should print {wanted:?}: {out}"
        );
    }
}

/// **`exit` leaves a Lua prompt**, which is the word every shell answers and Lua has no meaning
/// for. As an expression it is an unset global, so the prompt printed `nil` and stayed open —
/// there was no way out that a shell user would guess.
#[test]
fn exit_leaves_the_lua_prompt() {
    let lua = &[("OSLO_DEFAULT_MODE", "lua")];
    let out = typed("exit\nprint(\"AFTER\")\n", lua);
    assert!(
        !out.contains("AFTER"),
        "the shell kept reading after exit: {out}"
    );
    assert!(!out.contains("nil"), "exit answered a value: {out}");
}

/// A line that is a whole statement still runs as one, and a failure still says where.
#[test]
fn a_statement_still_runs_and_a_failure_still_says_where() {
    let lua = &[("OSLO_DEFAULT_MODE", "lua")];
    let out = typed("local t = {} ; t.x = 1 ; print(t.x)\n", lua);
    assert!(out.lines().any(|line| line.trim() == "1"), "{out}");

    // One label and one position, where this used to read
    // `Lua error: (oslo lua): runtime error: (oslo lua):1: …`.
    let failed = typed("nosuchfn_zz()\n", lua);
    assert!(failed.contains("(oslo lua):1:"), "{failed}");
    assert!(
        !failed.contains("runtime error:"),
        "the VM's own category leaked: {failed}"
    );
    assert_eq!(
        failed.matches("(oslo lua)").count(),
        1,
        "the chunk was named twice: {failed}"
    );
}
