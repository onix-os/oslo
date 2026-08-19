//! What a line did, round-tripped through a real store.

use super::*;
use crate::track::log::MODE_SHELL;

fn temp_db() -> (tempfile::TempDir, crate::track::Track) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("nested/history.db");
    let track = crate::track::Track::open(&path).expect("the store opens");
    (dir, track)
}

fn link(segment: u32, join: &str, text: &str, status: Option<i32>, ms: i64) -> Outcome {
    Outcome {
        segment,
        join: join.to_string(),
        text: text.to_string(),
        status,
        duration_ms: ms,
        dir_id: 0,
    }
}

/// The line and its links come back in order, joined to the log row by the id `append` answered.
#[test]
fn an_outcome_joins_the_line_it_belongs_to() {
    let (_dir, track) = temp_db();
    let id = track
        .append("make clean && make build", MODE_SHELL)
        .expect("an id");

    assert!(track.record_outcome(
        id,
        &[
            Outcome::line(7, Some(1), 417),
            link(1, "", "make clean", Some(0), 5),
            link(2, "&&", "make build", Some(1), 412),
        ],
    ));

    let rows = track.outcome_of(id);
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(rows[0].segment, 0);
    assert_eq!(rows[0].dir_id, 7, "the line knows where it ran");
    assert_eq!(rows[1].text, "make clean");
    assert_eq!(rows[2].join, "&&");
    assert_eq!(rows[2].status, Some(1));
}

/// **A link that never ran reads back as never having run**, not as a success and not as a
/// failure. Storing `None` as a real status would make it one of the two.
#[test]
fn a_link_that_never_ran_survives_the_round_trip() {
    let (_dir, track) = temp_db();
    let id = track.append("false && echo never", MODE_SHELL).expect("id");

    track.record_outcome(
        id,
        &[
            Outcome::line(0, Some(1), 3),
            link(1, "", "false", Some(1), 3),
            link(2, "&&", "echo never", None, 0),
        ],
    );

    let rows = track.outcome_of(id);
    assert!(rows[1].ran());
    assert_eq!(rows[2].status, None);
    assert!(!rows[2].ran(), "a skipped link must not read as success");
}

/// A status of zero is success, and must not be confused with the sentinel that means "no status".
#[test]
fn zero_is_a_status_and_not_an_absent_one() {
    let (_dir, track) = temp_db();
    let id = track.append("true", MODE_SHELL).expect("id");
    track.record_outcome(id, &[link(1, "", "true", Some(0), 1)]);
    assert_eq!(track.outcome_of(id)[0].status, Some(0));
}

/// One line's outcomes never reach another's, even though the ids are adjacent.
#[test]
fn outcomes_do_not_leak_between_lines() {
    let (_dir, track) = temp_db();
    let first = track.append("one", MODE_SHELL).expect("id");
    let second = track.append("two", MODE_SHELL).expect("id");

    track.record_outcome(first, &[link(1, "", "one", Some(0), 1)]);
    track.record_outcome(second, &[link(1, "", "two", Some(0), 2)]);

    assert_eq!(track.outcome_of(first).len(), 1);
    assert_eq!(track.outcome_of(first)[0].text, "one");
    assert_eq!(track.outcome_of(second)[0].text, "two");
}

/// A line with no outcome is not an error. It means the command is still running, the shell died,
/// or the line never reached execution — and a replay must be able to tell that from a failure.
#[test]
fn a_line_with_no_outcome_answers_empty() {
    let (_dir, track) = temp_db();
    let id = track.append("still going", MODE_SHELL).expect("id");
    assert!(track.outcome_of(id).is_empty());
}

/// **Trimming the log trims the outcomes with it.** An outcome whose line is gone is a row nothing
/// can ever join to, and the store has no `VACUUM` to reclaim it later.
#[test]
fn trimming_the_log_takes_the_outcomes_too() {
    let (_dir, track) = temp_db();
    let mut ids = Vec::new();
    for i in 1..=10 {
        let id = track.append(&format!("cmd {i}"), MODE_SHELL).expect("id");
        track.record_outcome(id, &[link(1, "", &format!("cmd {i}"), Some(0), 1)]);
        ids.push(id);
    }

    assert!(track.trim(3));

    assert!(
        track.outcome_of(ids[0]).is_empty(),
        "the oldest line's outcome went with it"
    );
    assert!(
        !track.outcome_of(ids[9]).is_empty(),
        "the newest line kept its outcome"
    );
}

/// **The one call a replay makes.** Oldest first, each line joined to what it did, in a single
/// transaction so all three buckets are read from one snapshot.
#[test]
fn observations_come_back_in_order_with_what_each_line_did() {
    let (_dir, track) = temp_db();
    let first = track.append("true && false", MODE_SHELL).expect("id");
    track.record_outcome(
        first,
        &[
            Outcome::line(3, Some(1), 9),
            link(1, "", "true", Some(0), 1),
            link(2, "&&", "false", Some(1), 8),
        ],
    );
    let second = track.append("echo done", MODE_SHELL).expect("id");
    track.record_outcome(second, &[Outcome::line(3, Some(0), 2)]);

    let (seen, _places) = track.observations(10);
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].line, "true && false", "oldest first");
    assert_eq!(seen[1].line, "echo done");

    assert_eq!(seen[0].status, Some(1));
    assert_eq!(seen[0].dir_id, 3);
    assert_eq!(seen[0].segments.len(), 2, "the links, not the line");
    assert_eq!(seen[0].segments[1].join, "&&");
    assert!(seen[1].segments.is_empty(), "not a chain");
}

/// A line still running has no outcome, and says so rather than reporting a status it never had.
/// A replay must skip it — and break the sequence there rather than splicing its neighbours.
#[test]
fn a_line_with_no_outcome_is_returned_but_unfinished() {
    let (_dir, track) = temp_db();
    track.append("sleep 300", MODE_SHELL).expect("id");
    let (seen, _) = track.observations(10);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].status, None, "not zero, and not a failure");
}

/// The boundary and the outcome in one transaction, which is what halves a command's commits.
///
/// The directory has to be the one the boundary *just* resolved — it is the whole reason the two
/// were separate writes, and the whole reason they need not be.
#[test]
fn a_boundary_stamps_the_outcome_with_the_directory_it_resolved() {
    use crate::track::{Step, Visit};

    let (_dir, track) = temp_db();
    let id = track.append("cargo build", MODE_SHELL).expect("an id");
    let step = Step {
        ran_in: Visit {
            path: "/w/project",
            root: None,
        },
        moved_to: None,
        dwell_ms: 0,
        run: None,
    };

    assert!(track.record_settled(&step, id, &[Outcome::line(0, Some(0), 12)]));

    let rows = track.outcome_of(id);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, Some(0));
    assert_ne!(rows[0].dir_id, 0, "the boundary resolved a directory");
    assert_eq!(
        rows[0].dir_id,
        track.current_dir_id(),
        "and the outcome names that one, not a second lookup's answer"
    );
}

/// The same answer by the longer road, for the lines whose boundary could not carry them.
#[test]
fn an_outcome_written_on_its_own_still_finds_the_directory() {
    use crate::track::{Step, Visit};

    let (_dir, track) = temp_db();
    let id = track.append("cargo test", MODE_SHELL).expect("an id");
    track.record(&Step {
        ran_in: Visit {
            path: "/w/project",
            root: None,
        },
        moved_to: None,
        dwell_ms: 0,
        run: None,
    });

    assert!(track.record_outcome_here(id, &[Outcome::line(0, Some(1), 3)]));

    let rows = track.outcome_of(id);
    assert_eq!(rows[0].dir_id, track.current_dir_id());
    assert_ne!(rows[0].dir_id, 0);
}

/// The session and its counter survive the join, which is what makes per-shell ordering possible.
#[test]
fn an_observation_carries_the_session_it_belongs_to() {
    let (_dir, track) = temp_db();
    track.append("one", MODE_SHELL);
    track.append("two", MODE_SHELL);
    let (seen, _) = track.observations(10);
    assert_eq!(seen[0].session, seen[1].session);
    assert!(seen[1].seq > seen[0].seq);
    assert!(seen[1].id > seen[0].id, "the id is the global order");
}
