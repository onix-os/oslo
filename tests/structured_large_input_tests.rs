//! A structured pipeline whose byte prefix produces more than a pipe holds.
//!
//! The prefix ran to completion with stdout on a pipe, and only *then* was the pipe read. A pipe
//! holds 64 KiB, so the moment the prefix produced more than that it blocked in `write`, and the
//! thing that would have drained it was waiting for the prefix to exit. `cat big.json | from json`
//! hung for ever — at exactly one byte over the capacity, which is why nothing under 64 KiB ever
//! showed it and no test caught it.

mod common;

use std::io::Write;

/// Bigger than any pipe: Linux's default capacity is 64 KiB, and this is several times that.
const ROWS: usize = 20_000;

fn lines_fixture(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("big.txt");
    let mut file = std::fs::File::create(&path).expect("fixture");
    for i in 0..ROWS {
        writeln!(file, "line{i:06}").expect("write");
    }
    path
}

/// **The case it hung on.** Counting the rows means the whole prefix has to arrive.
#[test]
fn a_prefix_larger_than_a_pipe_still_completes() {
    let dir = tempfile::tempdir().expect("tempdir");
    lines_fixture(dir.path());

    let run = common::run_in(dir.path(), "cat big.txt | lines | length");
    assert_eq!(run.out(), ROWS.to_string(), "stderr: {}", run.stderr);
    assert_eq!(run.status, 0);
}

/// The module's own documented headline — `… -o json | from json | where …` — for a document that
/// any real command would produce.
#[test]
fn a_json_document_larger_than_a_pipe_still_parses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("rows.json");
    let mut file = std::fs::File::create(&path).expect("fixture");
    write!(file, "[").expect("write");
    for i in 0..ROWS {
        if i > 0 {
            write!(file, ",").expect("write");
        }
        write!(file, "{{\"id\":{i},\"name\":\"n{i}\"}}").expect("write");
    }
    write!(file, "]").expect("write");
    drop(file);
    assert!(
        std::fs::metadata(&path).expect("stat").len() > 65_536,
        "the fixture has to be bigger than a pipe or it proves nothing"
    );

    let run = common::run_in(dir.path(), "cat rows.json | from json | length");
    assert_eq!(run.out(), ROWS.to_string(), "stderr: {}", run.stderr);
    assert_eq!(run.status, 0);
}

/// Exactly at the boundary and exactly one byte past it — the two sizes that used to differ.
#[test]
fn the_pipe_capacity_is_not_a_boundary_any_more() {
    let dir = tempfile::tempdir().expect("tempdir");
    lines_fixture(dir.path());

    for bytes in [65_536usize, 65_537, 131_072] {
        let run = common::run_in(
            dir.path(),
            &format!("head -c {bytes} big.txt | lines | length"),
        );
        assert_eq!(run.status, 0, "{bytes} bytes: {}", run.stderr);
        let counted: usize = run.out().parse().unwrap_or_else(|_| {
            panic!("{bytes} bytes gave {:?}, stderr {}", run.out(), run.stderr)
        });
        assert!(counted > 0, "{bytes} bytes counted nothing");
    }
}

/// A prefix that fails still reports its status rather than hanging or losing it.
#[test]
fn a_failing_prefix_still_reports() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = common::run_in(dir.path(), "cat nosuchfile | lines | length; echo rc=$?");
    assert!(run.out().contains("rc="), "stderr: {}", run.stderr);
}

/// **An upstream with no end is read as it arrives.**
///
/// `yes | lines | first 2` used to reach 4.4 GB of resident memory in three seconds, then — once the
/// read was bounded — to fail at 256 MiB. Neither is what `yes | head -2` does, which is to answer
/// instantly: `head` takes its two lines and exits, and `yes` dies of the `SIGPIPE` that follows.
///
/// The structured half does that now. `first` counts across batches, and the batch that satisfies it
/// closes the reader — which is the same mechanism, arriving at last.
#[test]
fn an_endless_upstream_is_read_as_it_arrives() {
    let started = std::time::Instant::now();
    let run = common::run_in(std::path::Path::new("."), "yes | lines | first 2");

    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.out().trim(), "y\ny", "two lines, and only two");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "took {:?}, so the upstream was not let go",
        started.elapsed()
    );
}

/// **And the cap still guards what cannot stream.**
///
/// `sort-by` cannot answer its first row before it has seen the last, so a pipeline containing one
/// materialises however endless its upstream is. That is not a gap in the streaming — it is the
/// reason the bound has to stay.
#[test]
fn an_endless_upstream_that_cannot_stream_is_still_refused() {
    let started = std::time::Instant::now();
    let run = common::run_in(std::path::Path::new("."), "yes | lines | sort-by line");

    assert_ne!(
        run.status, 0,
        "an upstream that cannot be read is a failure"
    );
    assert!(
        run.stderr.contains("MiB"),
        "the message names the cap: {}",
        run.stderr
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(60),
        "took {:?}, which means the bound is not being enforced",
        started.elapsed()
    );
    assert!(
        run.out().trim().is_empty(),
        "no rows escape a truncated read, got {:?}",
        run.out()
    );
}
