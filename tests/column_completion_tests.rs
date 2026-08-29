//! Naming a column is the most common thing typed at a structured prompt, and it was the one thing
//! with no help.
//!
//! `vocab` already fed the menu the *names* of the verbs — that is why `where` stopped being painted
//! as a command that does not exist. What it could not feed was the columns, because the declaration
//! stopped at the shape. `data::columns` carries the other half, and `data::complete` walks a
//! half-typed line to work out which stream is upstream of the cursor.

use oslo::env::Environment;
use oslo::ui::OsloHelper;
use std::sync::{Arc, Mutex};

/// The hook is thread-local and the REPL installs it on its own thread, so a test has to as well.
fn helper() -> OsloHelper {
    oslo::data::tools::register_all();
    oslo_ui::completion::set_column_source(Some(std::rc::Rc::new(|line: &str, pos: usize| {
        oslo::data::complete::columns_at(line, pos)
    })));
    let mut helper = OsloHelper::new(Arc::new(Mutex::new(Environment::new())));
    helper.set_menu(false);
    helper
}

fn offered(line: &str) -> Vec<String> {
    let helper = helper();
    let (_, candidates) = helper.candidates(line, line.len());
    candidates.iter().map(|c| c.display.clone()).collect()
}

/// The headline.
#[test]
fn a_producers_columns_are_offered_after_the_pipe() {
    let names = offered("ls | sort-by ");
    for wanted in ["name", "size", "modified", "is_dir"] {
        assert!(
            names.iter().any(|n| n == wanted),
            "`{wanted}` should be offered, got {names:?}"
        );
    }
}

/// A partly typed column narrows to the ones that start with it — `mode` and `modified` both do,
/// and both belong.
#[test]
fn a_prefix_narrows_the_offer() {
    let mut names = offered("ls | cols mod");
    names.sort();
    assert_eq!(names, ["mode", "modified"], "got {names:?}");

    let one = offered("ls | cols size_");
    assert_eq!(one, ["size_human"], "got {one:?}");
}

/// **The algebra follows the line**, so what an earlier verb did is reflected in what is offered.
#[test]
fn the_offer_follows_what_earlier_stages_did() {
    let after_reject = offered("ls | reject size | sort-by ");
    assert!(after_reject.iter().any(|n| n == "name"));
    assert!(
        !after_reject.iter().any(|n| n == "size"),
        "reject removed it: {after_reject:?}"
    );

    let after_rename = offered("ls | rename size bytes | sort-by ");
    assert!(
        after_rename.iter().any(|n| n == "bytes"),
        "{after_rename:?}"
    );
    assert!(!after_rename.iter().any(|n| n == "size"));

    let after_group = offered("ps | group-by name | cols ");
    assert!(after_group.iter().any(|n| n == "count"), "{after_group:?}");
    assert!(after_group.iter().any(|n| n == "rows"));
}

/// `parse` names its own columns, so the stage after it completes from the pattern alone.
///
/// Sorted before comparing: the menu orders every source by frecency then name, and a column is not
/// exempt from that.
#[test]
fn parse_feeds_the_next_stage() {
    let mut names = offered("cat /etc/passwd | parse '{user}:{x}:{uid}' | cols ");
    names.sort();
    assert_eq!(names, ["uid", "user", "x"], "got {names:?}");
}

/// **A position that is not a column falls through**, and must keep doing whatever it did before.
/// `ls <Tab>` is a directory, and offering `is_dir` there would be nonsense.
#[test]
fn a_path_position_is_not_hijacked() {
    let names = offered("ls ");
    assert!(
        !names.iter().any(|n| n == "is_dir"),
        "a column leaked into a path position: {names:?}"
    );
}

/// **A column position with nothing knowable offers nothing rather than filenames.** A filename
/// where a column belongs is the wrong nothing — the same rule a declared spec position follows.
#[test]
fn an_unknowable_stream_offers_nothing_rather_than_files() {
    let names = offered("cat x.json | from json | cols ");
    assert!(
        names.is_empty(),
        "a column position must not fall through to files: {names:?}"
    );
}

/// A column candidate carries its own kind, so `oslo.completion.sh_sources` can order or drop it
/// the way it does every other source.
#[test]
fn a_column_candidate_says_what_it_is() {
    let helper = helper();
    let (_, candidates) = helper.candidates("ls | cols na", 12);
    let column = candidates
        .iter()
        .find(|c| c.display == "name")
        .expect("name is offered");
    assert_eq!(column.kind.as_deref(), Some("column"));
}
