//! Conversion tests for the brush -> rush AST bridge.

use super::*;
use crate::ast::*;

fn parse(src: &str) -> CommandList {
    parse_bash_script(src).expect("parse")
}

/// The single simple command in a one-item script.
fn only_simple(src: &str) -> SimpleCommand {
    let list = parse(src);
    assert_eq!(list.items.len(), 1, "expected one item in {:?}", src);
    match &list.items[0].and_or.first.commands[0] {
        Command::Simple(s) => s.clone(),
        other => panic!("expected simple command, got {:?}", other),
    }
}

fn only_compound(src: &str) -> CompoundCommand {
    let list = parse(src);
    assert_eq!(list.items.len(), 1, "expected one item in {:?}", src);
    match &list.items[0].and_or.first.commands[0] {
        Command::Compound { kind, .. } => kind.clone(),
        other => panic!("expected compound command, got {:?}", other),
    }
}

mod conditionals;
mod redirections;

// --- separators -------------------------------------------------------

#[test]
fn background_separator_is_preserved() {
    let list = parse("sleep 1 &");
    assert_eq!(list.items[0].op, ListOp::Background);
}

#[test]
fn sequential_separator_is_preserved() {
    let list = parse("echo a; echo b");
    assert_eq!(list.items.len(), 2);
    assert_eq!(list.items[0].op, ListOp::Sequential);
}

#[test]
fn mixed_separators() {
    let list = parse("echo a & echo b; echo c &");
    assert_eq!(list.items.len(), 3);
    assert_eq!(list.items[0].op, ListOp::Background);
    assert_eq!(list.items[1].op, ListOp::Sequential);
    assert_eq!(list.items[2].op, ListOp::Background);
}
// --- general ----------------------------------------------------------

#[test]
fn assignments_and_words_are_separated() {
    let cmd = only_simple("FOO=bar baz qux");
    assert_eq!(cmd.assignments.len(), 1);
    assert_eq!(cmd.assignments[0].name(), "FOO");
    assert_eq!(cmd.words.len(), 2);
}

#[test]
fn quoted_arguments_stay_one_word() {
    let cmd = only_simple(r#"echo "a   b""#);
    assert_eq!(cmd.words.len(), 2);
}

#[test]
fn pipeline_negation_is_preserved() {
    let list = parse("! false");
    assert!(list.items[0].and_or.first.negated);
}

#[test]
fn function_definitions_convert() {
    let list = parse("greet() { echo hi; }");
    match &list.items[0].and_or.first.commands[0] {
        Command::FunctionDef { name, .. } => assert_eq!(name, "greet"),
        other => panic!("expected function def, got {:?}", other),
    }
}

#[test]
fn subshell_and_group_are_distinguished() {
    assert!(matches!(
        only_compound("( echo x )"),
        CompoundCommand::Subshell(_)
    ));
    assert!(matches!(
        only_compound("{ echo x; }"),
        CompoundCommand::Group(_)
    ));
}

#[test]
fn for_loop_items_convert() {
    match only_compound("for i in a b c; do echo $i; done") {
        CompoundCommand::For {
            var_name, items, ..
        } => {
            assert_eq!(var_name, "i");
            assert_eq!(items.expect("items").len(), 3);
        }
        other => panic!("expected for, got {:?}", other),
    }
}

#[test]
fn for_loop_without_items_is_none() {
    match only_compound("for i; do echo $i; done") {
        CompoundCommand::For { items, .. } => assert!(items.is_none()),
        other => panic!("expected for, got {:?}", other),
    }
}

#[test]
fn until_loop_converts() {
    assert!(matches!(
        only_compound("until false; do echo x; done"),
        CompoundCommand::Until { .. }
    ));
}
