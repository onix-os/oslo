use super::*;

#[test]
fn a_plugin_that_declared_nothing_has_no_tests() {
    assert!(run("never-registered-anything").is_empty());
}

#[test]
fn the_table_offers_test_beside_health() {
    let Value::Table(built) = super::super::health::build() else {
        panic!("not a table")
    };
    let built = built.borrow();
    assert!(matches!(built.get(&Value::str("test")), Value::Function(_)));
    assert!(matches!(
        built.get(&Value::str("health")),
        Value::Function(_)
    ));
}

#[test]
fn a_message_is_the_third_argument_and_a_missing_one_still_names_something() {
    assert_eq!(said(&[Value::str("a"), Value::str("b")], 1), "b");
    assert_eq!(said(&[], 0), "unnamed check");
    // A non-string message is not a message: a caller who passed a table meant it as a value.
    assert_eq!(said(&[Value::Bool(true)], 0), "unnamed check");
}

#[test]
fn an_outcome_with_no_failures_passed() {
    let clean = Outcome {
        name: "a".into(),
        checked: 1,
        failures: Vec::new(),
    };
    assert!(clean.passed());
    let broken = Outcome {
        failures: vec![Failure { says: "no".into() }],
        ..clean
    };
    assert!(!broken.passed());
}
