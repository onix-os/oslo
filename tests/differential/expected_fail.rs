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
    ("builtin_exec_redirect.sh", "R5.7", "`exec 3> file` closes fd 3: exec::simple now builds the non-restoring guard, but redirect.rs `apply` opens the file (which lands on fd 3), does a no-op dup2(3,3), then drops the File and closes it"),
    ("builtin_unset.sh", "R5.10", "unset -f cannot remove a function"),
    ("builtin_readonly.sh", "R5.10", "assignment to a readonly variable succeeds"),

    // --- Round 6: shell options and traps ---
    // Empty: every option in the `set -o` table that has behaviour now acts on it, and traps are
    // both stored and run — the EXIT handler on every exit path, signals at command boundaries.

    // --- Round 7: job control ---
    // Empty for `wait` (R7.5): a pid operand reports the child's own status, a signalled child
    // reports 128 + signo, an unknown pid is 127, and `-n` and `%n` operands work. What remains
    // in Round 7 needs a pty and is covered by the job-control integration tests instead.

    // --- Round 8: missing language features ---
    ("array_whole_operators.sh", "R8.1", "an operator applied to a whole array — `${a[@]:1}` slicing, `${a[@]#pat}` element-wise — is rejected loudly instead of evaluated; every other array form matches bash"),
    ("arith_for_unspaced_sections.sh", "R8.3", "brush 0.4 tokenizes the `;;` in `for ((;;))` as the case terminator, so the idiomatic unspaced infinite loop is a parse error; `for (( ; ; ))` works"),
    ("syntax_unsupported_process_substitution.sh", "R8.4", "process substitution is refused by name (step 1); the `/dev/fd/N` implementation is step 2 and deferred"),
    ("syntax_unsupported_coproc.sh", "R8.5", "coproc is refused by name and deliberately not implemented — it needs job control; bash runs the body and exits 0"),
    ("syntax_unsupported_select.sh", "R8.6", "select is refused by name and deliberately not implemented — it needs a prompt, PS3 and REPLY; bash runs the loop and reads EOF"),

    // --- divergences the audit did not enumerate ---
    ("redir_heredoc.sh", "UNFILED", "an unquoted heredoc body is not parameter-expanded"),
    ("redir_herestring_expansion.sh", "UNFILED", "a here-string word is not expanded, only unquoted"),
    ("robust_special_builtin_failure.sh", "UNFILED", "a failing special builtin does not exit a POSIX-mode shell"),
    ("expansion_brace_precedes_params.sh", "UNFILED", "brace expansion works on word parts, so `{$v,y}z` cannot fuse into the name `$vz` the way bash's textual pass does"),
];

/// Corpus file and why bash cannot arbitrate it. Empty is the healthy state.
pub const KNOWN_DIVERGENT: &[(&str, &str)] = &[];
