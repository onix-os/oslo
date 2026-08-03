//! What the seam promises, tested against a real file.
//!
//! Every test here opens a database in a temporary directory, because the properties being claimed
//! — the mode of the file, the lock it does not hold, the transaction it does not commit — are all
//! properties of the file rather than of the code that writes it.

use super::*;
use std::os::unix::fs::PermissionsExt;

/// A store in a temporary directory, one level down, so that the directory-creating path is the
/// one every test exercises.
fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Store::open(&dir.path().join("nested/track.kv")).expect("the store opens");
    (dir, store)
}

/// `(dir_id, mode, argv)`, the key the contract is written in.
fn run_key(dir: u64, mode: &str, argv: &str) -> Vec<u8> {
    Key::new().int(dir).text(mode).text(argv).done()
}

fn mode_of(path: &Path) -> u32 {
    std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("{} should exist: {error}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

/// The privacy rule, and the ordering jammdb forces on it. Under turso the file had to be created
/// before the engine saw the path; here that panics, so the directory is closed first and the file
/// is tightened before `open` hands anybody a `Store` to write through.
#[test]
fn the_store_is_private_before_any_caller_can_write_to_it() {
    let (dir, store) = store();
    assert_eq!(mode_of(store.path()), 0o600);
    assert_eq!(mode_of(&dir.path().join("nested")), 0o700);
}

/// turso needed a `-wal` and an `-shm` kept private beside the database, and getting that wrong
/// leaked the newest rows. jammdb writes one file. If that ever stops being true, the mode of
/// whatever appears is nobody's job, and this is where it is noticed.
#[test]
fn the_store_is_one_file_with_nothing_beside_it() {
    let (dir, store) = store();
    store
        .write(|w| w.put(Tree::Run, run_key(1, "sh", "cargo build"), b"1".to_vec()))
        .expect("the write commits");

    let beside: Vec<String> = std::fs::read_dir(dir.path().join("nested"))
        .expect("the directory is there")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        beside,
        vec!["track.kv".to_string()],
        "no sidecar, no lock file"
    );
    assert!(store.size() > 0);
}

/// The property the whole design rests on, asserted directly rather than inferred: an open store
/// holds no lock on its file, so another terminal can take one. If someone ever makes `Store` hold
/// a `DB`, this fails here instead of hanging somebody's second shell.
#[test]
fn an_open_store_holds_no_lock_for_another_terminal_to_wait_on() {
    use nix::fcntl::{Flock, FlockArg};

    let (_dir, store) = store();
    store
        .write(|w| w.put(Tree::Run, run_key(1, "sh", "cargo build"), b"1".to_vec()))
        .expect("the write commits");

    let file = std::fs::File::open(store.path()).expect("the file opens");
    let taken = Flock::lock(file, FlockArg::LockExclusiveNonblock);
    assert!(
        taken.is_ok(),
        "nothing is holding the file, so another terminal's lock is free to take"
    );
}

/// Two stores against one file, interleaved, which is the shape of two terminals. Every operation
/// opens and closes, so neither ever waits on the other for longer than one transaction.
#[test]
fn two_stores_on_one_file_both_write_and_both_see_the_other() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("track.kv");
    let first = Store::open(&path).expect("the first opens");
    let second = Store::open(&path).expect("the second opens too");

    for round in 0..10u64 {
        first
            .write(|w| {
                w.put(
                    Tree::Run,
                    run_key(1, "sh", &format!("a-{round}")),
                    b"1".to_vec(),
                )
            })
            .expect("the first writes");
        second
            .write(|w| {
                w.put(
                    Tree::Run,
                    run_key(1, "sh", &format!("b-{round}")),
                    b"1".to_vec(),
                )
            })
            .expect("the second writes");
    }

    let rows = |store: &Store| {
        store
            .read(|r| Some(r.count(Tree::Run, &Span::all())))
            .expect("the read succeeds")
    };
    assert_eq!(rows(&first), 20, "nothing was lost");
    assert_eq!(rows(&second), 20, "and each sees the other's work");
}

/// The write helper exists so that no caller has to remember to commit. `Some` commits.
#[test]
fn a_write_that_answers_something_is_still_there_afterwards() {
    let (_dir, store) = store();
    let key = run_key(1, "sh", "cargo build");
    let answered = store.write(|w| {
        w.put(Tree::Run, key.clone(), b"counters".to_vec())?;
        Some(42)
    });

    assert_eq!(answered, Some(42), "and the closure's answer comes back");
    assert_eq!(
        store.read(|r| r.get(Tree::Run, &key)),
        Some(b"counters".to_vec())
    );
}

/// And `None` discards, without the caller writing a rollback. A half-finished write must not be
/// able to leave a run attributed to a directory whose arrival was never recorded.
#[test]
fn a_write_that_answers_nothing_leaves_the_store_as_it_found_it() {
    let (_dir, store) = store();
    let kept = run_key(1, "sh", "cargo build");
    store
        .write(|w| w.put(Tree::Run, kept.clone(), b"kept".to_vec()))
        .expect("the first write commits");

    let abandoned = run_key(1, "sh", "cargo test");
    let answered: Option<()> = store.write(|w| {
        w.put(Tree::Run, abandoned.clone(), b"never".to_vec())?;
        w.delete(Tree::Run, &kept);
        None
    });

    assert_eq!(answered, None);
    store.read(|r| {
        assert!(!r.has(Tree::Run, &abandoned), "the put was discarded");
        assert!(r.has(Tree::Run, &kept), "and so was the delete");
        Some(())
    });
}

/// The half-open range, end to end through the composite key: a seek to the lower bound and a walk
/// to the upper one, which must find everything that starts with what was typed and nothing that
/// merely contains it — including at the boundaries where the range begins and ends.
#[test]
fn a_prefix_scan_finds_only_what_starts_with_what_was_typed() {
    let (_dir, store) = store();
    store
        .write(|w| {
            for (dir, mode, argv) in [
                (7, "sh", "cargo t"),
                (7, "sh", "cargo te"),
                (7, "sh", "cargo test"),
                (7, "sh", "cargo tesseract"),
                (7, "sh", "cargo tests --all"),
                (7, "sh", "cargo tf"),
                (7, "sh", "make cargo test"),
                (7, "lua", "cargo test"),
                (8, "sh", "cargo test"),
                (6, "sh", "cargo test"),
            ] {
                w.put(Tree::Run, run_key(dir, mode, argv), b"1".to_vec())?;
            }
            Some(())
        })
        .expect("the rows are written");

    let span = Span::prefix(Key::new().int(7).text("sh").text_prefix("cargo te").done());
    let found: Vec<String> = store
        .read(|r| {
            Some(r.collect(Tree::Run, &span, |key, _| {
                let mut fields = Fields::of(key);
                fields.int()?;
                fields.text()?;
                Some(fields.text()?.into_owned())
            }))
        })
        .expect("the read succeeds");

    assert_eq!(
        found,
        vec![
            "cargo te".to_string(),
            "cargo tesseract".to_string(),
            "cargo test".to_string(),
            "cargo tests --all".to_string(),
        ],
        "in key order, which is the order the range was walked in"
    );
}

/// `LIMIT 1`. The suggestion query is by far the commonest read in the store and it wants one row,
/// so the scan has to be able to stop — a range that collected and then took the first would pay
/// for the whole range to answer with one of it.
#[test]
fn a_scan_stops_where_it_is_told_rather_than_reading_the_range() {
    let (_dir, store) = store();
    store
        .write(|w| {
            for i in 0..500u64 {
                w.put(
                    Tree::Run,
                    run_key(1, "sh", &format!("echo {i:04}")),
                    b"1".to_vec(),
                )?;
            }
            Some(())
        })
        .expect("the rows are written");

    let span = Span::prefix(Key::new().int(1).text("sh").text_prefix("echo ").done());
    let mut seen = 0;
    let first = store
        .read(|r| {
            r.find(Tree::Run, &span, |key, _| {
                seen += 1;
                let mut fields = Fields::of(key);
                fields.int()?;
                fields.text()?;
                Some(fields.text()?.into_owned())
            })
        })
        .expect("something matched");

    assert_eq!(first, "echo 0000");
    assert_eq!(seen, 1, "one row was read, not five hundred");
}

/// A store's first read happens before its first write, and must answer "nothing" rather than
/// "failure": a bucket that no write has created yet holds exactly the rows an empty one does.
#[test]
fn a_store_with_nothing_in_it_answers_nothing_rather_than_failing() {
    let (_dir, store) = store();
    store
        .read(|r| {
            assert_eq!(r.get(Tree::Run, &run_key(1, "sh", "anything")), None);
            assert!(!r.has(Tree::Dir, b"nothing"));
            assert_eq!(r.count(Tree::Run, &Span::all()), 0);
            assert!(
                r.collect(Tree::History, &Span::all(), |_, _| Some(()))
                    .is_empty()
            );
            Some(())
        })
        .expect("a read of an empty store is still a read");
}

/// The cascade. jammdb has no foreign keys, so dropping a directory's runs is this code's job, and
/// it is a range delete over the `dir_id` the composite key begins with. It must take everything
/// belonging to that directory and nothing belonging to its neighbours — including the one whose
/// `dir_id` is the very next number.
#[test]
fn deleting_a_directorys_span_takes_its_runs_and_only_its_runs() {
    let (_dir, store) = store();
    store
        .write(|w| {
            for dir in [6u64, 7, 8] {
                for argv in ["cargo build", "cargo test", "make verify"] {
                    w.put(Tree::Run, run_key(dir, "sh", argv), b"1".to_vec())?;
                }
            }
            Some(())
        })
        .expect("the rows are written");

    let gone = store
        .write(|w| {
            let span = Span::prefix(Key::new().int(7).done());
            Some(w.delete_span(Tree::Run, &span))
        })
        .expect("the delete commits");

    assert_eq!(gone, 3);
    assert_eq!(
        store.read(|r| Some(r.count(Tree::Run, &Span::all()))),
        Some(6),
        "six rows left, three each side"
    );
    assert_eq!(
        store.read(|r| Some(r.count(Tree::Run, &Span::prefix(Key::new().int(7).done())))),
        Some(0)
    );
    assert!(store.read(|r| Some(r.has(Tree::Run, &run_key(6, "sh", "cargo test")))) == Some(true));
    assert!(store.read(|r| Some(r.has(Tree::Run, &run_key(8, "sh", "cargo test")))) == Some(true));
}

/// What `history -c` and `forget_runs` need: one bucket emptied, the others untouched.
#[test]
fn clearing_one_bucket_leaves_the_others_standing() {
    let (_dir, store) = store();
    store
        .write(|w| {
            w.put(Tree::Run, run_key(1, "sh", "cargo build"), b"1".to_vec())?;
            w.put(Tree::Dir, Key::new().int(1).done(), b"/w/alpha".to_vec())?;
            Some(())
        })
        .expect("the rows are written");

    assert_eq!(store.write(|w| Some(w.clear(Tree::Run))), Some(true));
    assert_eq!(
        store.read(|r| Some(r.count(Tree::Run, &Span::all()))),
        Some(0)
    );
    assert_eq!(
        store.read(|r| Some(r.count(Tree::Dir, &Span::all()))),
        Some(1),
        "where you work is not one of the lines you asked to forget"
    );
}

/// A file that is not one of ours is refused rather than opened. Measured on jammdb 0.11.0:
/// `DB::open` *panics* on one, so this is the difference between an upgrade that quietly stops
/// tracking and an upgrade that will not start a shell — and every existing oslo has a SQLite file
/// at exactly this path.
#[test]
fn a_file_that_is_not_one_of_ours_is_refused_and_left_alone() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("track.db");
    let mut sqlite = b"SQLite format 3\0".to_vec();
    sqlite.resize(16 * 4096, 0);
    std::fs::write(&path, &sqlite).expect("written");

    assert!(
        Store::open(&path).is_none(),
        "refused, and without panicking"
    );
    assert_eq!(
        std::fs::read(&path).expect("still readable"),
        sqlite,
        "and somebody else's file is not touched, let alone rewritten"
    );
}

/// A bucket name is one string in one place. If a variant ever shares a name with another the two
/// silently become one bucket, and the symptom is a store that answers with somebody else's rows.
#[test]
fn every_bucket_is_named_once() {
    let mut names: Vec<&str> = Tree::all().iter().map(|tree| tree.name()).collect();
    let all = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), all, "two buckets share a name: {names:?}");
}

/// The growth note, pinned to a number. jammdb allocates in 8 MiB steps and never gives one back,
/// so a store bounded at a few hundred rows costs 128 KiB and one that is not costs 8.5 MiB for
/// ever. That is what makes the per-directory cap load-bearing rather than tidy, and this is the
/// measurement that says so.
#[test]
fn a_small_store_stays_small_and_a_large_one_takes_a_whole_step() {
    let (_dir, store) = store();
    let write = |from: u64, to: u64| {
        store
            .write(|w| {
                for i in from..to {
                    let argv = format!("cargo run --example {i:06} --release");
                    w.put(
                        Tree::Run,
                        run_key(i % 200, "sh", &argv),
                        format!("{i}").into_bytes(),
                    )?;
                }
                Some(())
            })
            .expect("the rows are written");
    };

    write(0, 400);
    assert!(
        store.size() <= 128 * 1024,
        "four hundred rows fit in the file jammdb starts with, and oslo's real store is smaller \
         than that: {} bytes",
        store.size()
    );

    write(400, 4_000);
    assert!(
        store.size() >= 8 * 1024 * 1024,
        "and the step past it is 8 MiB, not a page: {} bytes",
        store.size()
    );
}

/// The name libtest knows the crash test by, which is how the parent re-invokes the binary to get
/// a *real* second process to kill. A thread would not do: `kill -9` is the thing being tested, and
/// what makes it survivable is that the kernel unmaps the pages and drops the `flock` on a process
/// that is not there to clean up after itself.
const CRASH_TEST: &str = "track::kv::tests::the_store_survives_a_shell_killed_mid_write";

/// The environment variable that turns that test into the child half of itself.
const CRASH_CHILD: &str = "OSLO_STORE_CRASH_CHILD";

/// Contract item 7. A shell is killed all the time — a closed terminal, an OOM, a `kill -9` on a
/// hung pipeline — and it must cost at most the transaction that was in flight. jammdb writes no
/// page until `commit` and alternates two meta pages, so the worst case is the last write; the file
/// itself must still open, still hold everything committed before the kill, and still take writes.
#[test]
fn the_store_survives_a_shell_killed_mid_write() {
    // The child half: write until somebody kills us.
    if let Ok(path) = std::env::var(CRASH_CHILD) {
        let store = Store::open(Path::new(&path)).expect("the child opens the same store");
        for round in 0.. {
            store.write(|w| {
                for i in 0..200u64 {
                    w.put(
                        Tree::Run,
                        run_key(9, "sh", &format!("child {round} {i}")),
                        b"1".to_vec(),
                    )?;
                }
                Some(())
            });
        }
        return;
    }

    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("track.kv");
    let store = Store::open(&path).expect("the store opens");
    store
        .write(|w| {
            for i in 0..200u64 {
                w.put(
                    Tree::Run,
                    run_key(1, "sh", &format!("committed {i:04}")),
                    b"1".to_vec(),
                )?;
            }
            Some(())
        })
        .expect("the rows that must survive are committed first");

    let exe = std::env::current_exe().expect("the test binary");
    let mut child = std::process::Command::new(exe)
        .args(["--exact", CRASH_TEST, "--nocapture"])
        .env(CRASH_CHILD, &path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a second process");
    // Long enough that it is certainly inside a transaction rather than still starting up.
    std::thread::sleep(std::time::Duration::from_millis(300));
    child
        .kill()
        .expect("killed with SIGKILL, which no handler can catch");
    let status = child.wait().expect("reaped");
    assert!(!status.success(), "the child died rather than finishing");

    let after = Store::open(&path).expect("the file is still a database this shell can open");
    assert_eq!(
        after.read(|r| Some(r.count(
            Tree::Run,
            &Span::prefix(Key::new().int(1).text("sh").text_prefix("committed").done())
        ))),
        Some(200),
        "everything committed before the kill is still there"
    );
    assert_eq!(
        after.write(|w| w.put(
            Tree::Run,
            run_key(1, "sh", "after the crash"),
            b"1".to_vec()
        )),
        Some(()),
        "and the store takes writes again — the lock died with the process that held it"
    );
}

/// A span too large for one transaction, deleted anyway.
///
/// `Writer::delete_span` panics inside jammdb — and therefore deletes nothing — when the rows it
/// removes empty a leaf node of a bucket deep enough to have a branch of branches. Measured: 3,500
/// rows, delete the last 100 in one transaction, nothing goes. `Store::delete_span_in_chunks` keeps
/// each transaction under a quarter page and gets all of them. The rows go in one transaction each,
/// because that is how a shell writes them and the tree shape is what the defect turns on.
#[test]
fn a_span_too_large_for_one_transaction_is_deleted_in_pieces() {
    let (_dir, store) = store();
    for id in 0..3_500u64 {
        store
            .write(|w| {
                w.put(
                    Tree::History,
                    Key::with_capacity(8).int(id).done(),
                    vec![b'x'; 40],
                )
            })
            .expect("the row goes in");
    }
    let doomed = Span::from(Key::with_capacity(8).int(3_400).done());

    assert_eq!(store.delete_span_in_chunks(Tree::History, &doomed), 100);
    let left = store
        .read(|r| Some(r.count(Tree::History, &Span::all())))
        .expect("the rows are still readable");
    assert_eq!(left, 3_400, "and only the doomed hundred went");
    assert_eq!(
        store.delete_span_in_chunks(Tree::History, &doomed),
        0,
        "a span with nothing in it is not an error and not a loop"
    );
}

/// The chunk is measured in bytes rather than rows, so one enormous value is one transaction and
/// does not take a quarter page of small ones with it.
#[test]
fn a_chunk_is_a_quarter_page_of_rows_however_big_the_rows_are() {
    let (_dir, store) = store();
    store
        .write(|w| {
            for id in 0..8u64 {
                w.put(
                    Tree::History,
                    Key::with_capacity(8).int(id).done(),
                    vec![b'x'; 4_000],
                )?;
            }
            Some(())
        })
        .expect("the rows go in");

    assert_eq!(store.delete_span_in_chunks(Tree::History, &Span::all()), 8);
    assert_eq!(
        store.read(|r| Some(r.count(Tree::History, &Span::all()))),
        Some(0)
    );
}
