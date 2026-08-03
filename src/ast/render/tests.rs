//! What `$BASH_COMMAND` reads as, for the shapes a preexec hook actually sees.
//!
//! Every expectation here was taken from bash rather than reasoned about, with
//! `trap 'echo "[$BASH_COMMAND]"' DEBUG`. Where bash normalises — the space after a redirection
//! operator is the visible one — the normalised form is what is asserted, because a hook matching
//! on this text has to see what bash's hooks see.

use super::*;
use crate::parser::parse_bash_script;

/// Render the first simple command in `source`.
fn render(source: &str) -> String {
    let script = parse_bash_script(source).expect("parses");
    let mut found = None;
    for item in &script.items {
        for command in &item.and_or.first.commands {
            if let Command::Simple(simple) = command {
                found = Some(simple.clone());
            }
        }
    }
    simple_command(&found.expect("a simple command"))
}

#[test]
fn a_plain_command_renders_as_itself() {
    assert_eq!(render("echo hello"), "echo hello");
    assert_eq!(render("ls -la /tmp"), "ls -la /tmp");
}

/// An assignment is a command in its own right and fires the trap on its own.
#[test]
fn assignments_render_before_the_words() {
    assert_eq!(render("x=world"), "x=world");
    assert_eq!(render("x=1 y=2 env"), "x=1 y=2 env");
    assert_eq!(render("PATH=$PATH:/x cmd"), "PATH=$PATH:/x cmd");
}

/// bash writes a space after the operator that the script did not. Matching that is the point:
/// a hook comparing against bash's output would otherwise miss.
#[test]
fn a_redirection_gets_bashs_space() {
    assert_eq!(render("ls /x 2>/dev/null"), "ls /x 2> /dev/null");
    assert_eq!(render("cat <in >out"), "cat < in > out");
    assert_eq!(render("echo hi >>log"), "echo hi >> log");
}

/// Quoting is part of the word, not decoration: a hook that greps for `'` has to find it.
#[test]
fn quoting_survives() {
    assert_eq!(render("echo 'single'"), "echo 'single'");
    assert_eq!(render(r#"echo "double""#), r#"echo "double""#);
    assert_eq!(render(r#"echo "a $x b""#), r#"echo "a $x b""#);
    assert_eq!(render(r"echo a\ b"), r"echo a\ b");
}

/// The expansion is *not* performed — `$BASH_COMMAND` is what is about to run, not its result.
#[test]
fn expansions_stay_unexpanded() {
    assert_eq!(render("echo $HOME"), "echo $HOME");
    assert_eq!(render("echo ${x:-fallback}"), "echo ${x:-fallback}");
    assert_eq!(render("echo ${x-}"), "echo ${x-}");
    assert_eq!(render("echo ${#x}"), "echo ${#x}");
    assert_eq!(render("echo ${x%%.c}"), "echo ${x%%.c}");
    assert_eq!(render("echo ${x/a/b}"), "echo ${x/a/b}");
    assert_eq!(render("echo $(date)"), "echo $(date)");
    assert_eq!(render("echo $((1 + 2))"), "echo $((1 + 2))");
}

/// A bare name needs no braces; anything else keeps them, or the render stops being re-parsable.
#[test]
fn braces_appear_only_where_they_are_needed() {
    assert!(plain_name("PATH"));
    assert!(plain_name("_x1"));
    assert!(plain_name("?"));
    assert!(!plain_name("11"));
    assert!(!plain_name("a b"));
    assert!(!plain_name(""));
}

/// The shape hexe's hook matches on: a bare function name, which must come back exactly.
#[test]
fn a_hook_can_recognise_its_own_function() {
    assert_eq!(render("__hexe_preexec"), "__hexe_preexec");
    assert_eq!(render("hexe shp exit-intent"), "hexe shp exit-intent");
}
