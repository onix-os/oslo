//! Aliases, exit status, prefix assignments and variable scoping.
//!
//! Spawns the real binary; see `common/mod.rs` for why.

mod common;

use oslo::Environment;

use common::{assert_out, run};

// --- aliases ---

#[test]
fn multi_word_alias_expands_to_multiple_arguments() {
    assert_out("alias e='echo one two'\ne", "one two");
}

#[test]
fn alias_arguments_are_appended() {
    assert_out("alias e='echo prefix'\ne suffix", "prefix suffix");
}

#[test]
fn alias_body_keeps_quoted_grouping() {
    assert_out("alias e='echo \"a  b\"'\ne", "a  b");
}

/// A definition and its use on the *same* line: bash answers `e: command not found`, because an
/// alias is not available until the next command is read. These three tests used to be written
/// this way and passed only because oslo expanded aliases after parsing.
#[test]
fn an_alias_is_not_available_on_the_line_that_defines_it() {
    let r = run("alias e='echo hi'; e");
    assert_ne!(r.status, 0, "stdout: {}", r.stdout);
    assert!(r.stderr.contains("not found"), "stderr: {}", r.stderr);
}

/// The reason alias substitution moved ahead of the parser: an alias body is source text, and may
/// open a construct the word it replaced could not.
#[test]
fn an_alias_may_open_a_compound_command() {
    assert_out(
        "alias forever='while :; do'\nn=0\nforever\n  n=$((n+1))\n  [ $n -ge 3 ] && break\ndone\necho $n",
        "3",
    );
}

#[test]
fn builtin_default_aliases_work() {
    // `ll` ships as `ls -la`; expanded as a single argv[0] it would be "command not found".
    let r = run("ll");
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
}

// --- exit status ---

#[test]
fn exit_status_reflects_the_last_command() {
    assert_eq!(run("true").status, 0);
    assert_eq!(run("false").status, 1);
    assert_eq!(run("true; false").status, 1);
    assert_eq!(run("false; true").status, 0);
}

#[test]
fn unknown_command_exits_127() {
    assert_eq!(run("nosuchcommand_xyz").status, 127);
}

#[test]
fn explicit_exit_code_is_honoured() {
    assert_eq!(run("exit 7").status, 7);
}

#[test]
fn function_return_becomes_the_shell_status() {
    assert_eq!(run("f() { return 3; }; f").status, 3);
}

#[test]
fn pipeline_status_is_that_of_the_last_stage() {
    assert_eq!(run("false | true").status, 0);
    assert_eq!(run("true | false").status, 1);
}

#[test]
fn negated_pipeline_inverts_the_status() {
    assert_eq!(run("! false").status, 0);
    assert_eq!(run("! true").status, 1);
}

#[test]
fn status_is_readable_as_a_parameter() {
    assert_out("false; echo $?", "1");
    assert_out("true; echo $?", "0");
}

// --- prefix assignments ---

#[test]
fn prefix_assignment_reaches_the_child_environment() {
    assert_out(r#"FOO=bar sh -c 'echo $FOO'"#, "bar");
}

#[test]
fn prefix_assignment_does_not_outlive_the_command() {
    assert_out("FOO=bar true; echo \"after=[$FOO]\"", "after=[]");
}

#[test]
fn prefix_assignment_restores_a_previous_value() {
    assert_out(
        "FOO=orig; FOO=temp true; echo \"after=[$FOO]\"",
        "after=[orig]",
    );
}

#[test]
fn prefix_assignment_does_not_leak_into_later_children() {
    // The temporary export must be removed from the process environment afterwards, not merely
    // from the shell's own variable table.
    assert_out(
        r#"FOO=orig; FOO=temp true; sh -c 'echo child=[$FOO]'"#,
        "child=[]",
    );
}

#[test]
fn prefix_assignment_keeps_quoted_values_intact() {
    assert_out(r#"FOO="a b" sh -c 'echo [$FOO]'"#, "[a b]");
}

// --- scoping regressions ---

#[test]
fn local_shadows_and_restores() {
    assert_out(
        "V=outer; f() { local V=inner; echo $V; }; f; echo $V",
        "inner\nouter",
    );
}

#[test]
fn local_without_a_value_still_scopes() {
    assert_out(
        "f() { local Z; Z=set; echo $Z; }; f; echo \"after=[$Z]\"",
        "set\nafter=[]",
    );
}

#[test]
fn readonly_is_enforced() {
    let r = run("readonly R=1; R=2; echo $R");
    assert_eq!(r.out(), "1");
}

// --- `export NAME` with no value: marked for export, still unset (PLAN.md round C) ---

/// `export V` with no value marks `V` for export and leaves it **unset**.
///
/// bash gives an empty `${V+set}` and no `V=` in a child's environment; oslo used to create it
/// empty, so every `${V+set}` answered "set" and every child saw a spurious `V=`. A later
/// assignment must still be exported, which is the whole reason the intention is recorded.
#[test]
fn exporting_a_name_that_does_not_exist_yet_does_not_create_it() {
    let mut env = Environment::new();
    assert!(env.export_var("PENDING_ONE"));
    assert!(env.get_var("PENDING_ONE").is_none(), "must still be unset");

    // Assigning later honours the pending export.
    assert!(env.set_var("PENDING_ONE", "1", false));
    assert_eq!(env.get_var("PENDING_ONE"), Some("1"));
    assert!(env.exported_vars().iter().any(|(n, _)| n == "PENDING_ONE"));
}

/// `unset` undoes the intention, or `export V; unset V; V=1` would still export.
#[test]
fn unsetting_forgets_a_pending_export() {
    let mut env = Environment::new();
    assert!(env.export_var("PENDING_TWO"));
    env.unset_var("PENDING_TWO");
    assert!(env.set_var("PENDING_TWO", "9", false));
    assert!(!env.exported_vars().iter().any(|(n, _)| n == "PENDING_TWO"));
}
