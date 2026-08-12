//! Only the parsing and rendering, which need no directory. Reading and writing the real file is
//! covered end to end by `tests/plugin_tests.rs`, where a whole home can be temporary.

use super::*;

fn one() -> Installed {
    Installed {
        name: "notes".to_string(),
        entry: "init.lua".to_string(),
        builtins: vec!["note".to_string()],
        tools: vec!["notes".to_string()],
        hash: "abc123".to_string(),
        requires: None,
        load_on: None,
    }
}

#[test]
fn an_entry_survives_being_written_and_read() {
    let document = json!({
        "version": VERSION,
        "plugins": [ {
            "name": "notes", "entry": "init.lua",
            "builtins": ["note"], "tools": ["notes"], "hash": "abc123",
        } ],
    });
    let read = parse(&document.to_string()).expect("parse");
    assert_eq!(read, vec![one()]);
    assert_eq!(
        read[0].names().collect::<Vec<_>>(),
        vec!["note", "notes"],
        "both kinds are reserved together"
    );
}

/// A version this oslo does not write is refused rather than guessed at.
#[test]
fn a_future_index_is_refused() {
    let document = json!({ "version": VERSION + 1, "plugins": [] });
    let refused = parse(&document.to_string()).expect_err("should refuse");
    assert!(refused.contains("not one this oslo writes"), "{refused}");

    let refused = parse(&json!({ "plugins": [] }).to_string()).expect_err("should refuse");
    assert!(refused.contains("no version"), "{refused}");
}

#[test]
fn an_entry_missing_its_name_or_hash_is_refused() {
    for document in [
        json!({ "version": VERSION, "plugins": [ { "hash": "x" } ] }),
        json!({ "version": VERSION, "plugins": [ { "name": "notes" } ] }),
    ] {
        assert!(parse(&document.to_string()).is_err());
    }
}

#[test]
fn an_empty_index_is_no_plugins_rather_than_an_error() {
    let found = parse(&json!({ "version": VERSION, "plugins": [] }).to_string()).expect("parse");
    assert!(found.is_empty());
    // And a document with no `plugins` at all reads the same way.
    let found = parse(&json!({ "version": VERSION }).to_string()).expect("parse");
    assert!(found.is_empty());
}

#[test]
fn text_that_is_not_json_is_a_message_rather_than_a_panic() {
    assert!(parse("{").is_err());
    assert!(parse("").is_err());
}
