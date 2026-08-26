use super::*;
use crate::spec::action::Action;

fn values(action: &Action) -> Vec<String> {
    resolve(action, &Query::default())
        .offers
        .into_iter()
        .map(|o| o.value)
        .collect()
}

#[test]
fn literals_come_back_in_the_order_they_were_written() {
    let action = Action::list(["one", "two\tthe second", "three"]);
    assert_eq!(values(&action), vec!["one", "two", "three"]);
    let resolved = resolve(&action, &Query::default());
    assert_eq!(
        resolved.offers[1].description.as_deref(),
        Some("the second")
    );
    assert!(resolved.paths.is_none());
}

#[test]
fn a_file_macro_asks_for_path_completion_rather_than_answering_itself() {
    let resolved = resolve(&Action::list(["$files([.go, .rs])"]), &Query::default());
    assert!(resolved.offers.is_empty());
    assert_eq!(
        resolved.paths,
        Some(Paths {
            suffixes: vec![".go".into(), ".rs".into()],
            ..Paths::default()
        })
    );

    let dirs = resolve(&Action::list(["$directories"]), &Query::default());
    assert_eq!(dirs.paths.map(|p| p.only_dirs), Some(true));
}

/// A position can offer both: three names it knows, and then whatever is on disk.
#[test]
fn a_position_can_name_values_and_ask_for_files_too() {
    let resolved = resolve(&Action::list(["-", "$files"]), &Query::default());
    assert_eq!(resolved.offers.len(), 1);
    assert!(resolved.paths.is_some());
}

#[test]
fn a_batch_modifier_reaches_everything_before_it() {
    let resolved = resolve(
        &Action::list(["one", "two", "three", "$filter([two])"]),
        &Query::default(),
    );
    let names: Vec<_> = resolved.offers.iter().map(|o| &o.value).collect();
    assert_eq!(names, vec!["one", "three"]);
}

/// …and one behind a `|||` reaches only the entry it is attached to.
#[test]
fn an_attached_modifier_reaches_only_its_own_entry() {
    let resolved = resolve(
        &Action::list(["kept", "tagged ||| $tag(marked)"]),
        &Query::default(),
    );
    assert_eq!(resolved.offers[0].tag, None);
    assert_eq!(resolved.offers[1].tag.as_deref(), Some("marked"));
}

#[test]
fn filterargs_drops_what_the_line_already_has() {
    let query = Query {
        args: vec!["two".into()],
        ..Query::default()
    };
    let resolved = resolve(
        &Action::list(["one", "two", "three", "$filterargs"]),
        &query,
    );
    let names: Vec<_> = resolved.offers.iter().map(|o| &o.value).collect();
    assert_eq!(names, vec!["one", "three"]);
}

#[test]
fn a_list_modifier_says_the_word_is_delimited() {
    let resolved = resolve(
        &Action::list(["a", "b", "$uniquelist(,)"]),
        &Query::default(),
    );
    assert_eq!(resolved.split.as_deref(), Some(","));
    assert!(resolved.unique);
}

#[test]
fn prefix_and_suffix_decorate_what_was_produced() {
    let resolved = resolve(
        &Action::list(["apple", "melon", "$suffix(juice)"]),
        &Query::default(),
    );
    let names: Vec<_> = resolved.offers.iter().map(|o| o.value.clone()).collect();
    assert_eq!(names, vec!["applejuice", "melonjuice"]);
}

/// **A macro needing a shell answers nothing when nothing installed one.** Non-interactive oslo has
/// no runner, and a spec mentioning `$(…)` must still complete its literals rather than panicking.
#[test]
fn a_shell_macro_with_no_runner_is_simply_quiet() {
    crate::spec::action::set_runner(None);
    let resolved = resolve(
        &Action::list(["local", "$(echo remote)"]),
        &Query::default(),
    );
    assert_eq!(values(&Action::list(["local"])), vec!["local"]);
    assert_eq!(resolved.offers.len(), 1);
}

#[test]
fn the_runner_answers_the_macros_this_crate_cannot() {
    crate::spec::action::set_runner(Some(std::rc::Rc::new(|name, arg, _| {
        vec![Offer::plain(format!("{name}:{arg}"))]
    })));
    let resolved = resolve(&Action::list(["$(echo hi)"]), &Query::default());
    assert_eq!(resolved.offers[0].value, ":echo hi");
    crate::spec::action::set_runner(None);
}

/// A function is a position's answer in the language a config is already written in.
#[test]
fn a_computed_position_is_asked_at_tab_time() {
    let action = Action::Call(std::rc::Rc::new(|query: &Query| {
        vec![Offer::plain(format!("saw-{}", query.value))]
    }));
    let query = Query {
        value: "abc".into(),
        ..Query::default()
    };
    assert_eq!(resolve(&action, &query).offers[0].value, "saw-abc");
}
