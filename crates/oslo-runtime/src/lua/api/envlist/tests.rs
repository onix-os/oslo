//! Reading the options table. What the list operations *do* is `oslo_shell::env::lists`' subject
//! and is tested there; what matters here is that `{ var = …, last = … }` reaches them.

use super::*;
use oslo_base::value::Table;

fn table_of(entries: &[(&str, Value)]) -> Value {
    let mut table = Table::new();
    for (name, value) in entries {
        table.set_str(name, value.clone());
    }
    Value::table(table)
}

#[test]
fn nothing_means_the_front_of_path() {
    assert_eq!(options(None), ("PATH".to_string(), false));
    // A stray argument that is not a table is ignored rather than refused — see `options`.
    assert_eq!(
        options(Some(&Value::str("MANPATH"))),
        ("PATH".to_string(), false)
    );
}

#[test]
fn the_options_table_names_the_variable_and_the_end() {
    let asked = table_of(&[("var", Value::str("MANPATH")), ("last", Value::Bool(true))]);
    assert_eq!(options(Some(&asked)), ("MANPATH".to_string(), true));

    let front = table_of(&[("var", Value::str("LD_LIBRARY_PATH"))]);
    assert_eq!(
        options(Some(&front)),
        ("LD_LIBRARY_PATH".to_string(), false)
    );

    // `last = false` is the default said out loud, and must not read as "last".
    let explicit = table_of(&[("last", Value::Bool(false))]);
    assert_eq!(options(Some(&explicit)), ("PATH".to_string(), false));
}
