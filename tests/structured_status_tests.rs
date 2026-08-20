//! What a structured pipeline says went wrong, and what it does with bytes it cannot read.
//!
//! The structured path was built for the middle of a pipeline and grew edges later, so the edges
//! are where it disagreed with the byte path it is supposed to be interchangeable with: a status
//! that ignored `pipefail`, a listing that reported a missing directory as an empty one, and a
//! drain that threw away a whole stage's output over a single byte.

mod common;

use std::io::Write;

/// **One bad byte must not cost the whole stage.**
///
/// `capture()` drained the byte prefix with `read_to_string`, which answers `InvalidData` on the
/// first non-UTF-8 byte and leaves the buffer *empty* — so a two-megabyte log with one stray byte
/// anywhere in it counted as zero rows, with no error and status 0. The head-position path in the
/// same file already read it lossily; now both do.
#[test]
fn a_byte_that_is_not_utf8_does_not_discard_the_stage() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("log.txt");
    let mut file = std::fs::File::create(&path).expect("fixture");
    for i in 0..200 {
        writeln!(file, "line{i}").expect("write");
    }
    // A byte no UTF-8 sequence can begin with, in the middle of otherwise ordinary text.
    file.write_all(b"tail\xffend\n").expect("write");
    drop(file);

    let run = common::run_in(dir.path(), "cat log.txt | lines | length");
    let counted: usize = run
        .out()
        .parse()
        .unwrap_or_else(|_| panic!("counted {:?}, stderr {}", run.out(), run.stderr));
    assert!(
        counted >= 201,
        "the stage was discarded rather than read: counted {counted}"
    );
}

/// `set -o pipefail` is a property of the pipeline, not of the path it happens to run on.
///
/// The structured path ended with `statuses.last()` and never asked, so appending one structured
/// verb to an ordinary pipeline silently disarmed pipefail for the whole thing — and `set -e` with
/// it. Both spellings are asserted, because a fix that always reported the failure would be just as
/// wrong as one that never did.
#[test]
fn pipefail_reaches_a_structured_pipeline() {
    let dir = tempfile::tempdir().expect("tempdir");

    let on = common::run_in(
        dir.path(),
        "set -o pipefail\nfalse | lines | length\necho RC=$?",
    );
    assert!(
        on.out().contains("RC=1"),
        "pipefail did not carry the failure: {:?} {}",
        on.out(),
        on.stderr
    );

    let off = common::run_in(
        dir.path(),
        "set +o pipefail\nfalse | lines | length\necho RC=$?",
    );
    assert!(
        off.out().contains("RC=0"),
        "without pipefail a pipeline reports its last stage: {:?}",
        off.out()
    );
}

/// A directory that cannot be read is not an empty directory.
///
/// `ls_rows` answers an empty table for both, and `run_tool` reported status 0 unconditionally, so
/// `ls /nope | length` said `0` with nothing on stderr where the ordinary `ls` refuses. The status
/// is checked through `pipefail`, because a pipeline without it reports its *last* stage — `length`
/// succeeded, and saying otherwise would break the rule bash follows.
#[test]
fn a_directory_that_cannot_be_read_is_not_an_empty_one() {
    let dir = tempfile::tempdir().expect("tempdir");

    let quiet = common::run_in(dir.path(), "ls /nope/at/all | length");
    assert!(
        !quiet.stderr.is_empty(),
        "a missing directory was reported as an empty listing, silently"
    );

    let status = common::run_in(
        dir.path(),
        "set -o pipefail\nls /nope/at/all | length\necho RC=$?",
    );
    assert!(
        status.out().contains("RC=2"),
        "the refusal never reached the pipeline's status: {:?}",
        status.out()
    );

    // The ordinary case still answers, or the fix would have cost more than it bought.
    std::fs::write(dir.path().join("one"), b"").expect("fixture");
    std::fs::write(dir.path().join("two"), b"").expect("fixture");
    let listed = common::run_in(dir.path(), "ls . | length");
    assert_eq!(listed.out(), "2", "stderr: {}", listed.stderr);
}

/// **A redirection does not un-structure the pipeline.**
///
/// The planner treated a redirection on a stage as blocking the edge *into* it, so the whole
/// pipeline fell to the byte path — where `lines` and `length` are not commands at all.
/// `… | lines | length >/dev/null` answered `lines: command not found` and exited 127, which made
/// the structured verbs unusable in exactly the scripts that would test them: every natural way to
/// write "run this and check `$?`" ends in a redirection.
///
/// A redirection governs what a stage *writes*. Rows may reach it, and `structured::run` applies
/// the redirection around its own output.
#[test]
fn a_redirection_does_not_drop_the_pipeline_to_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("t.txt"), b"a\nb\n").expect("fixture");

    let quiet = common::run_in(
        dir.path(),
        "cat t.txt | lines | length >/dev/null\necho rc=$?",
    );
    assert!(
        quiet.out().contains("rc=0"),
        "the verbs were looked up as external commands: {:?} {}",
        quiet.out(),
        quiet.stderr
    );

    // And the redirection is honoured rather than merely tolerated.
    let written = common::run_in(dir.path(), "cat t.txt | lines | length > out.txt");
    assert!(written.stderr.is_empty(), "stderr: {}", written.stderr);
    let landed = std::fs::read_to_string(dir.path().join("out.txt")).expect("the file was written");
    assert_eq!(landed.trim(), "2", "the count never reached the file");

    // A redirection on a *producer* still forces text: its bytes went to the file, so the stage
    // after it has nothing to read, and pretending otherwise would be the opposite mistake.
    let upstream = common::run_in(dir.path(), "cat t.txt >/dev/null | lines | length");
    assert_eq!(upstream.out().trim(), "0", "stderr: {}", upstream.stderr);
}

/// **A tool may be followed by something that is not one.**
///
/// This was the missing half of the seam. An external command could *lead* — `cat x | lines |
/// length` has worked since the byte prefix was built — but a tool followed by a non-tool fell back
/// for the whole pipeline, and on that path the verbs are not commands:
///
/// ```text
/// $ ls | first 2 | cat
/// oslo: first: command not found
/// $ echo $?
/// 0
/// ```
///
/// Empty output reporting success, which is the failure `docs/known-gaps.md` opens by saying oslo
/// does not have. Worse, the structured stages had already run before the fallback re-ran the line
/// from the start, so a tool with a side effect performed it twice.
#[test]
fn a_tool_may_hand_over_to_a_byte_stage() {
    let dir = tempfile::tempdir().expect("tempdir");
    for name in ["alpha", "beta", "gamma"] {
        std::fs::write(dir.path().join(name), "x").expect("fixture");
    }

    let run = common::run_in(dir.path(), "ls | first 2 | cat");
    assert!(
        !run.stderr.contains("command not found"),
        "the verbs must not reach the byte path: {}",
        run.stderr
    );
    assert!(
        !run.stdout.trim().is_empty(),
        "the rows were rendered and handed over, so something must arrive"
    );
    assert_eq!(run.status, 0);

    // The count proves the *rows* crossed rather than the whole listing being re-run.
    let counted = common::run_in(dir.path(), "ls | first 2 | wc -l");
    assert_eq!(counted.stdout.trim(), "2", "stderr: {}", counted.stderr);

    // A verb after a verb after a non-verb: the handover is wherever the tools stop, not fixed.
    let named = common::run_in(dir.path(), "ls | first 3 | cols name | cat");
    assert!(named.stdout.contains("alpha"), "{}", named.stdout);
    assert!(named.stdout.contains("gamma"), "{}", named.stdout);
    assert!(
        !named.stderr.contains("command not found"),
        "{}",
        named.stderr
    );
}

/// **Never the drawn table.** What crosses into another program is the transport rendering — a
/// box-drawing character on somebody's standard input is the failure the whole design exists to
/// prevent.
#[test]
fn what_crosses_is_transport_not_a_table() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("alpha"), "x").expect("fixture");

    let run = common::run_in(dir.path(), "ls | first 1 | cat");
    for drawn in ['│', '─', '┌', '└', '├'] {
        assert!(
            !run.stdout.contains(drawn),
            "a table reached another program's stdin: {:?}",
            run.stdout
        );
    }
}

/// Every stage keeps its own status across the seam, so `PIPESTATUS` and `pipefail` still describe
/// the pipeline that was written rather than the halves it happened to run in.
#[test]
fn status_survives_the_handover() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("alpha"), "x").expect("fixture");

    let listed = common::run_in(
        dir.path(),
        r#"ls | first 1 | cat >/dev/null; echo "${PIPESTATUS[*]}""#,
    );
    assert_eq!(listed.stdout.trim(), "0 0 0", "stderr: {}", listed.stderr);

    let failed = common::run_in(
        dir.path(),
        r#"ls | first 1 | false; echo "${PIPESTATUS[*]}""#,
    );
    assert_eq!(failed.stdout.trim(), "0 0 1", "stderr: {}", failed.stderr);

    // `pipefail` is a property of the pipeline, not of the path it ran on.
    let strict = common::run_in(dir.path(), "set -o pipefail; ls | first 1 | false");
    assert_eq!(strict.status, 1);
}

/// **Nothing structured has run, so the byte path still gets the whole pipeline.** A bare `ls`
/// followed by anything is coreutils, exactly as it always was — the handover must not claim a
/// pipeline it never entered.
#[test]
fn a_pipeline_with_no_tool_edge_is_untouched() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("alpha"), "x").expect("fixture");

    let run = common::run_in(dir.path(), "ls | cat");
    assert_eq!(run.stdout.trim(), "alpha", "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
}

/// **A tool that prints, not just one that produces rows.**
///
/// `to json` writes with `println!` and hands back nothing, which is how `df | to json` puts JSON on
/// a terminal. Carrying only the *rows* across the seam therefore gave the byte suffix an empty
/// input while the JSON went straight to the shell's own stdout — `ps | to json | jq .` printed the
/// unfiltered document and jq silently did nothing to it.
#[test]
fn what_a_printing_tool_wrote_crosses_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("alpha"), "x").expect("fixture");

    // `grep -c` counts what reached it; if the JSON went past the suffix this is 0.
    let counted = common::run_in(dir.path(), "ls | first 1 | to json | grep -c name");
    assert_eq!(counted.stdout.trim(), "1", "stderr: {}", counted.stderr);

    let lines = common::run_in(dir.path(), "ls | first 1 | to json | wc -l");
    assert_ne!(lines.stdout.trim(), "0", "the suffix read nothing");

    // And with no suffix at all it still reaches the terminal rather than a scratch file.
    let alone = common::run_in(dir.path(), "ls | first 1 | to json");
    assert!(alone.stdout.contains("\"name\""), "{}", alone.stdout);
}
