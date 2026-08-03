//! The knobs a config sets, other than colours.
//!
//! Read from `oslo.completion`, `oslo.suggest`, `oslo.history` and `oslo.keys` once the config has
//! run, and merged over the defaults the same way [`super::theme`] is — naming one field must not
//! blank the rest.
//!
//! Kept apart from the theme because they answer different questions: a theme says what things
//! *look* like, and these say what the shell *does*. A user who wants a different colour scheme
//! and a user who wants fewer dropdown rows are not the same user.

use super::matching::Fuzzy;

/// Everything configurable that is not a colour.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub completion: Completion,
    pub suggest: Suggest,
    pub history: History,
    /// `oslo.vi`: whether vi mode is on, and the cursor for each mode.
    pub vi: Vi,
    /// `oslo.notify`: when a finished command is worth a desktop notification.
    pub notify: Notify,
    /// `oslo.finder`: the full-screen history search.
    pub finder: Finder,
    /// `oslo.dirs`: the directories `@name` reaches.
    ///
    /// Sorted, because table iteration has no order and a diagnostic that named them in a
    /// different order each run would be maddening to compare.
    pub dirs: Vec<(String, String)>,
    /// `oslo.keys`: key name to action name, both as written.
    ///
    /// Kept as strings rather than resolved here because binding needs rustyline types, and this
    /// module is meant to be readable without one. [`super::keys`] turns them into bindings and
    /// reports the ones it does not recognise.
    pub keys: Vec<(String, String)>,
}

/// `oslo.notify` — a desktop notification when a slow command finishes.
///
/// ```lua
/// oslo.notify = { after = 10 }   -- seconds; 0 turns it off
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notify {
    /// Seconds a command must run before finishing is worth telling you about. `0` never notifies.
    pub after: u64,
}

impl Default for Notify {
    fn default() -> Self {
        // Ten seconds: long enough that you have looked away, short enough to catch a test run.
        // A notification for something you sat and watched is noise, and noise gets muted.
        Notify { after: 10 }
    }
}

/// `oslo.vi` — vi mode, on fish's model.
///
/// ```lua
/// oslo.vi = {
///   enabled = true,
///   cursor_insert = "line",     -- fish's names, so a config need not be translated
///   cursor_normal = "block",
///   cursor_replace = "underscore",
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vi {
    pub enabled: bool,
    pub cursors: super::vi::Cursors,
}

impl Default for Vi {
    fn default() -> Self {
        // On by default. Emacs bindings are still the shell tradition, but oslo's are the ones a
        // user has to opt *out* of — `oslo.vi = { enabled = false }` — because a vi user who has
        // to discover a setting before the arrow keys behave has already had a bad first minute.
        Vi {
            enabled: true,
            cursors: super::vi::Cursors::default(),
        }
    }
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
    /// How far the dropdown will stretch to reach a candidate you did not prefix.
    ///
    /// Tried only after the three prefix matchers have all come back empty, so turning it on can
    /// never push a candidate you *did* prefix down the list. Free on the typing path either way:
    /// the dropdown is built once per Tab, not per keystroke.
    pub fuzzy: Fuzzy,
    /// How candidates are ordered: by how often they are used, or by name.
    pub sort: Sort,
    /// The kinds of candidate offered at all. `None` means every kind, which is the default.
    ///
    /// Kept as the kind strings the candidates already carry, so adding a kind does not mean
    /// touching an enum here as well.
    pub sources: Option<Vec<String>>,
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
            // On by default in the dropdown, which is the place it costs nothing and where you
            // have already said "help me" by pressing Tab.
            fuzzy: Fuzzy::Smart,
            sort: Sort::default(),
            sources: None,
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

/// `oslo.finder` — the full-screen history search.
///
/// Separate from `oslo.completion` because it answers a different question. Completion suggests
/// what a half-typed word *could* be; this searches what you have actually run. They share the
/// fuzzy setting, since "how loosely should matching work" is one preference and having two would
/// only ever be a way to set them inconsistently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finder {
    /// Off means Up keeps walking history a line at a time, as it always did.
    pub enabled: bool,
    /// The key that opens it. Any name `oslo.keys` understands.
    pub key: String,
    /// How many distinct commands to load. The finder reads once when it opens and filters in
    /// memory, so this bounds the work done on the keystroke that opens it — not per keystroke
    /// while you type.
    pub limit: usize,
}

impl Default for Finder {
    fn default() -> Self {
        Finder {
            enabled: true,
            // Up rather than Ctrl-R: Ctrl-R is muscle memory pointing at a *search*, and this is
            // first of all a history *list* — the thing you reach for by pressing Up. Both are
            // configurable, and Up still walks a line at a time when the finder is off.
            key: "up".to_string(),
            // Far more than anyone has, so the list is "everything" in practice, and still a bound
            // rather than an unbounded read on a store that has been collecting for years.
            limit: 10_000,
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
mod from_lua;
pub use from_lua::read_lua_settings;

use std::sync::RwLock;

static SETTINGS: RwLock<Option<Settings>> = RwLock::new(None);

/// `--vi` / `--no-vi`, which outrank whatever a config says.
///
/// Kept apart from [`Settings`] rather than folded into it because the config is read *after* the
/// command line and would otherwise overwrite the flag. A flag the user typed on this invocation
/// should beat a file they wrote once, which is the whole reason for having it.
static VI_OVERRIDE: RwLock<Option<bool>> = RwLock::new(None);

/// Force vi mode on or off for this session, or `None` to leave the config in charge.
pub fn force_vi(on: Option<bool>) {
    if let Ok(mut slot) = VI_OVERRIDE.write() {
        *slot = on;
    }
}

/// The settings in force.
pub fn current() -> Settings {
    let mut settings = SETTINGS
        .read()
        .ok()
        .and_then(|s| s.clone())
        .unwrap_or_default();
    if let Some(on) = VI_OVERRIDE.read().ok().and_then(|s| *s) {
        settings.vi.enabled = on;
    }
    settings
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

    /// An empty list is a real answer — "offer nothing" — and must not read as "offer everything".
    #[test]
    fn the_completion_sources_are_read_including_an_empty_list() {
        let (all, _) = settings_from("oslo = { completion = {} }");
        assert_eq!(all.completion.sources, None, "unset means every kind");

        let (some, problems) =
            settings_from("oslo = { completion = { sources = {'command', 'dir'} } }");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            some.completion.sources.as_deref(),
            Some(["command".to_string(), "dir".to_string()].as_slice())
        );

        let (none, _) = settings_from("oslo = { completion = { sources = {} } }");
        assert_eq!(none.completion.sources.as_deref(), Some([].as_slice()));
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
