//! Redirection conversion: files, descriptors, here-documents and here-strings.

use super::{only_simple, parse};
use oslo_base::ast::*;

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

/// The body of a heredoc.
fn heredoc(src: &str) -> Word {
    let cmd = only_simple(src);
    cmd.redirections[0]
        .heredoc_content
        .clone()
        .unwrap_or_else(|| panic!("no heredoc body in {src:?}"))
}

#[test]
fn heredoc_body_is_captured() {
    let cmd = only_simple("cat <<EOF\nline one\nline two\nEOF");
    assert_eq!(cmd.redirections.len(), 1);
    assert_eq!(cmd.redirections[0].kind, RedirectKind::Heredoc);
    assert_eq!(
        heredoc("cat <<EOF\nline one\nline two\nEOF").parts,
        vec![WordPart::Literal("line one\nline two\n".into())]
    );
}

#[test]
fn dash_heredoc_strips_leading_tabs() {
    let cmd = only_simple("cat <<-EOF\n\tindented\nEOF");
    assert_eq!(cmd.redirections[0].kind, RedirectKind::HeredocStrip);
    assert_eq!(
        heredoc("cat <<-EOF\n\tindented\nEOF").parts,
        vec![WordPart::Literal("indented\n".into())]
    );
}

/// R11.B2. An unquoted body is parts, not text, because it has to expand before the command
/// reads it.
#[test]
fn an_unquoted_heredoc_body_keeps_its_expansions() {
    assert_eq!(
        heredoc("cat <<EOF\nx=$v\nEOF").parts,
        vec![
            WordPart::Literal("x=".into()),
            WordPart::Variable {
                name: "v".into(),
                expansion_type: ParamExpansion::Normal,
            },
            WordPart::Literal("\n".into()),
        ]
    );
}

/// A quoted delimiter makes the whole body inert, which the AST records as one literal part —
/// the `$v` must not become a `Variable` that expansion would then act on.
#[test]
fn a_quoted_delimiter_makes_the_body_one_literal() {
    assert_eq!(
        heredoc("cat <<'EOF'\nx=$v\nEOF").parts,
        vec![WordPart::Literal("x=$v\n".into())]
    );
}

/// A body is not a word: `'` and `"` are ordinary characters in it, and a scanner that treated
/// them as quotes would reject `it's` outright.
#[test]
fn quote_characters_in_a_body_are_literal_text() {
    assert_eq!(
        heredoc("cat <<EOF\nit's \"fine\"\nEOF").parts,
        vec![WordPart::Literal("it's \"fine\"\n".into())]
    );
}

/// R11.B3. A here-string is one ordinary word, so quoting survives as parts and expansion does
/// the quote removal a textual strip used to do — wrongly, whenever the quotes were not the
/// outermost characters.
#[test]
fn here_string_becomes_a_heredoc() {
    let cmd = only_simple("cat <<< hello");
    assert_eq!(cmd.redirections[0].kind, RedirectKind::Heredoc);
    assert!(cmd.redirections[0].here_string);
    assert_eq!(
        heredoc("cat <<< hello").parts,
        vec![WordPart::Literal("hello".into())]
    );
}

#[test]
fn here_string_quoting_survives_as_parts() {
    assert_eq!(
        heredoc(r#"cat <<< "a $v""#).parts,
        vec![WordPart::DoubleQuoted(vec![
            WordPart::Literal("a ".into()),
            WordPart::Variable {
                name: "v".into(),
                expansion_type: ParamExpansion::Normal,
            },
        ])]
    );
}

/// The trailing newline is the consume site's job, not the parser's: baked in here it would be
/// appended before command substitution's own trailing-newline trim could apply.
#[test]
fn the_here_string_newline_is_not_baked_into_the_word() {
    assert_eq!(
        heredoc(r#"cat <<< "$(echo x)""#).parts,
        vec![WordPart::DoubleQuoted(vec![WordPart::CommandSubstitution(
            "echo x".into()
        )])]
    );
}

/// A here-document is not a here-string, and only one of the two gets a newline appended.
#[test]
fn only_a_here_string_is_marked_as_one() {
    assert!(!only_simple("cat <<EOF\nbody\nEOF").redirections[0].here_string);
    assert!(!only_simple("echo hi > out.txt").redirections[0].here_string);
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
