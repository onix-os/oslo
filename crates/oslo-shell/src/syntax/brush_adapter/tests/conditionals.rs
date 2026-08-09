//! `case`, `if`/`elif`, and the `[[ ]]` lowering.

use super::{only_compound, parse};
use oslo_base::ast::*;

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
            WordPart::Literal(l) | WordPart::SingleQuoted(l) | WordPart::Escaped(l) => {
                out.push_str(l)
            }
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
fn regex_match_lowers_to_the_pattern_operator() {
    assert_eq!(
        test_invocations("[[ abc =~ a.c ]]"),
        vec![vec![
            "[[".to_string(),
            "abc".into(),
            "=~".into(),
            "a.c".into(),
            "]]".into()
        ]]
    );
}

#[test]
fn quoted_regex_operand_lowers_to_the_literal_operator() {
    // Same distinction the `==`/`=` pair draws: quoting the right operand turns off its special
    // meaning, so `[[ abc =~ "a.c" ]]` is false where the unquoted form is true.
    assert_eq!(
        test_invocations(r#"[[ abc =~ "a.c" ]]"#),
        vec![vec![
            "[[".to_string(),
            "abc".into(),
            "=~lit".into(),
            "a.c".into(),
            "]]".into()
        ]]
    );
}

#[test]
fn every_unary_predicate_has_a_flag() {
    // These used to be `unsupported test predicate` syntax errors while `[ -o errexit ]` answered
    // correctly — one predicate, two verdicts, decided by which bracket was typed.
    for (source, flag) in [
        ("[[ -o errexit ]]", "-o"),
        ("[[ -t 1 ]]", "-t"),
        ("[[ -N f ]]", "-N"),
        ("[[ -G f ]]", "-G"),
        ("[[ -O f ]]", "-O"),
        ("[[ -u f ]]", "-u"),
        ("[[ -g f ]]", "-g"),
        ("[[ -k f ]]", "-k"),
        ("[[ -R v ]]", "-R"),
    ] {
        assert_eq!(test_invocations(source)[0][1], flag, "{}", source);
    }
}
