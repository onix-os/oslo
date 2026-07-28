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

// --- redirections -----------------------------------------------------

#[test]
fn output_redirect_is_carried_through() {
    let cmd = only_simple("echo hi > out.txt");
    assert_eq!(cmd.redirections.len(), 1);
    assert_eq!(cmd.redirections[0].kind, RedirectKind::Output);
    assert_eq!(cmd.redirections[0].fd, None);
    assert_eq!(cmd.redirections[0].target, Word::from_literal("out.txt"));
    // The redirection must not leak into the argument list.
    assert_eq!(cmd.words.len(), 2);
}

#[test]
fn input_and_append_redirects() {
    assert_eq!(
        only_simple("cat < in.txt").redirections[0].kind,
        RedirectKind::Input
    );
    assert_eq!(
        only_simple("echo x >> log").redirections[0].kind,
        RedirectKind::Append
    );
    assert_eq!(
        only_simple("echo x >| log").redirections[0].kind,
        RedirectKind::Clobber
    );
    assert_eq!(
        only_simple("exec 3<> file").redirections[0].kind,
        RedirectKind::ReadWrite
    );
}

#[test]
fn explicit_fd_is_preserved() {
    let cmd = only_simple("ls 2> err.txt");
    assert_eq!(cmd.redirections[0].fd, Some(2));
    assert_eq!(cmd.redirections[0].kind, RedirectKind::Output);
}

#[test]
fn fd_duplication() {
    let cmd = only_simple("ls 2>&1");
    assert_eq!(cmd.redirections[0].fd, Some(2));
    assert_eq!(cmd.redirections[0].kind, RedirectKind::DupOutput);
    assert_eq!(cmd.redirections[0].target, Word::from_literal("1"));
}

#[test]
fn multiple_redirects_keep_their_order() {
    let cmd = only_simple("cmd > out.txt 2> err.txt < in.txt");
    assert_eq!(cmd.redirections.len(), 3);
    assert_eq!(cmd.redirections[0].kind, RedirectKind::Output);
    assert_eq!(cmd.redirections[1].fd, Some(2));
    assert_eq!(cmd.redirections[2].kind, RedirectKind::Input);
}

#[test]
fn redirect_before_the_command_is_found() {
    let cmd = only_simple("> out.txt echo hi");
    assert_eq!(cmd.redirections.len(), 1);
    assert_eq!(cmd.redirections[0].kind, RedirectKind::Output);
    assert_eq!(cmd.words[0], Word::from_literal("echo"));
}

#[test]
fn output_and_error_becomes_two_redirects() {
    let cmd = only_simple("cmd &> both.txt");
    assert_eq!(cmd.redirections.len(), 2);
    assert_eq!(cmd.redirections[0].fd, Some(1));
    assert_eq!(cmd.redirections[0].kind, RedirectKind::Output);
    assert_eq!(cmd.redirections[1].fd, Some(2));
    assert_eq!(cmd.redirections[1].kind, RedirectKind::DupOutput);
    assert_eq!(cmd.redirections[1].target, Word::from_literal("1"));
}

#[test]
fn heredoc_body_is_captured() {
    let cmd = only_simple("cat <<EOF\nline one\nline two\nEOF");
    assert_eq!(cmd.redirections.len(), 1);
    assert_eq!(cmd.redirections[0].kind, RedirectKind::Heredoc);
    let body = cmd.redirections[0].heredoc_content.as_deref().unwrap();
    assert!(body.contains("line one"), "body was {:?}", body);
    assert!(body.contains("line two"), "body was {:?}", body);
}

#[test]
fn dash_heredoc_strips_leading_tabs() {
    let cmd = only_simple("cat <<-EOF\n\tindented\nEOF");
    assert_eq!(cmd.redirections[0].kind, RedirectKind::HeredocStrip);
    let body = cmd.redirections[0].heredoc_content.as_deref().unwrap();
    assert!(
        body.starts_with("indented"),
        "tabs not stripped: {:?}",
        body
    );
}

#[test]
fn here_string_becomes_a_heredoc() {
    let cmd = only_simple("cat <<< hello");
    assert_eq!(cmd.redirections[0].kind, RedirectKind::Heredoc);
    assert_eq!(
        cmd.redirections[0].heredoc_content.as_deref(),
        Some("hello\n")
    );
}

#[test]
fn here_string_quotes_are_stripped() {
    let cmd = only_simple(r#"cat <<< "a b""#);
    assert_eq!(
        cmd.redirections[0].heredoc_content.as_deref(),
        Some("a b\n")
    );
}

#[test]
fn compound_redirects_are_attached() {
    let list = parse("while true; do echo x; done > log.txt");
    match &list.items[0].and_or.first.commands[0] {
        Command::Compound { redirections, .. } => {
            assert_eq!(redirections.len(), 1);
            assert_eq!(redirections[0].kind, RedirectKind::Output);
            assert_eq!(redirections[0].target, Word::from_literal("log.txt"));
        }
        other => panic!("expected compound, got {:?}", other),
    }
}

#[test]
fn redirects_survive_inside_a_pipeline() {
    let list = parse("cat < in.txt | grep x > out.txt");
    let cmds = &list.items[0].and_or.first.commands;
    assert_eq!(cmds.len(), 2);
    match (&cmds[0], &cmds[1]) {
        (Command::Simple(a), Command::Simple(b)) => {
            assert_eq!(a.redirections[0].kind, RedirectKind::Input);
            assert_eq!(b.redirections[0].kind, RedirectKind::Output);
        }
        _ => panic!("expected two simple commands"),
    }
}

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

// --- case -------------------------------------------------------------

#[test]
fn case_is_converted() {
    match only_compound("case $x in a) echo A;; b) echo B;; esac") {
        CompoundCommand::Case { word, items } => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0].patterns[0], Word::from_literal("a"));
            assert_eq!(items[1].patterns[0], Word::from_literal("b"));
            assert!(!word.parts.is_empty());
        }
        other => panic!("expected case, got {:?}", other),
    }
}

#[test]
fn case_with_multiple_patterns_per_branch() {
    match only_compound("case $x in a|b|c) echo hit;; esac") {
        CompoundCommand::Case { items, .. } => {
            assert_eq!(items[0].patterns.len(), 3);
        }
        other => panic!("expected case, got {:?}", other),
    }
}

#[test]
fn case_with_empty_branch_body() {
    match only_compound("case $x in a) ;; esac") {
        CompoundCommand::Case { items, .. } => {
            assert_eq!(items.len(), 1);
            assert!(items[0].body.items.is_empty());
        }
        other => panic!("expected case, got {:?}", other),
    }
}

// --- if / elif --------------------------------------------------------

#[test]
fn single_elif_becomes_an_elif_branch() {
    match only_compound("if a; then b; elif c; then d; fi") {
        CompoundCommand::If {
            elif_branches,
            else_branch,
            ..
        } => {
            assert_eq!(elif_branches.len(), 1);
            assert!(else_branch.is_none());
        }
        other => panic!("expected if, got {:?}", other),
    }
}

#[test]
fn multiple_elifs_are_all_preserved() {
    match only_compound("if a; then b; elif c; then d; elif e; then f; else g; fi") {
        CompoundCommand::If {
            elif_branches,
            else_branch,
            ..
        } => {
            assert_eq!(elif_branches.len(), 2);
            assert!(else_branch.is_some());
        }
        other => panic!("expected if, got {:?}", other),
    }
}

#[test]
fn plain_else_has_no_elif_branches() {
    match only_compound("if a; then b; else c; fi") {
        CompoundCommand::If {
            elif_branches,
            else_branch,
            ..
        } => {
            assert!(elif_branches.is_empty());
            assert!(else_branch.is_some());
        }
        other => panic!("expected if, got {:?}", other),
    }
}

// --- extended test ----------------------------------------------------

/// Render a word's literal text, ignoring quoting — enough to assert on argument lists.
fn flatten_word(w: &Word) -> String {
    fn part(p: &WordPart, out: &mut String) {
        match p {
            WordPart::Literal(l) | WordPart::SingleQuoted(l) => out.push_str(l),
            WordPart::DoubleQuoted(inner) => {
                for p in inner {
                    part(p, out);
                }
            }
            other => out.push_str(&format!("{:?}", other)),
        }
    }
    let mut s = String::new();
    for p in &w.parts {
        part(p, &mut s);
    }
    s
}

/// Flatten a converted `[[ ]]` back to the argument lists it produced.
fn test_invocations(src: &str) -> Vec<Vec<String>> {
    fn walk(cmd: &Command, out: &mut Vec<Vec<String>>) {
        match cmd {
            Command::Simple(s) => {
                out.push(s.words.iter().map(flatten_word).collect());
            }
            Command::Compound { kind, .. } => {
                if let CompoundCommand::Group(list) = kind {
                    for item in &list.items {
                        for c in &item.and_or.first.commands {
                            walk(c, out);
                        }
                        for (_, p) in &item.and_or.rest {
                            for c in &p.commands {
                                walk(c, out);
                            }
                        }
                    }
                }
            }
            Command::FunctionDef { .. } => {}
        }
    }

    let list = parse(src);
    let mut out = Vec::new();
    for c in &list.items[0].and_or.first.commands {
        walk(c, &mut out);
    }
    out
}

#[test]
fn unquoted_rhs_is_a_glob_comparison() {
    assert_eq!(
        test_invocations("[[ a == b ]]"),
        vec![vec![
            "[[".to_string(),
            "a".into(),
            "==".into(),
            "b".into(),
            "]]".into()
        ]]
    );
}

#[test]
fn quoted_rhs_is_a_literal_comparison() {
    assert_eq!(
        test_invocations(r#"[[ a == "b" ]]"#),
        vec![vec![
            "[[".to_string(),
            "a".into(),
            "=".into(),
            "b".into(),
            "]]".into()
        ]]
    );
}

#[test]
fn negative_predicates_negate_the_pipeline() {
    // `!=` is emitted as a negated `==`, so the builtin only implements the positive form.
    let list = parse("[[ a != b ]]");
    match &list.items[0].and_or.first.commands[0] {
        Command::Compound {
            kind: CompoundCommand::Group(inner),
            ..
        } => assert!(inner.items[0].and_or.first.negated),
        other => panic!("expected group, got {:?}", other),
    }
    assert_eq!(test_invocations("[[ a != b ]]")[0][2], "==");
}

#[test]
fn extended_test_unary_is_converted() {
    assert_eq!(
        test_invocations("[[ -f foo ]]"),
        vec![vec![
            "[[".to_string(),
            "-f".into(),
            "foo".into(),
            "]]".into()
        ]]
    );
}

#[test]
fn extended_test_arithmetic_predicates() {
    assert_eq!(
        test_invocations("[[ 1 -lt 2 ]]"),
        vec![vec![
            "[[".to_string(),
            "1".into(),
            "-lt".into(),
            "2".into(),
            "]]".into()
        ]]
    );
}

#[test]
fn extended_test_and_becomes_an_and_or_list() {
    let invocations = test_invocations("[[ -f a && -d b ]]");
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[0][1], "-f");
    assert_eq!(invocations[1][1], "-d");
}

#[test]
fn extended_test_negation_negates_the_pipeline() {
    let list = parse("[[ ! -f a ]]");
    // The outer command is a group wrapping the negated pipeline.
    match &list.items[0].and_or.first.commands[0] {
        Command::Compound {
            kind: CompoundCommand::Group(inner),
            ..
        } => {
            assert!(inner.items[0].and_or.first.negated);
        }
        other => panic!("expected group, got {:?}", other),
    }
}

#[test]
fn unsupported_predicate_is_an_error_not_a_silent_true() {
    // `=~` needs a regex engine. It must surface as a syntax error rather than being
    // converted into something that always succeeds.
    assert!(parse_bash_script("[[ abc =~ a.c ]]").is_err());
}

// --- general ----------------------------------------------------------

#[test]
fn assignments_and_words_are_separated() {
    let cmd = only_simple("FOO=bar baz qux");
    assert_eq!(cmd.assignments.len(), 1);
    assert_eq!(cmd.assignments[0].name, "FOO");
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
