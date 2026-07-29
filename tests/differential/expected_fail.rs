//! The two lists that make the differential suite a ratchet.
//!
//! `EXPECTED_FAIL` names every corpus case rush currently gets wrong, with the PLAN.md finding
//! that explains it. Two rules keep the list from rotting:
//!
//! * a case that is not listed and does not match bash fails the suite — no new divergence
//!   lands unnoticed;
//! * a case that is listed and *does* match bash also fails the suite — so closing a finding
//!   means deleting its line here, and a round is done when its entries are gone.
//!
//! `UNFILED` is not a plan ID: it marks a divergence the audit did not enumerate. Those are
//! genuine bugs with no scheduled owner yet.
//!
//! `KNOWN_DIVERGENT` is the escape hatch for cases where bash is not a valid oracle at all.
//! Entries there are skipped, never compared, and each one needs a reason. Keep the two lists
//! separate: "we are wrong" and "the comparison is meaningless" are different claims.

/// Corpus file, PLAN.md finding ID, and what rush does instead.
///
/// One row per line, deliberately: closing a finding is a one-line deletion, and rustfmt would
/// otherwise wrap each row across four lines and hide that.
#[rustfmt::skip]
pub const EXPECTED_FAIL: &[(&str, &str, &str)] = &[

    // --- Round 1: hangs, crashes, and data executed as code ---
    // Empty: every Round 1 finding now matches bash.

    // --- Round 2: quoting, fields, and parameter expansion ---
    // Empty: the expansion operators match bash, and so does the status a fatal expansion error
    // leaves a non-interactive shell with (127, decided by `ShellError::fatal_exit_status`).

    // --- Round 3: arithmetic ---
    // Empty: the operator ladder matches bash, and a fatal arithmetic error exits 127 with it.

    // --- Round 4: exit status, descriptors, and subshell state ---
    // Empty: exit codes survive subshells, pipeline stages and background jobs; `$?` is written
    // after every pipeline; a signalled child reports 128 + signo.

    // --- Round 5: builtins conformance ---
    ("builtin_test_and_or.sh", "R5.1", "every 4+-operand test expression is true"),
    ("builtin_test_bad_operator.sh", "R5.1", "an unparseable expression is true instead of exit 2"),
    ("builtin_test_negation.sh", "R5.2", "! is only handled in the 2-operand form"),
    ("builtin_test_file_predicates.sh", "R5.3", "-s is silently false"),
    ("builtin_test_unreadable.sh", "R5.3", "-r uses stat() instead of access()"),
    ("builtin_test_nonnumeric.sh", "R5.3", "a non-numeric operand becomes 0 instead of an error"),
    ("redir_stderr_to_file.sh", "R5.3", "-s is silently false"),
    ("builtin_read_ifs.sh", "R5.4", "read splits on whitespace and ignores IFS"),
    ("builtin_function_shadows_builtin.sh", "R5.6", "builtins are resolved before functions"),
    ("builtin_command_v.sh", "R5.7", "command is not implemented"),
    ("builtin_command_bypass.sh", "R5.7", "command is not implemented"),
    ("builtin_exec_redirect.sh", "R5.7", "exec is not implemented"),
    ("builtin_getopts.sh", "R5.7", "getopts is not implemented"),
    ("builtin_colon.sh", "R5.8", ": is not implemented"),
    ("builtin_kill_signal_zero.sh", "R5.9", "kill -0 terminates the process it should probe"),
    ("builtin_kill_bad_signal.sh", "R5.9", "an unparseable signal name still sends SIGTERM"),
    ("builtin_unset.sh", "R5.10", "unset -f cannot remove a function"),
    ("builtin_readonly.sh", "R5.10", "assignment to a readonly variable succeeds"),
    ("builtin_echo_n.sh", "R5.11", "only a single leading -n is recognised"),
    ("builtin_echo_escapes.sh", "R5.11", "-e and -E are printed as data"),
    ("status_not_executable.sh", "R5.13", "a non-executable directory operand silently cds"),
    ("builtin_type_kinds.sh", "R5.14", "type has no -t"),
    ("builtin_umask_bad.sh", "R5.16", "an out-of-range mask silently succeeds"),
    ("builtin_exit_nonnumeric.sh", "R5.17", "exit abc succeeds instead of exiting 2"),

    // --- Round 6: shell options and traps ---
    ("options_errexit.sh", "R6.1", "set -e is parsed as a positional parameter"),
    ("options_set_multiple.sh", "R6.1", "set -eu is parsed as a positional parameter"),
    ("options_dollar_dash.sh", "R6.1", "$- is not a parameter"),
    ("options_nounset.sh", "R6.3", "nounset has no implementation point"),
    ("options_xtrace.sh", "R6.3", "xtrace has no implementation point"),
    ("options_noglob.sh", "R6.3", "noglob has no implementation point"),
    ("redir_noclobber.sh", "R6.3", "noclobber has no implementation point"),
    ("options_pipefail.sh", "R6.4", "pipefail has no implementation point"),
    ("trap_exit.sh", "R6.5", "no caller ever reads a trap handler"),
    ("trap_exit_on_exit_call.sh", "R6.5", "no caller ever reads a trap handler"),

    // --- Round 7: job control ---
    ("builtin_wait_status.sh", "R7.5", "wait discards the child's status"),

    // --- Round 8: missing language features ---
    ("arith_command.sh", "R8.2", "((expr)) is rejected by the adapter"),
    ("redir_heredoc_fallback_not_executed.sh", "R8.2", "((expr)) is rejected, so the whole script is a syntax error — the heredoc body is never run either way"),
    ("arith_while_condition.sh", "R8.2", "((expr)) is rejected by the adapter"),
    ("arith_for_loop.sh", "R8.3", "for ((;;)) is rejected by the adapter"),
    ("syntax_unsupported_process_substitution.sh", "R8.4", "a process substitution argument is deleted from argv"),
    ("control_case_fallthrough.sh", "R8.10", ";& behaves as ;;"),

    // --- divergences the audit did not enumerate ---
    ("redir_heredoc.sh", "UNFILED", "an unquoted heredoc body is not parameter-expanded"),
    ("redir_herestring_expansion.sh", "UNFILED", "a here-string word is not expanded, only unquoted"),
    ("status_command_not_found.sh", "UNFILED", "the not-found diagnostic ignores the command's 2> redirection"),
    ("robust_special_builtin_failure.sh", "UNFILED", "a failing special builtin does not exit a POSIX-mode shell"),
    ("expansion_brace_precedes_params.sh", "UNFILED", "brace expansion works on word parts, so `{$v,y}z` cannot fuse into the name `$vz` the way bash's textual pass does"),
];

/// Corpus file and why bash cannot arbitrate it. Empty is the healthy state.
pub const KNOWN_DIVERGENT: &[(&str, &str)] = &[];
