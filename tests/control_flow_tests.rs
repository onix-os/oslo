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

// --- R8.10: case fallthrough and re-test ---

/// `;&` runs the *next* branch's body without consulting its pattern. Executing it as `;;` — as
/// rush did — silently skipped every branch the script chained onto the match.
#[test]
fn semicolon_ampersand_falls_through_to_the_next_branch() {
    assert_out(
        "case a in a) echo one;& b) echo two;; c) echo three;; esac",
        "one\ntwo",
    );
}

#[test]
fn fallthrough_chains_through_several_branches() {
    assert_out(
        "case a in a) echo one;& b) echo two;& c) echo three;; esac",
        "one\ntwo\nthree",
    );
}

/// A branch reached by fallthrough runs even though its own pattern does not match.
#[test]
fn a_branch_reached_by_fallthrough_ignores_its_own_pattern() {
    assert_out("case a in a) echo one;& zzz) echo two;; esac", "one\ntwo");
}

/// `;&` on the last branch has nothing to fall into and simply ends the case.
#[test]
fn fallthrough_on_the_last_branch_ends_the_case() {
    assert_out("case a in a) echo one;& esac; echo after", "one\nafter");
}

/// `;;&` keeps testing the remaining patterns, so several branches can run — but only the ones
/// that actually match.
#[test]
fn double_semicolon_ampersand_keeps_matching() {
    assert_out(
        "case abc in a*) echo first;;& *c) echo second;;& zzz) echo third;; esac",
        "first\nsecond",
    );
}

#[test]
fn re_test_stops_at_the_first_branch_that_ends_with_a_plain_terminator() {
    assert_out(
        "case abc in a*) echo first;;& *c) echo second;; ab*) echo third;; esac",
        "first\nsecond",
    );
}

/// The status of a `case` is the status of the last body it ran, not of the failed match that
/// followed it.
#[test]
fn re_test_reports_the_status_of_the_last_body_that_ran() {
    assert_out("case a in a) true;;& zzz) false;; esac; echo $?", "0");
    assert_out("case a in a) false;;& zzz) true;; esac; echo $?", "1");
}

#[test]
fn a_case_that_matches_nothing_is_still_a_success() {
    assert_out("case a in b) echo no;; esac; echo $?", "0");
}

// --- R8.2: `(( expr ))` as a command ---

/// The construct exists to have side effects on the shell that ran it. Approximating it as a
/// subshell — which is what the deleted fallback parser did — lost every assignment.
#[test]
fn an_arithmetic_command_assigns_in_the_current_shell() {
    assert_out("x=5; ((x++)); echo $x", "6");
    assert_out("((y = 7)); echo $y", "7");
    assert_out("n=0; for i in a b c; do ((n++)); done; echo $n", "3");
}

/// The status is inverted with respect to the value: non-zero is success.
#[test]
fn the_status_of_an_arithmetic_command_is_inverted() {
    assert_out("((1)); echo $?", "0");
    assert_out("((0)); echo $?", "1");
    assert_out("((-1)); echo $?", "0");
    assert_out("if ((3 > 5)); then echo yes; else echo no; fi", "no");
}

#[test]
fn an_arithmetic_command_drives_a_while_loop() {
    assert_out(
        "i=0; while ((i < 3)); do echo $i; ((i += 1)); done",
        "0\n1\n2",
    );
}

/// A bad expression fails the command, not the shell: bash reports it, leaves `$?` at 1, and
/// carries on — unlike `$(( … ))` in a word, which is fatal.
#[test]
fn a_bad_arithmetic_command_does_not_stop_the_script() {
    let r = run("((1 +)); echo after=$?");
    assert_eq!(r.out(), "after=1", "stderr: {}", r.stderr);
    assert!(!r.stderr.is_empty(), "the failure must be reported");
}

// --- R8.3: `for ((init; cond; step))` ---

#[test]
fn an_arithmetic_for_loop_counts() {
    assert_out("for ((i = 0; i < 3; i++)); do echo $i; done", "0\n1\n2");
}

/// A condition that is false at the start means zero iterations, not one.
#[test]
fn an_arithmetic_for_loop_can_run_zero_times() {
    assert_out(
        "for ((i = 5; i < 3; i++)); do echo $i; done; echo done",
        "done",
    );
}

/// An absent condition is *true*. Reading it as the empty expression, and so as 0, would turn the
/// idiomatic infinite loop into a no-op.
#[test]
fn an_absent_condition_loops_until_something_breaks_it() {
    assert_out(
        "n=0; for (( ; ; )); do ((n++)); ((n >= 3)) && break; done; echo $n",
        "3",
    );
}

/// `continue` still runs the step expression; if it did not, a counting loop would spin forever.
#[test]
fn continue_in_an_arithmetic_for_loop_still_steps() {
    assert_out(
        "for ((i = 0; i < 5; i++)); do ((i == 2)) && continue; echo $i; done",
        "0\n1\n3\n4",
    );
}

#[test]
fn break_leaves_an_arithmetic_for_loop_with_a_successful_status() {
    assert_out(
        "for ((i = 0; ; i++)); do echo $i; ((i >= 2)) && break; done; echo st=$?",
        "0\n1\n2\nst=0",
    );
}

#[test]
fn break_two_escapes_a_nested_arithmetic_for_loop() {
    assert_out(
        "for ((a = 0; a < 3; a++)); do for ((b = 0; b < 3; b++)); do ((b == 1)) && break 2; echo $a.$b; done; done",
        "0.0",
    );
}

/// The loop variable keeps whatever the step last wrote, exactly as in bash.
#[test]
fn the_loop_variable_survives_the_loop() {
    assert_out("for ((i = 0; i < 3; i++)); do :; done; echo $i", "3");
}
