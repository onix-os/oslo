//! A streamed pipeline and a materialised one must be indistinguishable.
//!
//! # Why this is a property rather than a list of cases
//!
//! Which pipelines stream is an *implementation detail*: `plan` says yes when the upstream is one
//! simple command, the bridge is `lines` or `parse`, and every verb after it is row-local, counting
//! or folding. Nobody writing a script knows or should have to know that. So any observable
//! difference between the two paths is a bug by definition — not a case to remember, but a rule.
//!
//! # The bugs that came before the rule
//!
//! Three shipped, and each was found by accident rather than by looking:
//!
//! * the stream path flattened every verb failure to 1, losing the `2` that "no such column"
//!   answers everywhere else;
//! * it set `PIPESTATUS` to a single element, so `${PIPESTATUS[1]}` was empty — silently, and only
//!   for pipelines that happened to be streamable;
//! * a satisfied `first n` stopped the whole chain rather than only the *reading*, so
//!   `first 2 | length` printed two rows where the materialised path answered `2`.
//!
//! The third was caught by the sweep that became this file, on its first run.
//!
//! # How the same pipeline is forced down the other path
//!
//! `plan` requires the upstream to be a simple command, so wrapping it in `{ …; }` makes an
//! otherwise identical pipeline materialise. Nothing else changes: same verbs, same input, same
//! order.

mod common;

/// Every verb the streaming path claims, alone and in the combinations where state crosses.
const PIPELINES: &[&str] = &[
    // Row-local.
    "where 'line:match(\"a\")'",
    "map 'line:upper()'",
    "cols line",
    "get line",
    "reject line",
    "rename line text",
    "flatten",
    "compact",
    "default line 'x'",
    "upsert tag '1'",
    // Counting.
    "first 2",
    "skip 1",
    "every 2",
    "enumerate",
    // Folding.
    "length",
    "final 2",
    // **A count followed by anything.** `first 2 | length` is the one that was wrong: what a
    // satisfied count ends is the reading, not the pipeline.
    "first 2 | length",
    "first 2 | final 1",
    "first 2 | enumerate",
    "first 2 | cols line",
    "first 2 | where 'true'",
    "first 2 | upsert t '1'",
    "skip 1 | length",
    "every 2 | length",
    "enumerate | length",
    "final 2 | length",
    // Counts against each other, where both carry state.
    "first 3 | skip 1",
    "skip 1 | first 1",
    "enumerate | skip 1 | first 1",
    "first 2 | skip 1 | length",
    "skip 1 | every 2 | length",
    // Degenerate counts.
    "first 0 | length",
    "skip 99 | length",
    "final 0 | length",
    "every 1 | final 2",
    // Failures, whose *status* has to match as much as their output.
    "cols nope",
    "first 2 | cols nope",
    "where 'nosuchfn()'",
    "first 2 | where 'nosuchfn()'",
    // And the byte-shaped ends.
    "to csv",
    "first 1 | to json",
];

const INPUT: &str = "printf 'b 2\\na 1\\nc 3\\nd 4\\n'";

/// The same pipeline, streamed and materialised, must agree about everything observable.
#[test]
fn a_streamed_pipeline_answers_what_a_materialised_one_does() {
    let dir = tempfile::tempdir().expect("tempdir");

    for verbs in PIPELINES {
        let streamed = common::run_in(dir.path(), &format!("{INPUT} | lines | {verbs}"));
        // `plan` needs a simple command as the upstream, so a compound one materialises instead.
        let materialised = common::run_in(dir.path(), &format!("{{ {INPUT}; }} | lines | {verbs}"));

        assert_eq!(
            streamed.out(),
            materialised.out(),
            "stdout differs for `{verbs}`"
        );
        assert_eq!(
            streamed.err(),
            materialised.err(),
            "stderr differs for `{verbs}`"
        );
        assert_eq!(
            streamed.status, materialised.status,
            "status differs for `{verbs}`"
        );
    }
}

/// **`PIPESTATUS` describes the pipeline as written on both paths**, which is invariant I6. The
/// stream path reported one number for the whole pipeline, so `${PIPESTATUS[1]}` was empty exactly
/// when a pipeline happened to be streamable.
#[test]
fn pipestatus_agrees_across_the_two_paths() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = "echo \"${PIPESTATUS[0]}-${PIPESTATUS[1]}-${PIPESTATUS[2]}\"";

    let streamed = common::run_in(
        dir.path(),
        &format!("false | lines | length >/dev/null\n{report}"),
    );
    let materialised = common::run_in(
        dir.path(),
        &format!("{{ false; }} | lines | length >/dev/null\n{report}"),
    );

    assert_eq!(streamed.out(), materialised.out());
    assert_eq!(streamed.out(), "1-0-0", "and it describes the three stages");
}

/// `pipefail` is a property of the pipeline, not of the path it happens to run on.
#[test]
fn pipefail_agrees_across_the_two_paths() {
    let dir = tempfile::tempdir().expect("tempdir");

    for setting in ["set -o pipefail", "set +o pipefail"] {
        let streamed = common::run_in(
            dir.path(),
            &format!("{setting}\nfalse | lines | length >/dev/null\necho rc=$?"),
        );
        let materialised = common::run_in(
            dir.path(),
            &format!("{setting}\n{{ false; }} | lines | length >/dev/null\necho rc=$?"),
        );
        assert_eq!(streamed.out(), materialised.out(), "under `{setting}`");
    }
}
