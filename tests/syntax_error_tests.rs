//! What the shell does with input it cannot run.
//!
//! rush used to carry a second, hand-written parser and re-parse the whole program with it
//! whenever brush or the adapter reported an error. That fallback had no here-document support,
//! so it parsed heredoc *bodies* as commands and executed them — a file that merely contained
//! `touch /tmp/pwned` created the file. It also stopped silently at a token it did not know, so
//! half a program could run and the shell would still exit 0.
//!
//! These tests pin the two properties that replaced it: data stays data, and unrunnable input
//! produces a diagnostic and a non-zero status instead of a guess.

mod common;

use common::{run, run_in};

/// The reproduction from PLAN R1.1, verbatim.
///
/// `((x++))` is rejected by the adapter, which is exactly what used to reroute the whole script
/// to the fallback parser; the heredoc body then ran as commands.
#[test]
fn a_heredoc_body_is_never_executed_when_the_script_fails_to_parse() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("heredoc_executed_marker");
    let script = format!(
        "((x++))\ncat <<EOF\ntouch {}\nEOF\n",
        marker.to_string_lossy()
    );

    let r = run_in(dir.path(), &script);

    assert!(
        !marker.exists(),
        "the heredoc body was executed as a command: {} exists",
        marker.display()
    );
    assert_ne!(r.status, 0, "an unparseable script must not exit 0");
    assert!(!r.stderr.is_empty(), "the failure must be reported");
}

/// The same shape, but with the unsupported construct *after* the heredoc, so the parse of the
/// heredoc itself is what carries the body.
#[test]
fn a_heredoc_body_is_not_executed_when_a_later_line_fails_to_parse() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("later_marker");
    let script = format!(
        "cat <<EOF\ntouch {}\nEOF\ncoproc foo {{ true; }}\n",
        marker.to_string_lossy()
    );

    let r = run_in(dir.path(), &script);

    assert!(!marker.exists(), "the heredoc body was executed");
    assert_ne!(r.status, 0);
}

#[test]
fn a_stray_closing_paren_is_a_syntax_error() {
    let r = run("echo a; )\necho NOT_REACHED");
    assert_eq!(r.status, 2, "a syntax error exits 2\nstderr: {}", r.stderr);
    assert!(
        !r.stdout.contains("NOT_REACHED"),
        "nothing in an unparseable script may run: {:?}",
        r.stdout
    );
    assert!(
        !r.stdout.contains('a'),
        "the program is rejected as a whole, not run up to the bad token: {:?}",
        r.stdout
    );
}

#[test]
fn an_unterminated_quote_is_a_syntax_error() {
    let r = run("echo 'unterminated");
    assert_eq!(r.status, 2, "stderr: {}", r.stderr);
    assert!(!r.stderr.is_empty());
}

#[test]
fn an_incomplete_compound_command_is_a_syntax_error() {
    let r = run("if true; then\necho NOT_REACHED");
    assert_eq!(r.status, 2, "stderr: {}", r.stderr);
    assert!(!r.stdout.contains("NOT_REACHED"));
}

#[test]
fn a_syntax_error_reports_where_it_is() {
    let r = run("echo ok\nfor\n");
    assert_eq!(r.status, 2, "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("line 2") || r.stderr.contains("end of input"),
        "the diagnostic should carry brush's position: {:?}",
        r.stderr
    );
}

/// Constructs the adapter cannot represent must name themselves. Before the fallback was deleted
/// they silently rerouted the program; a bare "syntax error" would now be almost as unhelpful.
#[test]
fn unsupported_constructs_are_named_in_the_diagnostic() {
    for (script, needle) in [
        ("x=5\n((x++))", "((...))"),
        ("for ((i=0; i<3; i++)); do echo $i; done", "for ((...))"),
        ("coproc foo { true; }", "coproc"),
    ] {
        let r = run(script);
        assert_ne!(r.status, 0, "{script:?} must not succeed");
        assert!(
            r.stderr.contains(needle) && r.stderr.contains("not supported"),
            "{script:?}: stderr should name the construct, got {:?}",
            r.stderr
        );
    }
}

/// `select` is not in brush's grammar at all, so it surfaces as a plain parse error. It still
/// must not run anything.
#[test]
fn select_is_rejected() {
    let r = run("select x in a b; do echo $x; done\necho NOT_REACHED");
    assert_ne!(r.status, 0);
    assert!(!r.stdout.contains("NOT_REACHED"));
}

// --- R1.5: the substitution body is parsed before the fork ---

#[test]
fn an_unparseable_substitution_body_does_not_panic() {
    for script in ["echo $(if)", "echo $(;;)", "x=$(if); echo $x"] {
        let r = run(script);
        assert!(
            !r.stderr.contains("panicked"),
            "{script:?} panicked: {}",
            r.stderr
        );
        assert_ne!(r.status, 0, "{script:?} must report failure, got 0");
    }
}

#[test]
fn an_unparseable_substitution_body_stops_the_script() {
    let r = run("echo $(if)\necho NOT_REACHED");
    assert!(!r.stdout.contains("NOT_REACHED"), "stdout: {:?}", r.stdout);
}

// --- R1.6 (substitution half): NUL bytes never reach argv ---

#[test]
fn nul_bytes_are_stripped_from_substitution_output() {
    let r = run("x=$(printf 'a\\0b'); printf '[%s]\\n' \"$x\"; echo STILL_ALIVE");
    assert_eq!(r.out(), "[ab]\nSTILL_ALIVE", "stderr: {}", r.stderr);
    assert_eq!(r.status, 0);
}

#[test]
fn nul_bearing_substitution_output_can_be_passed_to_an_external_command() {
    let r = run("x=$(printf 'a\\0b'); /bin/echo \"[$x]\"");
    assert!(!r.stderr.contains("panicked"), "stderr: {}", r.stderr);
    assert_eq!(r.out(), "[ab]");
}

// --- eval and source report their own syntax errors ---

#[test]
fn eval_of_unparseable_text_returns_two_and_the_script_continues() {
    let r = run("eval 'if'; echo AFTER=$?");
    assert_eq!(r.out(), "AFTER=2", "stderr: {}", r.stderr);
    assert_eq!(r.status, 0);
}

#[test]
fn sourcing_an_unparseable_file_returns_two_and_the_script_continues() {
    let r = run("printf 'if\\n' > bad.sh; . ./bad.sh; echo AFTER=$?");
    assert_eq!(r.out(), "AFTER=2", "stderr: {}", r.stderr);
}
