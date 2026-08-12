//! The handle's shape. What it *stores* is `oslo_base::store`'s subject and is tested there; what
//! matters here is that the table offers the verbs and that `open` refuses a name that is not one.

use super::*;

#[test]
fn the_table_offers_open_and_path() {
    let Value::Table(built) = build() else {
        panic!("not a table")
    };
    let built = built.borrow();
    for name in ["open", "path"] {
        assert!(
            matches!(built.get(&Value::str(name)), Value::Function(_)),
            "no {name}"
        );
    }
}

#[test]
fn a_handle_offers_every_verb_and_knows_its_own_name() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Rc::new(Store::open(&dir.path().join("t.kv")).expect("open"));
    let Value::Table(handle) = handle("notes", store) else {
        panic!("not a table")
    };
    let handle = handle.borrow();
    for name in [
        "get", "set", "has", "delete", "keys", "write", "path", "size",
    ] {
        assert!(
            matches!(handle.get(&Value::str(name)), Value::Function(_)),
            "no {name}"
        );
    }
    match handle.get(&Value::str("name")) {
        Value::Str(name) => assert_eq!(name.as_ref(), "notes"),
        other => panic!("name is {}", other.type_name()),
    }
}

/// A name that could reach another database never gets as far as opening one.
#[test]
fn opening_a_name_that_is_not_a_name_is_a_message() {
    for bad in ["../history", "a/b", ""] {
        let refused = opened(bad).expect_err("should refuse");
        assert!(refused.contains("not a database name"), "{refused}");
    }
}
