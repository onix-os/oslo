
use super::*;
use crate::startup::read::{HeredocTracker, is_complete};

/// The command line that opens a here-document is still a command, so it is expanded; every
/// line after it is body, so none of them are.
#[test]
fn history_expansion_stops_at_a_here_document_body() {
    let mut heredoc = HeredocTracker::default();
    assert!(heredoc.expands_history());

    heredoc.observe("cat > note <<EOF");
    assert!(!heredoc.expands_history(), "the body must not be rewritten");

    heredoc.observe("remember: 10! is 3628800");
    assert!(!heredoc.expands_history());
    heredoc.observe("EOF");
    assert!(!heredoc.expands_history(), "conservative to the end");
}

/// An ordinary unfinished command keeps its history expansion: `for i in …` continued onto a
/// second line is code on both lines, and `!!` there still means what it always did.
#[test]
fn an_ordinary_continuation_is_still_expanded() {
    let mut heredoc = HeredocTracker::default();
    for line in ["for i in 1 2 3; do", "  echo $i", "done"] {
        assert!(heredoc.expands_history(), "{line:?}");
        heredoc.observe(line);
    }
    // A here-string takes its body from the same line, so it opens nothing.
    let mut heredoc = HeredocTracker::default();
    heredoc.observe("wc -l <<<\"$text\" &&");
    assert!(heredoc.expands_history());
}

/// C10: a multi-line command teaches the ranker about every command in it, not just the one
/// on the line that happened to parse on its own.
#[test]
fn a_multi_line_command_feeds_the_frecency_table() {
    use oslo_shell::Environment;
    use oslo_ui::OsloHelper;
    use std::sync::{Arc, Mutex};

    // Not interactive, so the table is in memory and no file in `$HOME` is touched.
    let helper = OsloHelper::new(Arc::new(Mutex::new(Environment::new())));
    assert_eq!(helper.frecency_score("zzrepl_a"), 0.0);
    assert_eq!(helper.frecency_score("zzrepl_b"), 0.0);

    helper.record_command_use("for i in 1 2\ndo\n  zzrepl_a $i\n  zzrepl_b\ndone");
    assert!(helper.frecency_score("zzrepl_a") > 0.0);
    assert!(helper.frecency_score("zzrepl_b") > 0.0);
}

#[test]
fn an_unfinished_compound_command_asks_for_more() {
    assert!(!is_complete("for i in 1 2 3; do", Mode::Shell));
    assert!(!is_complete("if true; then", Mode::Shell));
    assert!(!is_complete("while true; do echo hi", Mode::Shell));
    assert!(!is_complete("case x in", Mode::Shell));
    assert!(!is_complete("echo hi |", Mode::Shell));
    assert!(!is_complete("echo \"unterminated", Mode::Shell));
    assert!(!is_complete("x=$(echo hi", Mode::Shell));
}

#[test]
fn a_finished_command_runs() {
    assert!(is_complete("echo hi", Mode::Shell));
    assert!(is_complete("for i in 1 2 3; do echo $i; done", Mode::Shell));
    assert!(is_complete("if true; then echo y; fi", Mode::Shell));
}

/// Lua asks its own parser, rather than string-matching `<eof>` in an error message the way
/// the reference implementation's C API forces it to.
#[test]
fn lua_mode_continues_an_unfinished_chunk() {
    assert!(!is_complete("if true then", Mode::Lua));
    assert!(!is_complete("local t = {", Mode::Lua));
    assert!(!is_complete("function f(", Mode::Lua));
    assert!(is_complete("print(1)", Mode::Lua));
    // A real mistake never becomes valid, so asking for another line would wedge the prompt.
    assert!(is_complete("x = = 2", Mode::Lua));
}

#[test]
fn a_real_syntax_error_is_not_a_continuation() {
    // Otherwise a typo would wedge the prompt: every further line is also an error, and
    // there is no way back to PS1.
    assert!(is_complete("echo )", Mode::Shell));
    assert!(is_complete("fi", Mode::Shell));
    assert!(is_complete("done", Mode::Shell));
}
