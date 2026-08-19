//! The table's shape, and the shape of a row.
//!
//! What the store *holds* is `oslo_base::track::history`'s subject and is tested there. A unit test
//! here runs with no store installed, which is itself the case worth pinning: a shell whose store
//! would not open is a working shell, and a config looping over history in one must not break.

use super::super::util::probe;
use super::*;

#[test]
fn the_table_offers_reading_and_forgetting() {
    let built = Value::table(build());
    for name in ["commands", "forget"] {
        assert!(
            matches!(probe::field(&built, name), Value::Function(_)),
            "no {name}"
        );
    }
}

/// **With no store, an empty list rather than nil or an error.** A config writing
/// `for _, c in ipairs(oslo.history.commands()) do` should run everywhere, including in a shell
/// whose store would not open — and `ipairs(nil)` is the error that would make it not.
#[test]
fn with_no_store_the_answers_are_empty_rather_than_absent() {
    // No `track::install` has run on this thread, so `store()` is `None`.
    let built = Value::table(build());

    let listed = probe::first(&probe::field(&built, "commands"), Vec::new());
    let Value::Table(listed) = listed else {
        panic!("commands did not answer a table")
    };
    assert!(listed.borrow().sequence().is_empty());

    let gone = probe::first(
        &probe::field(&built, "forget"),
        vec![Value::str("anything")],
    );
    assert!(matches!(gone, Value::Number(_)), "forget answered a number");
}

/// `forget` needs a line, and says so rather than forgetting everything.
#[test]
fn forgetting_nothing_is_a_mistake_worth_raising() {
    let built = Value::table(build());
    let refused = probe::call(&probe::field(&built, "forget"), Vec::new())
        .expect_err("a missing line should be refused");
    assert!(refused.to_string().contains("argument #1"), "{refused}");
}

/// Every field the tracker kept reaches Lua, named as it is named in Rust.
#[test]
fn a_row_carries_every_field() {
    let command = oslo_base::track::history::Command {
        line: "cargo build".to_string(),
        mode: "sh".to_string(),
        runs: 12,
        last_at: 1_700_000_000,
        dir: "/home/u/proj".to_string(),
        places: 3,
        worked: true,
        session: "abc".to_string(),
        host: "box".to_string(),
        root: Some("/home/u/proj".to_string()),
    };
    let Value::Table(row) = row(command) else {
        panic!("not a table")
    };
    let row = row.borrow();
    for name in [
        "line", "mode", "runs", "last_at", "dir", "places", "worked", "session", "host", "root",
    ] {
        assert!(
            !matches!(row.get_str(name), Value::Nil),
            "{name} did not reach the row"
        );
    }
    // `last_at` is unix seconds, so `os.date` renders it and comparison stays numeric.
    assert!(matches!(row.get_str("last_at"), Value::Number(_)));
    assert!(matches!(row.get_str("worked"), Value::Bool(true)));
}

/// Outside a repository there is no worktree, and `root` is nil rather than an empty string —
/// "not in a repository" and "in one rooted at nowhere" are different answers.
#[test]
fn a_row_outside_a_repository_has_no_root() {
    let command = oslo_base::track::history::Command {
        line: "ls".to_string(),
        mode: "sh".to_string(),
        runs: 1,
        last_at: 0,
        dir: "/tmp".to_string(),
        places: 1,
        worked: true,
        session: String::new(),
        host: String::new(),
        root: None,
    };
    let Value::Table(row) = row(command) else {
        panic!("not a table")
    };
    assert!(matches!(row.borrow().get_str("root"), Value::Nil));
}
