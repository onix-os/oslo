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
    ("quoting_double_quote_then_glob.sh", "R2.1", "one word-level quoted flag suppresses globbing"),
    ("quoting_backslash_star.sh", "R2.1", "a backslash-escaped * still globs"),
    ("quoting_backslash_space.sh", "R2.1", "a backslash-escaped space still splits the field"),
    ("quoting_literal_not_split.sh", "R2.1", "literal text is field-split on IFS"),
    ("expansion_glob_directory.sh", "R2.1", "\"$d\"/*.txt never globs"),
    ("expansion_cmdsub_strips_newlines.sh", "R2.1", "quoted substitution output is still field-split"),
    ("expansion_dollar_at_quoted.sh", "R2.2", "\"$@\" collapses to a single field"),
    ("expansion_at_forwarding.sh", "R2.2", "\"$@\" collapses to a single field"),
    ("builtin_set_positional.sh", "R2.2", "\"$@\" collapses to a single field"),
    ("expansion_backticks.sh", "R2.3", "backticks are not scanned at all"),
    ("expansion_colonless_forms.sh", "R2.4", "${x-d} / ${x+s} are unimplemented and expand to empty"),
    ("expansion_length.sh", "R2.4", "${#} expands to empty instead of the positional count"),
    ("expansion_substring.sh", "R2.4", "${v:off:len} is unimplemented and expands to empty"),
    ("expansion_pattern_replace.sh", "R2.4", "${v/pat/rep} is unimplemented and expands to empty"),
    ("expansion_case_modification.sh", "R2.4", "${v^^} / ${v,,} are unimplemented and expand to empty"),
    ("expansion_indirect.sh", "R2.4", "${!name} is unimplemented and expands to empty"),
    ("expansion_param_error.sh", "R2.4", "a fatal expansion error exits 1, bash exits 127"),
    ("syntax_bad_substitution_body.sh", "R2.4", "a parse error inside $( ) exits 2, bash exits 127 for a fatal expansion error"),
    ("expansion_prefix_strip.sh", "R2.5", "# and ## are unanchored substring searches, not patterns"),
    ("expansion_suffix_strip.sh", "R2.5", "% and %% are unanchored and pick the wrong length"),
    ("expansion_param_default.sh", "R2.6", "the ${x:-word} payload is used as raw text, unexpanded"),
    ("expansion_nested_braces.sh", "R2.6", "the brace scanner stops at the first }"),
    ("expansion_cmdsub_nested_quotes.sh", "R2.7", "the re-lexer returns the source text verbatim on failure"),
    ("expansion_ansi_c_quoting.sh", "R2.8", "$'...' is not decoded"),
    ("expansion_ifs_ansi_c.sh", "R2.8", "IFS=$'\n' sets IFS to three literal characters"),
    ("expansion_assignment_no_split.sh", "R2.9", "an assignment RHS is field-split and rejoined"),
    ("expansion_assignment_no_glob.sh", "R2.9", "an assignment RHS is globbed"),
    ("expansion_field_splitting.sh", "R2.10", "splitting ignores a non-default IFS"),
    ("expansion_ifs_empty_fields.sh", "R2.10", "empty fields between IFS characters are dropped"),
    ("expansion_ifs_whitespace.sh", "R2.10", "IFS whitespace runs are not collapsed into fields"),
    ("expansion_dollar_star.sh", "R2.10", "\"$*\" joins with a hardcoded space, not IFS[0]"),
    ("expansion_glob_dotfiles.sh", "R2.11", "* matches dotfiles"),
    ("expansion_brace_list.sh", "R2.14", "brace expansion is absent"),
    ("expansion_brace_range.sh", "R2.14", "brace expansion is absent"),

    // --- Round 3: arithmetic ---
    ("arith_comparison_operators.sh", "R3.1", "comparison operators return the left operand"),
    ("arith_bitwise_operators.sh", "R3.1", "bitwise operators return the left operand"),
    ("arith_logical_operators.sh", "R3.1", "logical operators return the left operand"),
    ("arith_ternary_and_comma.sh", "R3.1", "?: and , return the left operand"),
    ("arith_precedence.sh", "R3.1", "the precedence ladder is two levels deep"),
    ("arith_division_by_zero.sh", "R3.1", "a fatal arithmetic error exits 1, bash exits 127"),
    ("arith_assignment.sh", "R3.2", "arithmetic assignment is structurally impossible"),
    ("arith_increment.sh", "R3.2", "++ and -- are hard errors"),
    ("arith_bases.sh", "R3.3", "only decimal literals are recognised"),
    ("arith_variable_operands.sh", "R3.4", "operands resolve by one round of text substitution"),
    ("arith_nested_substitution.sh", "R3.4", "$( ) and ${#s} inside $(( )) abort the command"),
    ("control_function_recursion.sh", "R3.4", "$(($1 - 1)) does not resolve a positional operand"),

    // --- Round 4: exit status, descriptors, and subshell state ---
    ("control_subshell_inherits.sh", "R4.1", "the forked child rebuilds Environment::new()"),
    ("expansion_cmdsub_function.sh", "R4.1", "a command substitution cannot see the shell's functions"),
    ("builtin_exit_status.sh", "R4.2", "( exit n ) collapses to 1"),
    ("status_subshell.sh", "R4.2", "( exit n ) collapses to 1"),
    ("status_exit_in_pipeline.sh", "R4.2", "exit n in a pipeline stage collapses to 1"),
    ("status_and_or_updates.sh", "R4.4", "$? is not updated between and-or members"),
    ("status_assignment_substitution.sh", "R4.4", "an assignment-only command always reports 0"),
    ("status_function.sh", "R4.4", "$? is not updated between and-or members"),
    ("redir_fd_not_leaked.sh", "R4.5", "saved descriptors are not CLOEXEC and leak into children"),
    ("redir_bad_fd.sh", "R4.8", "a redirection error aborts the whole script"),
    ("redir_missing_input_builtin.sh", "R4.8", "a redirection error on a builtin aborts the whole script"),
    ("status_background.sh", "R4.9", "the [bg] notice is printed unconditionally on stdout"),

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
];

/// Corpus file and why bash cannot arbitrate it. Empty is the healthy state.
pub const KNOWN_DIVERGENT: &[(&str, &str)] = &[];
