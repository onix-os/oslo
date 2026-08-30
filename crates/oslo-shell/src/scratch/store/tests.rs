use super::*;
use crate::scratch::{dir, scratch};

/// **The heart of it**: holding the lock is what "alive" means, and letting go is what "gone" means.
#[test]
fn a_held_lock_is_what_alive_means() {
    let (_dir, _lock) = scratch();
    dir::open_checked().expect("scratch directory");
    assert!(!alive("alpha"), "nothing has been created yet");

    let held = hold("alpha").expect("lock").expect("nobody else has it");
    assert!(alive("alpha"), "while it is held");

    // A second keeper for the same name is refused, which is how a double-start is prevented.
    assert!(hold("alpha").expect("lock").is_none());

    drop(held);
    assert!(!alive("alpha"), "the moment it is dropped");
}

/// A keeper killed with -9 cannot tidy up, so whoever looks next does it.
#[test]
fn a_dead_tab_is_swept_by_whoever_lists() {
    let (_scratch, _lock) = scratch();
    dir::open_checked().expect("scratch directory");
    // Leftovers with no keeper behind them: exactly what a `kill -9` leaves.
    for extension in ["lock", "sock", "meta", "log"] {
        std::fs::write(dir::path().join(format!("beta.{extension}")), b"").expect("write");
    }
    assert!(!alive("beta"));

    let scratches = list().expect("list");
    assert!(
        scratches.is_empty(),
        "a dead scratch is not listed: {scratches:?}"
    );
    assert!(
        !dir::path().join("beta.sock").exists(),
        "and its leftovers are gone"
    );
}

/// A live scratch is listed, with what it said about itself.
#[test]
fn a_live_tab_is_listed_with_its_meta() {
    let (_dir, _lock) = scratch();
    dir::open_checked().expect("scratch directory");
    let held = hold("gamma").expect("lock").expect("free");
    std::fs::write(
        Paths::new("gamma").meta(),
        Meta {
            cwd: "/tmp/somewhere".into(),
            started: 1000,
            pid: 42,
            keeper: 41,
        }
        .encode(),
    )
    .expect("write");

    let scratches = list().expect("list");
    assert_eq!(scratches.len(), 1);
    assert_eq!(scratches[0].0, "gamma");
    assert_eq!(scratches[0].1.cwd, "/tmp/somewhere");
    assert_eq!(scratches[0].1.pid, 42);
    drop(held);
}

/// Newest first, so the picker opens on what you were just doing.
#[test]
fn the_newest_tab_is_first() {
    let (_dir, _lock) = scratch();
    dir::open_checked().expect("scratch directory");
    let mut held = Vec::new();
    for (name, started) in [("alpha", 100u64), ("beta", 300), ("gamma", 200)] {
        held.push(hold(name).expect("lock").expect("free"));
        let meta = Meta {
            cwd: "/".into(),
            started,
            pid: 1,
            keeper: 1,
        };
        std::fs::write(Paths::new(name).meta(), meta.encode()).expect("write");
    }
    let names: Vec<String> = list().expect("list").into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["beta", "gamma", "alpha"]);
}

/// A torn `.meta` costs the decoration, never the scratch.
#[test]
fn an_unreadable_meta_still_lists_the_tab() {
    let (_dir, _lock) = scratch();
    dir::open_checked().expect("scratch directory");
    let held = hold("delta").expect("lock").expect("free");
    std::fs::write(
        Paths::new("delta").meta(),
        b"this is not key=value\n\x00\xff",
    )
    .expect("write");

    let scratches = list().expect("list");
    assert_eq!(scratches.len(), 1, "still listed");
    assert_eq!(scratches[0].1.pid, 0, "with the fields at their defaults");
    drop(held);
}

#[test]
fn meta_round_trips() {
    let meta = Meta {
        cwd: "/home/x/y".into(),
        started: 1_700_000_000,
        pid: 4321,
        keeper: 4320,
    };
    assert_eq!(Meta::decode(&meta.encode()), meta);
}

/// A file in the directory that is not a scratch is not read as one.
#[test]
fn only_lock_files_name_a_tab() {
    let (_scratch, _lock) = scratch();
    dir::open_checked().expect("scratch directory");
    std::fs::write(dir::path().join("notes.txt"), b"").expect("write");
    std::fs::write(dir::path().join("..lock"), b"").expect("write");
    assert!(list().expect("list").is_empty());
}

/// A name nothing is holding is already ended, so ending it is a tidy-up rather than a failure —
/// which is what makes the finder's delete key safe to press on a row that died a moment ago.
#[test]
fn killing_what_is_not_running_tidies_rather_than_fails() {
    let (_dir, _lock) = scratch();
    dir::open_checked().expect("scratch directory");
    std::fs::write(Paths::new("ghost").meta(), Meta::default().encode()).expect("write");

    kill("ghost").expect("a dead scratch is not an error");
    assert!(
        !Paths::new("ghost").meta().exists(),
        "the leftovers went too"
    );
}
