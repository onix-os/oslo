//! Argument reading, which is all of this module that does not need a nix to talk to. The
//! invocation itself is tested against a fake nix in `oslo_shell::nix_shell::json`.

use super::*;

/// `oslo.nix.run{…}` as it arrives: a Lua table with positional and named entries.
fn request(positional: &[&str], named: &[(&str, Value)]) -> LuaResult<Request> {
    let mut table = Table::new();
    for (i, arg) in positional.iter().enumerate() {
        table.set(Value::int(i as i64 + 1), Value::str(*arg));
    }
    for (key, value) in named {
        table.set(Value::str(*key), value.clone());
    }
    Request::from_lua(Some(&Value::table(table)))
}

#[test]
fn the_positional_entries_become_nix_arguments_in_order() {
    let read = request(&["flake", "metadata", "--flake", "nixpkgs"], &[]).expect("read");
    assert_eq!(read.argv, ["flake", "metadata", "--flake", "nixpkgs"]);
}

#[test]
fn a_number_is_a_word_and_a_table_is_not() {
    let mut table = Table::new();
    table.set(Value::int(1), Value::str("eval"));
    table.set(Value::int(2), Value::int(42));
    let read = Request::from_lua(Some(&Value::table(table))).expect("read");
    assert_eq!(read.argv, ["eval", "42"]);

    let mut nested = Table::new();
    nested.set(Value::int(1), Value::str("eval"));
    nested.set(Value::int(2), Value::table(Table::new()));
    let refused = Request::from_lua(Some(&Value::table(nested))).expect_err("should refuse");
    assert!(refused.to_string().contains("argument #2"), "{refused}");
}

#[test]
fn the_timeout_defaults_and_is_overridable_in_seconds() {
    assert_eq!(
        request(&["flake", "show"], &[]).expect("read").timeout,
        json::TIMEOUT
    );
    let asked = request(&["search"], &[("timeout", Value::int(5))]).expect("read");
    assert_eq!(asked.timeout, std::time::Duration::from_secs(5));
}

#[test]
fn a_timeout_that_cannot_be_waited_for_falls_back_rather_than_killing_at_once() {
    // `timeout = 0` reads as "kill it before it starts", which nobody writes on purpose.
    for silly in [Value::int(0), Value::int(-1), Value::str("soon")] {
        let read = request(&["flake", "show"], &[("timeout", silly)]).expect("read");
        assert_eq!(read.timeout, json::TIMEOUT);
    }
}

#[test]
fn calling_it_with_no_table_says_what_was_expected() {
    let refused = Request::from_lua(None).expect_err("should refuse");
    assert!(
        refused.to_string().contains("expected a table"),
        "{refused}"
    );
    let refused = Request::from_lua(Some(&Value::str("flake"))).expect_err("should refuse");
    assert!(refused.to_string().contains("got string"), "{refused}");
}

#[test]
fn an_empty_table_is_refused_rather_than_running_a_bare_nix() {
    let refused = request(&[], &[]).expect_err("should refuse");
    assert!(refused.to_string().contains("no arguments"), "{refused}");
}

/// The two Rust names, and the helpers `nix.lua` adds on top of them.
///
/// That the chunk parses and runs at all is worth asserting here: it is `include_str!`d, so a typo
/// in it is not a compile error, and startup only prints a complaint that nothing would read.
#[test]
fn the_table_offers_the_primitives_and_the_helpers() {
    // The helpers are added *in Lua*, by indexing the global — see `add_helpers` — so this needs a
    // VM with `oslo.nix` standing in it, and reads the result back out of the global.
    let engine = oslo_luavm::Engine::new();
    let mut oslo = Table::new();
    oslo.set_str("nix", build());
    oslo_luavm::Host::set_global(&engine, "oslo", Value::table(oslo));
    super::add_helpers(&engine);

    let Value::Table(oslo) = oslo_luavm::Host::global(&engine, "oslo") else {
        panic!("oslo is not a table")
    };
    let Value::Table(built) = oslo.borrow().get_str("nix") else {
        panic!("not a table")
    };
    let built = built.borrow();
    for name in [
        "run",
        "available",
        "metadata",
        "outputs",
        "config",
        "system",
        "dirty",
        "inputs",
        "shells",
    ] {
        assert!(
            matches!(built.get(&Value::str(name)), Value::Function(_)),
            "no {name}"
        );
    }
}
