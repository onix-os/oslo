//! The shape of a report. What it says about a *real* installation is covered end to end by
//! `tests/plugin_tests.rs`, where a whole home can be temporary.

use super::*;

#[test]
fn nothing_installed_is_a_clean_report_rather_than_an_empty_one() {
    // The path always has entries even on a fresh machine, so a report is never empty.
    let found = report();
    assert!(!found.is_empty(), "a report always says something");
}

#[test]
fn a_finding_carries_which_plugin_it_is_about() {
    let finding = Finding::new("notes", State::Bad, "it is not there");
    assert_eq!(finding.plugin, "notes");
    assert_eq!(finding.state, State::Bad);
    assert!(finding.says.contains("not there"));
}

/// A name that is not on the path cannot be checked, and saying so beats an empty answer.
#[test]
fn asking_a_plugin_that_is_not_on_the_path_says_so() {
    let found = checks_from("never-installed");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].state, State::Bad);
    assert!(
        found[0].says.contains("not on the runtimepath"),
        "{}",
        found[0].says
    );
}
