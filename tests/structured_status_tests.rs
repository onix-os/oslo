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
