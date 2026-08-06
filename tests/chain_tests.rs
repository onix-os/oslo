//! What each link of `a && b || c` did.
//!
//! `eval_and_or_list` computes this and used to drop all but the last status. `$PIPESTATUS`
//! answers the same question one level down — the stages inside one pipeline — and there has never
//! been anything for the level above it.
//!
//! Driven through the real evaluator rather than by pushing rows into the buffer by hand: the unit
//! tests beside the module already cover the buffer's own arithmetic, and what is worth proving
//! here is that the executor puts the right things in it.

use oslo::env::Environment;
use oslo::exec::eval_command_list;
use oslo::exec::pipeline::segments::{self, Join};
use oslo::parser::parse_bash_script;

/// Run `line` the way the read loop does, and hand back what the recorder saw.
fn run(line: &str) -> Vec<segments::Segment> {
    let mut env = Environment::new();
    let ast = parse_bash_script(line).expect("parses");
    segments::arm();
    let _ = eval_command_list(&mut env, &ast);
    segments::disarm();
    segments::taken()
}

/// Every link, in order, joined as written.
#[test]
fn a_chain_records_one_link_per_pipeline() {
    let segments = run("true && true && true");
    assert_eq!(segments.len(), 3, "{segments:?}");
    assert_eq!(segments[0].join, Join::First);
    assert_eq!(segments[1].join, Join::And);
    assert_eq!(segments[2].join, Join::And);
    assert!(segments.iter().all(|s| s.status == Some(0)), "{segments:?}");
}

/// **The distinction the whole thing exists for.** `echo never` did not run, which is not the same
/// as running and exiting 0 — and no shell records the difference.
#[test]
fn a_short_circuited_link_is_recorded_as_never_run() {
    let segments = run("true && false && echo never");
    assert_eq!(segments.len(), 3, "{segments:?}");
    assert_eq!(segments[0].status, Some(0));
    assert_eq!(segments[1].status, Some(1));
    assert_eq!(segments[2].status, None, "the third link never ran");
    assert!(!segments[2].ran());
    assert_eq!(segments[2].text, "echo never");
}

/// `||` short-circuits the other way, and the recorder must not assume `&&`.
#[test]
fn an_or_chain_skips_the_link_after_a_success() {
    let segments = run("true || echo never");
    assert_eq!(segments.len(), 2, "{segments:?}");
    assert_eq!(segments[0].status, Some(0));
    assert_eq!(segments[1].status, None);
    assert_eq!(segments[1].join, Join::Or);
}

/// The resumable line is the failed link and everything after it, operators included.
#[test]
fn a_broken_chain_can_be_resumed_from_where_it_stopped() {
    run("true && false && echo never");
    assert_eq!(
        segments::resumable().as_deref(),
        Some("false && echo never")
    );
}

/// A chain that finished has nothing to resume, and neither does a lone failing command — there is
/// no *from* in either case.
#[test]
fn nothing_to_resume_when_the_chain_finished_or_never_branched() {
    run("true && true");
    assert_eq!(segments::resumable(), None);

    run("false");
    assert_eq!(segments::resumable(), None, "one command is not a chain");
}

/// Several statements on one line are one line: `a; b && c` is three links.
#[test]
fn sequential_statements_are_links_of_the_same_line() {
    let segments = run("true; true && true");
    assert_eq!(segments.len(), 3, "{segments:?}");
}

/// **A chain inside a compound is not the line you typed.** Recording it would interleave two
/// different chains, so `if` bodies, functions and loops record nothing.
#[test]
fn a_chain_inside_a_compound_does_not_record() {
    let segments = run("if true; then false && echo inner; fi");
    assert!(
        segments.iter().all(|s| !s.text.contains("echo inner")),
        "the compound's own chain leaked into the line's: {segments:?}"
    );
}

/// A pipeline is one link, however many stages it has — `$PIPESTATUS` owns the level below.
#[test]
fn a_pipeline_is_a_single_link() {
    let segments = run("true | true && true");
    assert_eq!(segments.len(), 2, "{segments:?}");
    assert_eq!(segments[0].text, "true | true");
}

/// Nothing is recorded unless the read loop asked. A script pays nothing and reports nothing.
#[test]
fn a_shell_that_was_not_armed_records_nothing() {
    let mut env = Environment::new();
    let ast = parse_bash_script("true && false").expect("parses");
    segments::arm();
    segments::disarm();
    let _ = eval_command_list(&mut env, &ast);
    assert!(
        segments::taken().is_empty(),
        "an unarmed shell must not record"
    );
}

/// A pipeline's stages are recorded under the link they belong to.
///
/// `$PIPESTATUS` has the same numbers, and keeps them; this keeps them beside the text that
/// produced each one, which is what makes "which stage failed" answerable without counting.
#[test]
fn a_pipeline_records_its_stages_under_its_link() {
    let segments = run("true | false | true");
    assert_eq!(segments.len(), 1, "one link: {segments:?}");
    let stages = &segments[0].stages;
    assert_eq!(stages.len(), 3, "{stages:?}");
    assert_eq!(stages[0].text, "true");
    assert_eq!(stages[1].text, "false");
    assert_eq!(
        stages[1].status, 1,
        "the middle stage is the one that failed"
    );
    assert_eq!(stages[2].status, 0);
}

/// A link that is one command has no stages: a single-entry list would be noise on every line
/// anyone ever types.
#[test]
fn a_link_of_one_command_has_no_stages() {
    let segments = run("true && true");
    assert!(segments.iter().all(|s| s.stages.is_empty()), "{segments:?}");
}

/// Stages belong to the link that produced them, not to whichever link is pushed next.
#[test]
fn stages_do_not_bleed_into_the_next_link() {
    let segments = run("true | true && true");
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].stages.len(), 2, "the pipeline's own");
    assert!(
        segments[1].stages.is_empty(),
        "the plain link must not inherit them: {:?}",
        segments[1]
    );
}
