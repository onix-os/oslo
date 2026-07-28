//! Control flow: `case`, `if`/`elif`, `[[ ]]`, and loop control.
//!
//! Spawns the real binary; see `common/mod.rs` for why.

mod common;

use common::{assert_out, run};

// --- case ---

#[test]
fn case_matches_a_literal_pattern() {
    assert_out(
        "case abc in abc) echo MATCHED;; *) echo NO;; esac",
        "MATCHED",
    );
}

#[test]
fn case_matches_a_glob_pattern() {
    assert_out("case foo in bar) echo B;; f*) echo GLOB;; esac", "GLOB");
}

#[test]
fn case_patterns_are_not_expanded_against_the_filesystem() {
    // With pathname expansion wrongly applied, `f*` would expand to the files in the working
    // directory and stop matching the subject.
    assert_out(
        "touch f1 f2; case foo in f*) echo GLOB;; *) echo NO;; esac",
        "GLOB",
    );
}

#[test]
fn case_supports_alternate_patterns() {
    assert_out("case b in a|b|c) echo MULTI;; esac", "MULTI");
}

#[test]
fn case_falls_through_to_the_default() {
    assert_out("case zzz in a) echo A;; *) echo DEFAULT;; esac", "DEFAULT");
}

#[test]
fn case_with_no_match_is_a_no_op() {
    let r = run("case zzz in a) echo A;; esac; echo done");
    assert_eq!(r.out(), "done");
    assert_eq!(r.status, 0);
}

// --- if / elif ---

#[test]
fn second_elif_branch_is_reachable() {
    assert_out(
        "X=3; if [ $X -eq 1 ]; then echo one; \
         elif [ $X -eq 2 ]; then echo two; \
         elif [ $X -eq 3 ]; then echo three; else echo other; fi",
        "three",
    );
}

#[test]
fn else_is_reached_after_several_elifs() {
    assert_out(
        "X=9; if [ $X -eq 1 ]; then echo one; \
         elif [ $X -eq 2 ]; then echo two; \
         elif [ $X -eq 3 ]; then echo three; else echo other; fi",
        "other",
    );
}

#[test]
fn first_matching_branch_wins() {
    assert_out(
        "X=1; if [ $X -eq 1 ]; then echo one; elif [ $X -eq 1 ]; then echo two; fi",
        "one",
    );
}

// --- [[ ]] ---

#[test]
fn extended_test_reports_false() {
    assert_out("[[ 1 == 2 ]] && echo T || echo F", "F");
}

#[test]
fn extended_test_reports_true() {
    assert_out("[[ a == a ]] && echo T || echo F", "T");
}

#[test]
fn extended_test_unquoted_rhs_is_a_glob() {
    assert_out("[[ abc == a* ]] && echo T || echo F", "T");
}

#[test]
fn extended_test_quoted_rhs_is_literal() {
    assert_out(r#"[[ abc == "a*" ]] && echo T || echo F"#, "F");
    assert_out(r#"[[ "a*" == "a*" ]] && echo T || echo F"#, "T");
}

#[test]
fn extended_test_inequality() {
    assert_out("[[ a != b ]] && echo T || echo F", "T");
    assert_out("[[ a != a ]] && echo T || echo F", "F");
}

#[test]
fn extended_test_negation() {
    assert_out("[[ ! -f /nonexistent-xyz ]] && echo T || echo F", "T");
}

#[test]
fn extended_test_conjunction_and_disjunction() {
    assert_out("[[ -d /tmp && -d / ]] && echo T || echo F", "T");
    assert_out("[[ -d /nonexistent-xyz && -d / ]] && echo T || echo F", "F");
    assert_out("[[ -d /nonexistent-xyz || -d / ]] && echo T || echo F", "T");
}

#[test]
fn extended_test_arithmetic_comparisons() {
    assert_out("[[ 5 -gt 3 ]] && echo T || echo F", "T");
    assert_out("[[ 3 -gt 5 ]] && echo T || echo F", "F");
    assert_out("[[ 3 -le 3 ]] && echo T || echo F", "T");
}

#[test]
fn extended_test_string_predicates() {
    assert_out("[[ -z '' ]] && echo T || echo F", "T");
    assert_out("[[ -n x ]] && echo T || echo F", "T");
}

#[test]
fn extended_test_file_predicates() {
    assert_out("touch a; [[ -f a ]] && echo T || echo F", "T");
    assert_out("mkdir d; [[ -d d ]] && echo T || echo F", "T");
    assert_out("[[ -e /nonexistent-xyz ]] && echo T || echo F", "F");
}

// --- loop control ---

#[test]
fn break_leaves_the_loop() {
    assert_out("for i in 1 2 3; do echo $i; break; done", "1");
}

#[test]
fn break_with_a_depth_leaves_several_loops() {
    assert_out(
        "for i in 1 2; do for j in a b; do echo $i$j; break 2; done; done",
        "1a",
    );
}

#[test]
fn continue_skips_the_rest_of_the_iteration() {
    assert_out(
        "for i in 1 2 3; do if [ $i -eq 2 ]; then continue; fi; echo $i; done",
        "1\n3",
    );
}

#[test]
fn break_works_in_a_while_loop() {
    assert_out(
        "i=0; while true; do i=$((i+1)); if [ $i -ge 3 ]; then break; fi; done; echo $i",
        "3",
    );
}

#[test]
fn return_sets_the_function_status() {
    assert_out("f() { return 5; }; f; echo $?", "5");
}

#[test]
fn return_stops_executing_the_function() {
    assert_out("f() { echo a; return 0; echo b; }; f", "a");
}

#[test]
fn loop_control_outside_a_loop_is_not_an_error() {
    let r = run("break; echo survived");
    assert_eq!(r.out(), "survived");
    assert_eq!(r.status, 0);
    assert!(r.stderr.is_empty(), "unexpected stderr: {}", r.stderr);
}

#[test]
fn break_does_not_escape_a_function_into_the_callers_loop() {
    assert_out(
        "f() { break; }; for i in 1 2 3; do f; echo $i; done",
        "1\n2\n3",
    );
}
