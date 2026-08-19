//! What the lexer makes of a typed line.
//!
//! Split out because the split itself is the interesting half — it needs no `$PATH`, no
//! filesystem and no terminal, so every case here is a string in and spans out.

use super::*;

fn roles(line: &str) -> Vec<(String, Role)> {
    lex(line)
        .into_iter()
        .filter(|s| s.role != Role::Plain)
        .map(|s| (s.text, s.role))
        .collect()
}

/// The one invariant everything else rests on: the spans put the line back together exactly.
#[test]
fn spans_reassemble_into_the_original_line() {
    for line in [
        "echo hi",
        "ls -l | wc -l",
        "echo \"a b\" $HOME",
        "git commit -m 'x' && true",
        "cat <<EOF",
        "grep -n 'x' f.txt 2>&1 >out # a comment",
        "FOO=bar cmd arg",
        "echo ${HOME}/x $(date) \\$literal",
        "if true; then echo y; fi",
        "echo 'unclosed",
        "echo \\",
        "",
    ] {
        let joined: String = lex(line).iter().map(|s| s.text.as_str()).collect();
        assert_eq!(joined, line, "for {line:?}");
    }
}

#[test]
fn the_first_word_of_each_command_is_a_command_word() {
    let found: Vec<String> = lex("ls | wc; grep x && true")
        .into_iter()
        .filter(|s| s.role == Role::CommandWord)
        .map(|s| s.text)
        .collect();
    assert_eq!(found, vec!["ls", "wc", "grep", "true"]);
}

/// A keyword does not consume the command position — `if grep` still has `grep` as the
/// command, which is what makes an unknown command after `if` still show up as wrong.
#[test]
fn a_keyword_leaves_the_command_position_open() {
    let spans = lex("if grep x; then echo y; fi");
    let keywords: Vec<&str> = spans
        .iter()
        .filter(|s| s.role == Role::Keyword)
        .map(|s| s.text.as_str())
        .collect();
    assert_eq!(keywords, vec!["if", "then", "fi"]);
    let commands: Vec<&str> = spans
        .iter()
        .filter(|s| s.role == Role::CommandWord)
        .map(|s| s.text.as_str())
        .collect();
    assert_eq!(commands, vec!["grep", "echo"]);
}

/// `FOO=bar cmd` runs `cmd`, so the assignment must not eat the command position.
#[test]
fn an_assignment_prefix_is_not_the_command() {
    let spans = lex("FOO=bar BAZ=1 cmd arg");
    let commands: Vec<&str> = spans
        .iter()
        .filter(|s| s.role == Role::CommandWord)
        .map(|s| s.text.as_str())
        .collect();
    assert_eq!(commands, vec!["cmd"]);
    // And `x=1` alone is not a command at all.
    assert!(
        lex("x=1").iter().all(|s| s.role != Role::CommandWord),
        "{:?}",
        lex("x=1")
    );
}

#[test]
fn redirections_are_told_apart_from_operators_and_separators() {
    assert_eq!(
        roles("a > b >> c 2>&1 <<<x"),
        vec![
            ("a".into(), Role::CommandWord),
            (">".into(), Role::Redirection),
            ("b".into(), Role::Word),
            (">>".into(), Role::Redirection),
            ("c".into(), Role::Word),
            ("2>&1".into(), Role::Redirection),
            ("<<<".into(), Role::Redirection),
            ("x".into(), Role::Word),
        ]
    );
    // `;` and a lone `&` end a command; `|`, `||` and `&&` join two.
    assert_eq!(roles("a; b & c | d && e")[1].1, Role::End);
    assert_eq!(roles("a | b")[1].1, Role::Operator);
    assert_eq!(roles("a && b")[1].1, Role::Operator);
}

/// A `#` is a comment only at the start of a word. This is the same rule that had to be fixed
/// in the shell's own lexer, and getting it wrong greys out the rest of a good line.
#[test]
fn a_hash_inside_a_word_is_not_a_comment() {
    assert!(roles("echo a#b").iter().all(|(_, r)| *r != Role::Comment));
    assert!(roles("echo $#").iter().all(|(_, r)| *r != Role::Comment));
    let commented = roles("echo x # note");
    assert_eq!(
        commented.last().map(|(t, r)| (t.as_str(), *r)),
        Some(("# note", Role::Comment))
    );
}

#[test]
fn expansions_run_to_their_closing_bracket() {
    assert_eq!(roles("echo ${HOME}")[1], ("${HOME}".into(), Role::Variable));
    assert_eq!(roles("echo $(date)")[1], ("$(date)".into(), Role::Variable));
    // Nested, which a naive scan to the first `)` gets wrong.
    assert_eq!(
        roles("echo $(echo $(date))")[1],
        ("$(echo $(date))".into(), Role::Variable)
    );
    assert_eq!(roles("echo $?")[1], ("$?".into(), Role::Variable));
    assert_eq!(roles("echo $HOME")[1], ("$HOME".into(), Role::Variable));
}

/// A line being typed is usually unfinished, so an unclosed quote is a span rather than an
/// error — deciding the line is incomplete is the validator's job, not the highlighter's.
#[test]
fn an_unclosed_quote_runs_to_the_end_rather_than_failing() {
    assert_eq!(roles("echo 'abc")[1], ("'abc".into(), Role::SingleQuote));
    assert_eq!(roles("echo \"a b")[1], ("\"a b".into(), Role::DoubleQuote));
    // A backslash inside double quotes escapes the quote that would have closed them. The
    // string is split at the escape now, so the pieces are what to check.
    let pieces: String = roles(r#"echo "a\"b""#)[1..]
        .iter()
        .map(|(text, _)| text.as_str())
        .collect();
    assert_eq!(pieces, r#""a\"b""#);
}

#[test]
fn escapes_are_their_own_span() {
    let spans = roles(r"echo a\ b");
    assert!(
        spans.iter().any(|(t, r)| t == r"\ " && *r == Role::Escape),
        "{spans:?}"
    );
    // A trailing backslash does not run past the end of the line.
    assert_eq!(
        lex(r"echo \").iter().map(|s| s.text.len()).sum::<usize>(),
        6
    );
}

/// **The bug this pair of forms had in the highlighter.** `\rm` came out as a `\r` escape
/// span and an `m` word span, so the first letter took the escape colour and the rest read as
/// an argument. Both forms are one command word, backslashes included.
#[test]
fn an_escaped_command_is_one_word_not_an_escape() {
    assert_eq!(roles(r"\rm -rf x")[0], (r"\rm".into(), Role::CommandWord));
    assert_eq!(roles(r"\\rm -rf x")[0], (r"\\rm".into(), Role::CommandWord));
    assert_eq!(
        roles(r"\which ls")[0],
        (r"\which".into(), Role::CommandWord)
    );
    // And after an operator, which is command position too.
    let piped = roles(r"ls | \grep x");
    assert!(
        piped
            .iter()
            .any(|(t, r)| t == r"\grep" && *r == Role::CommandWord),
        "{piped:?}"
    );
}

/// The escapes that are still escapes: a backslash is only a command word when a *name*
/// follows it and it stands where a command does.
#[test]
fn a_backslash_that_is_not_an_escaped_command_stays_an_escape() {
    // Not in command position.
    assert!(
        roles(r"echo \rm")
            .iter()
            .any(|(t, r)| t == r"\r" && *r == Role::Escape)
    );
    // No name after it.
    assert!(
        roles(r"\ foo")
            .iter()
            .any(|(t, r)| t == r"\ " && *r == Role::Escape)
    );
    assert!(
        roles(r"\$HOME")
            .iter()
            .any(|(t, r)| t == r"\$" && *r == Role::Escape)
    );
    // Three backslashes are not a doubled escape plus a name.
    assert!(
        roles(r"\\\rm").iter().any(|(_, r)| *r == Role::Escape),
        "{:?}",
        roles(r"\\\rm")
    );
}

/// Whatever the lexer does, every span still concatenates back to the line it was given.
#[test]
fn the_escaped_forms_still_reassemble() {
    for line in [r"\rm -rf x", r"\\rm x", r"\ foo", r"echo \rm", r"\\\rm"] {
        let joined: String = lex(line).iter().map(|s| s.text.as_str()).collect();
        assert_eq!(joined, line);
    }
}
