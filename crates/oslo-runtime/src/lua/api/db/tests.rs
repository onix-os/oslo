//! The handle's shape. What it *stores* is `oslo_base::store`'s subject and is tested there; what
//! matters here is that the object offers the verbs, keeps its internals out of `pairs`, and
//! refuses everything once it is closed.

use super::super::util::probe;
use super::*;

/// A handle on a throwaway database. The directory has to outlive it, so it comes back too.
fn opened() -> (tempfile::TempDir, Value) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Rc::new(Store::open(&dir.path().join("t.kv")).expect("open"));
    (dir, handle("notes", store))
}

#[test]
fn the_table_offers_open_and_path() {
    let Value::Table(built) = build(&probe::Nowhere) else {
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
    let (_dir, handle) = opened();
    for name in ["get", "set", "has", "delete", "keys", "path", "size"] {
        assert!(
            matches!(probe::field(&handle, name), Value::Function(_)),
            "no {name}"
        );
    }
    match probe::field(&handle, "name") {
        Value::Str(name) => assert_eq!(name.as_ref(), "notes"),
        other => panic!("name is {}", other.type_name()),
    }
}

/// **The handle itself is empty**, so `pairs(db)` cannot show `__begin` — or anything else that is
/// `db:write`'s business rather than the caller's.
#[test]
fn the_internals_are_not_part_of_the_surface() {
    let (_dir, handle) = opened();
    let Value::Table(table) = &handle else {
        panic!("not a table")
    };
    assert!(
        table.borrow().pairs().is_empty(),
        "the handle has keys of its own: {:?}",
        table.borrow().pairs()
    );
    assert!(
        matches!(table.borrow().get_str("__begin"), Value::Nil),
        "__begin is reachable without the metatable"
    );
    // Still callable by name, because `db:write` needs it.
    assert!(matches!(
        probe::field(&handle, "__begin"),
        Value::Function(_)
    ));
}

/// A value written and read back is the same bytes.
#[test]
fn a_verb_reaches_the_store_behind_the_handle() {
    let (_dir, handle) = opened();
    probe::method(&handle, "set", vec![Value::str("a"), Value::str("1")]).expect("set");
    match probe::method(&handle, "get", vec![Value::str("a")]).expect("get")[0] {
        Value::Str(ref got) => assert_eq!(got.as_ref(), "1"),
        ref other => panic!("get answered {}", other.type_name()),
    }
}

/// **A dot instead of a colon is a message, not a wrong read.** `db.get("k")` passes `"k"` where
/// `self` goes, so the key argument is missing — which is what the verb reports.
#[test]
fn calling_a_verb_with_a_dot_is_refused() {
    let (_dir, handle) = opened();
    let refused = probe::call(&probe::field(&handle, "get"), vec![Value::str("a")])
        .expect_err("a dot call should not read anything");
    assert!(refused.to_string().contains("argument #2"), "{refused}");
}

/// Closing releases, and every verb says so afterwards.
#[test]
fn a_closed_handle_refuses_its_verbs() {
    let (_dir, handle) = opened();
    let Value::Table(table) = &handle else {
        panic!("not a table")
    };
    let closer = table
        .borrow()
        .metatable
        .clone()
        .expect("no metatable")
        .borrow()
        .get_str("__close");
    probe::call(&closer, vec![handle.clone()]).expect("close");

    let refused = probe::method(&handle, "get", vec![Value::str("a")]).expect_err("still open");
    assert!(refused.to_string().contains("closed"), "{refused}");
}

/// A typo is a mistake rather than a new field.
#[test]
fn writing_a_field_on_a_handle_is_refused() {
    let (_dir, handle) = opened();
    let Value::Table(table) = &handle else {
        panic!("not a table")
    };
    let write = table
        .borrow()
        .metatable
        .clone()
        .expect("no metatable")
        .borrow()
        .get_str("__newindex");
    let refused = probe::call(
        &write,
        vec![handle.clone(), Value::str("nmae"), Value::int(1)],
    )
    .expect_err("the write should be refused");
    assert!(refused.to_string().contains("nmae"), "{refused}");
}

/// A name that could reach another database never gets as far as opening one.
#[test]
fn opening_a_name_that_is_not_a_name_is_a_message() {
    for bad in ["../history", "a/b", ""] {
        let refused = super::opened(bad).expect_err("should refuse");
        assert!(refused.contains("not a database name"), "{refused}");
    }
}
