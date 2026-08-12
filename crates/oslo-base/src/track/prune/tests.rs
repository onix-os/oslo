//! The sweep, against a real file.
//!
//! Its own file rather than a module at the bottom of `mod.rs`, which is the pattern `kv` and
//! `history_db` already use: the rules above are a page of policy and these are the cases that
//! pin each one, and neither reads better for being scrolled past to reach the other.

use super::*;
use crate::track::db::fixture::*;
use crate::track::{Run, Step, Visit};

/// Backdate a row to `age` seconds ago, which is the only way to test a rule written in days.
fn age_run(track: &Track, path: &str, argv: &str, age: i64) {
    track
        .store
        .write(|writer| {
            let id = super::super::db::lookup_dir(writer, path)?;
            let run = key::run(id, SH, argv);
            let mut row = RunRow::decode(&writer.get(Tree::Run, &run)?)?;
            row.last_at = now() - age;
            writer.put(Tree::Run, run, row.encode())
        })
        .expect("the row is backdated");
}

/// The same, for the mark that says a directory has gone.
fn age_mark(track: &Track, path: &str, age: i64) {
    track
        .store
        .write(|writer| {
            let id = super::super::db::lookup_dir(writer, path)?;
            let was = read_dir(writer, id)?;
            let mut row = was.clone();
            row.missing_since = Some(now() - age);
            put_dir(writer, id, Some(&was), &row)
        })
        .expect("the mark is backdated");
}

/// The rule that bounds the table, and the two halves of it that must not be confused: run once
/// and forgotten is rubbish; run twice, however long ago, is a habit.
#[test]
fn a_line_run_once_months_ago_goes_and_one_run_twice_stays() {
    let (_dir, track) = store();
    track.record(&ran("/w/alpha", "git commit -m wip", 0));
    track.record(&ran("/w/alpha", "cargo build", 0));
    track.record(&ran("/w/alpha", "cargo build", 0));
    age_run(&track, "/w/alpha", "git commit -m wip", RUN_MAX_AGE + 60);
    age_run(&track, "/w/alpha", "cargo build", RUN_MAX_AGE + 60);

    assert_eq!(track.sweep(), 1);
    assert_eq!(
        lines_in(&track, "/w/alpha"),
        vec!["cargo build".to_string()],
        "the one-off went; the habit stayed however old it is"
    );
    assert_eq!(
        rows(&track, Tree::RunByArgv),
        1,
        "and the secondary index lost the row with it, rather than pointing at nothing"
    );
}

/// A line run once but run *recently* is the line you are about to run again.
#[test]
fn a_recent_one_off_is_left_alone() {
    let (_dir, track) = store();
    track.record(&ran("/w/alpha", "kill 12345", 0));
    assert_eq!(track.sweep(), 0);
    assert_eq!(rows(&track, Tree::Run), 1);
}

/// An unmounted disk is not a deleted directory, so a directory that has gone is noted and kept.
/// It is only forgotten once it has been gone for a month, and then its lines go with it —
/// which is contract item 4, and there are no foreign keys to do it.
#[test]
fn a_directory_that_vanished_is_noted_first_and_forgotten_later() {
    let (dir, track) = store();
    let here = dir.path().to_string_lossy().into_owned();
    track.record(&Step {
        ran_in: Visit::at(&here),
        moved_to: None,
        dwell_ms: 0,
        run: Some(Run {
            argv: "cargo test",
            mode: SH,
            status: Some(0),
            duration_ms: 1,
        }),
    });
    track.record(&Step {
        ran_in: Visit {
            path: "/w/gone-4e91",
            root: Some("/w"),
        },
        moved_to: None,
        dwell_ms: 0,
        run: Some(Run {
            argv: "cargo build",
            mode: SH,
            status: Some(0),
            duration_ms: 1,
        }),
    });

    track.sweep();
    assert_eq!(
        rows(&track, Tree::Dir),
        2,
        "noted as missing, not dropped — the disk may come back"
    );
    assert!(
        dir_row(&track, "/w/gone-4e91")
            .expect("still there")
            .missing_since
            .is_some(),
        "the one that is actually gone is marked"
    );
    assert_eq!(
        dir_row(&track, &here).expect("still there").missing_since,
        None,
        "and only the one that is actually gone is marked"
    );

    // A month later, with the directory still absent.
    age_mark(&track, "/w/gone-4e91", GONE_MAX_AGE + 60);
    track.sweep();

    assert_eq!(rows(&track, Tree::Dir), 1);
    assert_eq!(
        rows(&track, Tree::Run),
        1,
        "the cascade took the gone directory's lines with it"
    );
    assert_eq!(lines_in(&track, &here), vec!["cargo test".to_string()]);
    // Every index that named it is gone too, or a later scan would find a key pointing at a
    // directory that is not there.
    for (index, left) in [
        (Tree::DirByPath, 1),
        (Tree::DirByBase, 1),
        (Tree::DirByRoot, 0),
        (Tree::RunByArgv, 1),
    ] {
        assert_eq!(rows(&track, index), left, "{index:?} still names the dead");
    }
}

/// One directory cannot grow without bound, whatever the calendar says. The rows that survive
/// are the ones that were run most, not the ones that happen to have been inserted last.
#[test]
fn a_single_directory_cannot_grow_past_the_cap() {
    let (_dir, track) = store();
    for i in 0..RUNS_PER_DIR + 20 {
        let argv = format!("echo line-{i:04}");
        track.record(&Step {
            ran_in: Visit::at("/w/busy"),
            moved_to: None,
            dwell_ms: 0,
            run: Some(Run {
                argv: &argv,
                mode: SH,
                status: Some(0),
                duration_ms: 1,
            }),
        });
    }
    // One line the user actually leans on, and it is the newest, so an eviction that went by
    // age alone would take it.
    for _ in 0..5 {
        track.record(&ran("/w/busy", "make verify", 0));
    }

    track.sweep();
    assert_eq!(rows(&track, Tree::Run), RUNS_PER_DIR);
    assert_eq!(
        rows(&track, Tree::RunByArgv),
        RUNS_PER_DIR,
        "the index is capped with the rows, not left holding the evicted"
    );
    assert!(
        run_row(&track, "/w/busy", SH, "make verify").is_some(),
        "the habit survived; the once-run filler around it did not"
    );
}

/// The cap is per directory, not per store — one noisy directory must not evict a quiet
/// neighbour's lines.
#[test]
fn one_directory_over_the_cap_costs_its_neighbours_nothing() {
    let (_dir, track) = store();
    for i in 0..RUNS_PER_DIR + 10 {
        let argv = format!("echo line-{i:04}");
        track.record(&Step {
            ran_in: Visit::at("/w/busy"),
            moved_to: None,
            dwell_ms: 0,
            run: Some(Run {
                argv: &argv,
                mode: SH,
                status: Some(0),
                duration_ms: 1,
            }),
        });
    }
    track.record(&ran("/w/quiet", "cargo build", 0));

    track.sweep();
    assert_eq!(
        lines_in(&track, "/w/quiet"),
        vec!["cargo build".to_string()]
    );
    assert_eq!(lines_in(&track, "/w/busy").len(), RUNS_PER_DIR);
}

/// The sweep is daily, and a store that has never been swept is due at once rather than a day
/// after a version that stamps it first ran.
#[test]
fn the_sweep_is_due_once_a_day_and_immediately_on_a_store_that_has_never_had_one() {
    let (_dir, track) = store();
    assert!(track.sweep_is_due());
    track.sweep();
    assert!(!track.sweep_is_due(), "and not again for a day");

    track
        .store
        .write(|writer| set_meta(writer, LAST_PRUNE, now() - SWEEP_EVERY - 60))
        .expect("the stamp is backdated");
    assert!(track.sweep_is_due());
}

/// The bound the file now depends on. There is no write-ahead log to truncate any more and no
/// `VACUUM` to run, so the only thing standing between oslo and a permanent 8.5 MiB is that
/// the row count is held down in the first place.
#[test]
fn the_sweep_is_the_only_thing_holding_the_file_down() {
    let (dir, track) = store();
    for i in 0..RUNS_PER_DIR + 200 {
        let argv = format!("echo line-{i:04}");
        track.record(&Step {
            ran_in: Visit::at("/w/busy"),
            moved_to: None,
            dwell_ms: 0,
            run: Some(Run {
                argv: &argv,
                mode: SH,
                status: Some(0),
                duration_ms: 1,
            }),
        });
    }

    assert!(track.sweep() >= 200);
    assert_eq!(rows(&track, Tree::Run), RUNS_PER_DIR);
    let mut beside: Vec<String> = std::fs::read_dir(dir.path().join("nested"))
        .expect("the directory is there")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    beside.sort();
    // The lock replaces the extension rather than being appended to it, so a profile's directory
    // holds `hist.db` and `hist.lock` rather than `hist.db.lock`.
    assert_eq!(
        beside,
        vec!["track.kv".to_string(), "track.lock".to_string()]
    );
}

/// A file written by a version this binary does not understand is read, never rewritten — and
/// deleting rows out of it would be the most destructive way to break that promise.
#[test]
fn a_store_from_a_newer_version_is_never_swept() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("track.kv");
    {
        let track = Track::open(&path).expect("the store opens");
        track.record(&ran("/w/alpha", "cargo build", 0));
        age_run(&track, "/w/alpha", "cargo build", RUN_MAX_AGE + 60);
        track.claim_future_version();
    }

    let newer = Track::open(&path).expect("it still opens");
    assert!(!newer.sweep_is_due());
    assert_eq!(newer.sweep(), 0);
    assert_eq!(rows(&newer, Tree::Run), 1);
}

/// A cascade large enough to span multiple deletion chunks.
///
/// Verified by reverting `forget_directory` to one transaction, at which point this fails with the
/// directory still present and the sweep having reported it gone.
#[test]
fn a_directory_is_really_forgotten_out_of_a_deep_bucket_and_not_merely_reported_as_forgotten() {
    let (_dir, track) = store();
    let each = RUNS_PER_DIR - 50;
    let doomed = "/w/gone-huge";
    let mut kept = 0;
    for place in [
        "/w/n01", "/w/n02", "/w/n03", "/w/n04", "/w/n05", "/w/n06", "/w/n07", "/w/n08", "/w/n09",
        "/w/n10", "/w/n11", doomed,
    ] {
        for i in 0..each {
            let argv = format!("cargo run --example {i:06} --release --features a,b,c");
            track.record(&Step {
                ran_in: Visit {
                    path: place,
                    root: Some("/w"),
                },
                moved_to: None,
                dwell_ms: 0,
                run: Some(Run {
                    argv: &argv,
                    mode: SH,
                    status: Some(0),
                    duration_ms: 1,
                }),
            });
        }
        if place != doomed {
            kept += each;
        }
    }
    assert_eq!(rows(&track, Tree::Run), kept + each);

    // None of them has ever existed on disk, so the first sweep marks them all; only the doomed
    // one's mark is backdated, so only it is forgotten.
    track.sweep();
    age_mark(&track, doomed, GONE_MAX_AGE + 60);
    track.sweep();

    assert_eq!(
        dir_row(&track, doomed),
        None,
        "gone from the file, rather than from a return value"
    );
    assert_eq!(
        rows(&track, Tree::Run),
        kept,
        "every line it held went with it, and not one of its neighbours' did"
    );
    assert_eq!(
        rows(&track, Tree::RunByArgv),
        kept,
        "including the index entries, which are the half no span can reach"
    );
    assert_eq!(lines_in(&track, doomed).len(), 0);
    assert_eq!(lines_in(&track, "/w/n06").len(), each);
    for index in [Tree::DirByPath, Tree::DirByBase, Tree::DirByRoot] {
        assert_eq!(rows(&track, index), 11, "{index:?} still names the dead");
    }
}
