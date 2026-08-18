//! What each event, designator and substitution expands to, checked against `bash -i`.

use super::*;

fn history() -> Vec<String> {
    ["echo alpha beta gamma", "ls -l /tmp", "echo A B"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn expanded(line: &str) -> String {
    match expand(line, &history()) {
        Ok(Expansion::Expanded(s)) => s,
        other => panic!("{line:?} expected an expansion, got {other:?}"),
    }
}

fn err(line: &str) -> HistoryError {
    match expand(line, &history()) {
        Err(e) => e,
        other => panic!("{line:?} expected an error, got {other:?}"),
    }
}

#[test]
fn events_and_designators_match_bash() {
    // (input, expansion) — every row checked against `bash -i` with the same history.
    let cases = [
        ("!!", "echo A B"),
        ("!-1", "echo A B"),
        ("!-3", "echo alpha beta gamma"),
        ("!1", "echo alpha beta gamma"),
        ("!3", "echo A B"),
        ("!ls", "ls -l /tmp"),
        ("!echo", "echo A B"),
        ("!?tmp?", "ls -l /tmp"),
        ("!?tmp", "ls -l /tmp"),
        ("sudo !!", "sudo echo A B"),
        ("echo pre !$ post", "echo pre B post"),
        ("echo pre !^ post", "echo pre A post"),
        ("echo pre !* post", "echo pre A B post"),
        ("echo <!!:0>", "echo <echo>"),
        ("echo <!1:2>", "echo <beta>"),
        ("echo <!1:1-2>", "echo <alpha beta>"),
        ("echo <!1:1*>", "echo <alpha beta gamma>"),
        ("echo <!1:1->", "echo <alpha beta>"),
        // **`..` is oslo's, and reads as it does in a stream coordinate**: both ends included,
        // either end omittable. `{0:1..2}` and `!1:1..2` are the same two words.
        ("echo <!1:1..2>", "echo <alpha beta>"),
        ("echo <!1:0..1>", "echo <echo alpha>"),
        ("echo <!1:2..3>", "echo <beta gamma>"),
        // No end is *through the last* — where bash's `n-` stops one short, above.
        ("echo <!1:1..>", "echo <alpha beta gamma>"),
        // No start is word zero, the command itself.
        ("echo <!1:..1>", "echo <echo alpha>"),
        ("echo <!1:..>", "echo <echo alpha beta gamma>"),
        // **One dot is not a range**, so a suffix survives: this is why `:` stays required.
        ("echo <!1:1.bak>", "echo <alpha.bak>"),
        ("echo <!1:1.2>", "echo <alpha.2>"),
        ("echo <!1:$.gz>", "echo <gamma.gz>"),
        ("echo <!1:$>", "echo <gamma>"),
        // `[!!]` is the one bracket bash still expands, because `!!` wins over the glob rule.
        ("echo [!!]", "echo [echo A B]"),
        // A `[` with no `]` is not a bracket expression, so the reference stands.
        ("echo [!$", "echo [B"),
        ("echo [a!$]", "echo [aB]"),
        // Expansion happens inside double quotes but not inside single quotes.
        ("echo \"!$\"", "echo \"B\""),
        ("echo 'x' !$", "echo 'x' B"),
        // Two references in one line. Both resolve against the stored history, so the second
        // `!!` is still the previous *entry* and not what the first one just produced.
        ("!ls | !!", "ls -l /tmp | echo A B"),
        ("echo X!^X", "echo XAX"),
    ];
    for (input, want) in cases {
        assert_eq!(expanded(input), want, "expanding {input:?}");
    }
}

#[test]
fn lines_without_a_reference_are_left_alone() {
    // A `!` that bash treats as data: before whitespace, `=` and `(`, at end of line, inside
    // single quotes, and after a backslash.
    let cases = [
        "echo hello",
        "echo a! b",
        "echo a!=b",
        "test 1 != 2",
        "echo 5!",
        "echo !",
        "echo '!!'",
        "echo '!$'",
        "echo \\!",
        "echo \"\\!\"",
        "! true",
        "if ! grep -q x f; then echo no; fi",
        "echo a!(b)",
        // Glob bracket negation, the reason the `[!` rule exists at all.
        "ls [!a]*",
        "echo [!$]",
        "echo [!1]",
    ];
    for input in cases {
        assert_eq!(
            expand(input, &history()),
            Ok(Expansion::Unchanged),
            "{input:?} must be left alone"
        );
    }
}

#[test]
fn a_backslash_survives_into_the_shells_own_quote_removal() {
    // The pass must not eat the backslash: `echo \!` prints `!` only because the shell still
    // sees the escape afterwards.
    assert_eq!(expand("echo \\!", &history()), Ok(Expansion::Unchanged));
    assert_eq!(expanded("echo \\a !!"), "echo \\a echo A B");
}

#[test]
fn quick_substitution_edits_the_previous_line() {
    assert_eq!(expanded("^A^Z"), "echo Z B");
    assert_eq!(expanded("^A^Z^"), "echo Z B");
    assert_eq!(expanded("^A^Z^ | cat"), "echo Z B | cat");
    // Only the first occurrence, and only when `^` opens the line.
    assert_eq!(expand("echo a^b^c", &history()), Ok(Expansion::Unchanged));
    assert_eq!(
        err("^nope^x"),
        HistoryError::SubstitutionFailed("^nope^x".to_string())
    );
}

#[test]
fn unresolvable_references_are_errors_not_guesses() {
    assert_eq!(
        err("!nosuch"),
        HistoryError::EventNotFound("!nosuch".to_string())
    );
    assert_eq!(err("!99"), HistoryError::EventNotFound("!99".to_string()));
    assert_eq!(err("!0"), HistoryError::EventNotFound("!0".to_string()));
    assert_eq!(err("!-9"), HistoryError::EventNotFound("!-9".to_string()));
    assert_eq!(
        err("!?zzz?"),
        HistoryError::EventNotFound("!?zzz".to_string())
    );
    // Out of range and unknown designators both refuse rather than silently drop the word.
    assert_eq!(
        err("echo !!:9"),
        HistoryError::BadWordSpecifier(":9".to_string())
    );
    assert_eq!(
        err("echo !!:h"),
        HistoryError::BadWordSpecifier(":h".to_string())
    );
    // A range refuses the same way, and names itself so the message says which one.
    assert_eq!(
        err("echo !1:9.."),
        HistoryError::BadWordSpecifier(":9..".to_string())
    );
    assert_eq!(
        err("echo !1:1..99"),
        HistoryError::BadWordSpecifier(":1..99".to_string())
    );
    // Backwards is empty, and empty is refused rather than silently dropped.
    assert_eq!(
        err("echo !1:3..1"),
        HistoryError::BadWordSpecifier(":3..1".to_string())
    );
}

#[test]
fn an_empty_history_cannot_satisfy_any_reference() {
    assert_eq!(
        expand("!!", &[]),
        Err(HistoryError::EventNotFound("!!".to_string()))
    );
    assert_eq!(
        expand("^a^b", &[]),
        Err(HistoryError::EventNotFound("!!".to_string()))
    );
    assert_eq!(expand("echo hi", &[]), Ok(Expansion::Unchanged));
}

#[test]
fn error_text_names_the_reference_that_failed() {
    assert_eq!(
        HistoryError::EventNotFound("!x".into()).to_string(),
        "!x: event not found"
    );
    assert_eq!(
        HistoryError::BadWordSpecifier(":9".into()).to_string(),
        ":9: bad word specifier"
    );
    assert_eq!(
        HistoryError::SubstitutionFailed("^a^b".into()).to_string(),
        "^a^b: substitution failed"
    );
}
