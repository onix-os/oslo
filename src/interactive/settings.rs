//! The knobs a config sets, other than colours.
//!
//! Read from `oslo.completion`, `oslo.suggest`, `oslo.history` and `oslo.keys` once the config has
//! run, and merged over the defaults the same way [`super::theme`] is — naming one field must not
//! blank the rest.
//!
//! Kept apart from the theme because they answer different questions: a theme says what things
//! *look* like, and these say what the shell *does*. A user who wants a different colour scheme
//! and a user who wants fewer dropdown rows are not the same user.

use crate::lua::eval::Value;

/// Everything configurable that is not a colour.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub completion: Completion,
    pub suggest: Suggest,
    pub history: History,
    /// `oslo.keys`: key name to action name, both as written.
    ///
    /// Kept as strings rather than resolved here because binding needs rustyline types, and this
    /// module is meant to be readable without one. [`super::keys`] turns them into bindings and
    /// reports the ones it does not recognise.
    pub keys: Vec<(String, String)>,
}

/// `oslo.completion`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// Rows the dropdown shows before it scrolls.
    pub max_rows: usize,
    /// Whether the description column is drawn at all.
    pub descriptions: bool,
    /// Whether the kind badge is drawn.
    pub show_kind: bool,
    /// Whether a candidate must match the typed case.
    pub case_sensitive: bool,
    /// How candidates are ordered: by how often they are used, or by name.
    pub sort: Sort,
}

/// `oslo.completion.sort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    /// Most-used first, name as the tie-break. Without this, `exit` suggests `exitsnoop-bpfcc`.
    #[default]
    Frecency,
    /// Name only, ignoring how often anything has been used.
    Alpha,
}

impl Default for Completion {
    fn default() -> Self {
        Completion {
            // IRIS's default, and about the most a dropdown can show without covering the work.
            max_rows: 15,
            descriptions: true,
            show_kind: true,
            case_sensitive: false,
            sort: Sort::default(),
        }
    }
}

/// Where a ghost suggestion may come from, and in what order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A line the user has actually run.
    History,
    /// A command name being typed.
    Completion,
    /// A file or directory.
    Path,
}

impl Source {
    fn parse(name: &str) -> Option<Source> {
        match name {
            "history" => Some(Source::History),
            "completion" | "completions" => Some(Source::Completion),
            "path" | "paths" | "file" => Some(Source::Path),
            _ => None,
        }
    }
}

/// `oslo.suggest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggest {
    /// The sources to try, in order. Empty turns suggestions off entirely.
    pub sources: Vec<Source>,
    /// The key that takes the whole ghost suggestion, e.g. `"right"`.
    ///
    /// The action was always bindable through `oslo.keys`, but not under the name the suggestion
    /// settings use — so a config that wrote `oslo.suggest.accept = "right"`, which is how fish
    /// spells it, silently bound nothing.
    pub accept: Option<String>,
    /// The key that takes one word of it.
    pub accept_word: Option<String>,
}

impl Default for Suggest {
    fn default() -> Self {
        // fish's order. History first because a line the user has run is a better guess than
        // anything that can be ranked.
        Suggest {
            sources: vec![Source::History, Source::Completion, Source::Path],
            accept: None,
            accept_word: None,
        }
    }
}

/// `oslo.history`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    pub size: Option<usize>,
    pub file: Option<String>,
    /// Whether a line beginning with a space is kept out of the history.
    pub ignore_space: bool,
    /// Whether a line identical to the one before it is dropped.
    ///
    /// Off by default, and that is not an aesthetic choice: dropping a repeat renumbers every
    /// later event, so `!-2` would point one line further back than it says.
    pub ignore_dups: bool,
}

impl Default for History {
    fn default() -> Self {
        History {
            size: None,
            file: None,
            ignore_space: true,
            ignore_dups: false,
        }
    }
}

/// Read the settings out of the `oslo` table, and what was wrong with them.
pub fn read_lua_settings(oslo: &Value) -> (Settings, Vec<String>) {
    let mut settings = Settings::default();
    let mut problems = Vec::new();
    let Value::Table(oslo) = oslo else {
        return (settings, problems);
    };
    let oslo = oslo.borrow();

    if let Value::Table(table) = oslo.get(&Value::str("completion")) {
        let table = table.borrow();
        if let Some(n) = number(&table, "max_rows") {
            // One row is still a dropdown; zero would be a dropdown that never appears, which is
            // what turning completion off looks like and is not what a row count means.
            settings.completion.max_rows = (n.max(1) as usize).min(super::dropdown::MAX_ROWS);
        }
        flag(
            &table,
            "descriptions",
            &mut settings.completion.descriptions,
        );
        flag(&table, "show_kind", &mut settings.completion.show_kind);
        flag(
            &table,
            "case_sensitive",
            &mut settings.completion.case_sensitive,
        );
        if let Value::Str(name) = table.get(&Value::str("sort")) {
            match name.as_ref() {
                "frecency" | "frequency" => settings.completion.sort = Sort::Frecency,
                "alpha" | "alphabetical" | "name" => settings.completion.sort = Sort::Alpha,
                other => problems.push(format!(
                    "oslo.completion.sort: '{other}' is not an order; use 'frecency' or 'alpha'"
                )),
            }
        }
    }

    if let Value::Table(table) = oslo.get(&Value::str("suggest")) {
        let table = table.borrow();
        if let Value::Table(list) = table.get(&Value::str("sources")) {
            let mut sources = Vec::new();
            for value in list.borrow().sequence() {
                let Value::Str(name) = value else { continue };
                match Source::parse(name) {
                    Some(source) if !sources.contains(&source) => sources.push(source),
                    // A duplicate is harmless and silently ignored; a name nothing answers to is
                    // a typo, and a typo that turns a source off without saying so is exactly the
                    // kind of thing that gets blamed on the shell.
                    Some(_) => {}
                    None => problems.push(format!(
                        "oslo.suggest.sources: '{name}' is not a source; \
                         the sources are history, completion and path"
                    )),
                }
            }
            settings.suggest.sources = sources;
        }
        if let Value::Str(key) = table.get(&Value::str("accept")) {
            settings.suggest.accept = Some(key.to_string());
        }
        if let Value::Str(key) = table.get(&Value::str("accept_word")) {
            settings.suggest.accept_word = Some(key.to_string());
        }
    }

    if let Value::Table(table) = oslo.get(&Value::str("keys")) {
        for (key, action) in table.borrow().pairs() {
            match (&key, &action) {
                (Value::Str(key), Value::Str(action)) => {
                    settings.keys.push((key.to_string(), action.to_string()));
                }
                _ => problems.push(
                    "oslo.keys: every entry must be a key name mapped to an action name"
                        .to_string(),
                ),
            }
        }
        // Table iteration has no order, and a binding that depends on which of two entries was
        // applied last is one that behaves differently between runs.
        settings.keys.sort();
    }

    if let Value::Table(table) = oslo.get(&Value::str("history")) {
        let table = table.borrow();
        if let Some(n) = number(&table, "size") {
            settings.history.size = Some(n.max(0) as usize);
        }
        if let Value::Str(file) = table.get(&Value::str("file")) {
            settings.history.file = Some(file.to_string());
        }
        flag(&table, "ignore_space", &mut settings.history.ignore_space);
        flag(&table, "ignore_dups", &mut settings.history.ignore_dups);
    }

    (settings, problems)
}

fn number(table: &crate::lua::eval::Table, name: &str) -> Option<i64> {
    table.get(&Value::str(name)).as_number()?.as_int()
}

/// A boolean field, left alone when the config does not mention it.
///
/// `false` and "absent" have to be told apart, or `descriptions = false` would be indistinguishable
/// from not setting it and could never turn anything off.
fn flag(table: &crate::lua::eval::Table, name: &str, slot: &mut bool) {
    match table.get(&Value::str(name)) {
        Value::Nil => {}
        value => *slot = value.truthy(),
    }
}

use std::sync::RwLock;

static SETTINGS: RwLock<Option<Settings>> = RwLock::new(None);

/// The settings in force.
pub fn current() -> Settings {
    SETTINGS
        .read()
        .ok()
        .and_then(|s| s.clone())
        .unwrap_or_default()
}

pub fn install(settings: Settings) {
    if let Ok(mut slot) = SETTINGS.write() {
        *slot = Some(settings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::eval;

    fn settings_from(source: &str) -> (Settings, Vec<String>) {
        let interp = eval::Interp::new("settings test");
        let ast = eval::parse(source).expect("the test chunk must parse");
        interp.run_ast(&ast).expect("the test chunk must run");
        read_lua_settings(&interp.global("oslo"))
    }

    #[test]
    fn naming_one_field_keeps_every_other_default() {
        let (settings, problems) = settings_from("oslo = { completion = { max_rows = 5 } }");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(settings.completion.max_rows, 5);
        assert!(settings.completion.descriptions);
        assert_eq!(settings.suggest, Suggest::default());
        assert_eq!(settings.history, History::default());
    }

    /// `false` and "not mentioned" have to be different, or nothing could ever be turned off.
    #[test]
    fn a_false_flag_is_not_the_same_as_an_absent_one() {
        let (off, _) = settings_from("oslo = { completion = { show_kind = false } }");
        assert!(!off.completion.show_kind);
        let (absent, _) = settings_from("oslo = { completion = {} }");
        assert!(absent.completion.show_kind);
    }

    #[test]
    fn the_completion_order_is_read_and_a_typo_is_named() {
        let (alpha, problems) = settings_from("oslo = { completion = { sort = 'alpha' } }");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(alpha.completion.sort, Sort::Alpha);
        assert_eq!(Settings::default().completion.sort, Sort::Frecency);

        let (kept, problems) = settings_from("oslo = { completion = { sort = 'vibes' } }");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("vibes"));
        assert_eq!(
            kept.completion.sort,
            Sort::Frecency,
            "a typo keeps the default"
        );
    }

    /// `oslo.suggest.accept = "right"` is how fish spells it, and it used to bind nothing.
    #[test]
    fn the_suggestion_keys_are_read_under_their_own_names() {
        let (settings, problems) =
            settings_from("oslo = { suggest = { accept = 'right', accept_word = 'alt-right' } }");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(settings.suggest.accept.as_deref(), Some("right"));
        assert_eq!(settings.suggest.accept_word.as_deref(), Some("alt-right"));
        // Naming them does not disturb the sources.
        assert_eq!(settings.suggest.sources, Suggest::default().sources);
    }

    #[test]
    fn suggestion_sources_keep_the_order_they_were_written_in() {
        let (settings, problems) =
            settings_from("oslo = { suggest = { sources = {'path', 'history'} } }");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            settings.suggest.sources,
            vec![Source::Path, Source::History]
        );

        // An empty list turns suggestions off, which is a thing someone may want.
        let (off, _) = settings_from("oslo = { suggest = { sources = {} } }");
        assert!(off.suggest.sources.is_empty());
    }

    /// A typo that silently turns a source off is the kind of thing that gets blamed on the shell.
    #[test]
    fn an_unknown_source_is_named() {
        let (settings, problems) =
            settings_from("oslo = { suggest = { sources = {'history', 'psychic'} } }");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("psychic"), "{problems:?}");
        // The ones that were understood still take effect.
        assert_eq!(settings.suggest.sources, vec![Source::History]);
    }

    #[test]
    fn a_row_count_is_clamped_to_something_drawable() {
        let (tiny, _) = settings_from("oslo = { completion = { max_rows = 0 } }");
        assert_eq!(tiny.completion.max_rows, 1);
        let (huge, _) = settings_from("oslo = { completion = { max_rows = 9999 } }");
        assert_eq!(huge.completion.max_rows, super::super::dropdown::MAX_ROWS);
    }

    #[test]
    fn key_bindings_are_read_in_a_stable_order() {
        let (settings, problems) = settings_from(
            "oslo = { keys = { ['ctrl-l'] = 'clear-screen', ['shift-tab'] = 'toggle-language' } }",
        );
        assert!(problems.is_empty(), "{problems:?}");
        // Sorted, because table iteration has no order and a binding that depends on it would
        // behave differently between runs.
        assert_eq!(
            settings.keys,
            vec![
                ("ctrl-l".to_string(), "clear-screen".to_string()),
                ("shift-tab".to_string(), "toggle-language".to_string()),
            ]
        );
    }

    #[test]
    fn history_settings_are_read_through() {
        let (settings, _) = settings_from(
            "oslo = { history = { size = 50000, file = '~/h', ignore_dups = true } }",
        );
        assert_eq!(settings.history.size, Some(50000));
        assert_eq!(settings.history.file.as_deref(), Some("~/h"));
        assert!(settings.history.ignore_dups);
        // Untouched.
        assert!(settings.history.ignore_space);
    }
}
