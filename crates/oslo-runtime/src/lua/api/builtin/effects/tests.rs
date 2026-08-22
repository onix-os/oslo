//! Reading a returned table. Pure — no `Environment` is touched, which is the property that lets a
//! malformed return apply nothing at all.

use super::*;
use oslo_base::value::Table;

fn table(entries: Vec<(&str, Value)>) -> Table {
    let mut it = Table::new();
    for (name, value) in entries {
        it.set_str(name, value);
    }
    it
}

fn list(words: &[&str]) -> Value {
    let mut it = Table::new();
    for (i, word) in words.iter().enumerate() {
        it.set(Value::int(i as i64 + 1), Value::str(*word));
    }
    Value::table(it)
}

fn map(entries: &[(&str, &str)]) -> Value {
    let mut it = Table::new();
    for (name, value) in entries {
        it.set_str(name, Value::str(*value));
    }
    Value::table(it)
}

#[test]
fn an_empty_table_is_no_effects_and_success() {
    let parsed = parse(&Table::new(), Allow::All, "probe").expect("empty is valid");
    assert!(parsed.set.is_empty());
    assert!(parsed.status.is_none());
}

#[test]
fn each_field_reads_into_its_own_place() {
    let it = table(vec![
        ("set", map(&[("A", "1")])),
        ("export", map(&[("B", "2")])),
        ("unset", list(&["C"])),
        ("positional", list(&["x", "y"])),
        ("cd", Value::str("/tmp")),
        ("status", Value::int(3)),
    ]);
    let parsed = parse(&it, Allow::All, "probe").expect("valid");
    assert_eq!(parsed.set, vec![("A".to_string(), "1".to_string())]);
    assert_eq!(parsed.export, vec![("B".to_string(), "2".to_string())]);
    assert_eq!(parsed.unset, vec!["C".to_string()]);
    assert_eq!(
        parsed.positional.as_deref(),
        Some(&["x".to_string(), "y".to_string()][..])
    );
    assert_eq!(parsed.cd.as_deref(), Some("/tmp"));
}

/// A number is a word, because `set = { N = 5 }` is a reasonable thing to write and the shell
/// stores strings anyway.
#[test]
fn a_number_is_accepted_where_a_word_goes() {
    let mut values = Table::new();
    values.set_str("N", Value::int(5));
    let it = table(vec![("set", Value::table(values))]);
    let parsed = parse(&it, Allow::All, "probe").expect("a number is a word");
    assert_eq!(parsed.set, vec![("N".to_string(), "5".to_string())]);
}

/// `false` removes an alias, which reads as the opposite of setting one.
#[test]
fn an_alias_is_set_or_removed() {
    let mut aliases = Table::new();
    aliases.set_str("g", Value::str("git"));
    aliases.set_str("ll", Value::Bool(false));
    let it = table(vec![("alias", Value::table(aliases))]);
    let parsed = parse(&it, Allow::All, "probe").expect("valid");
    assert!(
        parsed
            .alias
            .contains(&("g".to_string(), Some("git".to_string())))
    );
    assert!(parsed.alias.contains(&("ll".to_string(), None)));
}

/// **An unknown key raises and names the set.** A silently ignored `setenv = {…}` is the failure
/// this whole contract exists to close, so it must not be reintroduced by the contract itself.
#[test]
fn an_unknown_key_is_refused_and_lists_the_real_ones() {
    let it = table(vec![("setenv", map(&[("A", "1")]))]);
    let refused = parse(&it, Allow::All, "probe").expect_err("setenv is not an effect");
    let message = refused.to_string();
    assert!(message.contains("setenv"), "{message}");
    assert!(
        message.contains("export"),
        "the accepted set is not named: {message}"
    );
}

#[test]
fn a_wrongly_typed_field_is_refused() {
    for (name, value) in [
        ("set", Value::int(5)),
        ("unset", Value::str("A")),
        ("cd", Value::Bool(true)),
        ("alias", Value::int(1)),
    ] {
        let it = table(vec![(name, value)]);
        assert!(
            parse(&it, Allow::All, "probe").is_err(),
            "{name} accepted a wrong type"
        );
    }
}

/// `cd` from `pre-change-dir` would re-enter the directory change it is answering about. Refused at
/// parse, which is the difference between a clear message and a stack overflow.
#[test]
fn cd_is_refused_where_it_would_re_enter() {
    let it = table(vec![("cd", Value::str("/tmp"))]);
    assert!(parse(&it, Allow::All, "probe").is_ok());
    let refused = parse(&it, Allow::NoCd, "pre-change-dir").expect_err("cd must be refused here");
    assert!(refused.to_string().contains("re-enter"), "{refused}");
}

/// The refusal names the builtin, so a config with twenty of them can find the one at fault.
#[test]
fn a_refusal_names_its_owner() {
    let it = table(vec![("nonsense", Value::int(1))]);
    let refused = parse(&it, Allow::All, "mybuiltin").expect_err("refused");
    assert!(refused.to_string().contains("mybuiltin"), "{refused}");
}
