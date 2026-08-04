//! What a config's `oslo` table turns into.

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
    assert_eq!(
        huge.completion.max_rows,
        crate::interactive::dropdown::CEILING_ROWS
    );
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
    let (settings, _) =
        settings_from("oslo = { history = { size = 50000, file = '~/h', ignore_dups = true } }");
    assert_eq!(settings.history.size, Some(50000));
    assert_eq!(settings.history.file.as_deref(), Some("~/h"));
    assert!(settings.history.ignore_dups);
    // Untouched.
    assert!(settings.history.ignore_space);
}

/// fish's `fish_greeting`, and the two ways of being quiet.
#[test]
fn a_greeting_is_separate_from_the_banner() {
    let (default, _) = settings_from("oslo = {}");
    assert!(default.misc.welcome);
    assert_eq!(default.misc.greeting, None);

    let (custom, problems) = settings_from("oslo = { misc = { greeting = 'hello' } }");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(custom.misc.greeting.as_deref(), Some("hello"));
}

/// The escape delay is clamped rather than refused: zero would read every arrow key as Esc.
#[test]
fn the_escape_delay_is_clamped_to_something_usable() {
    let (default, _) = settings_from("oslo = {}");
    assert_eq!(default.misc.escape_delay, 25);

    let (zero, _) = settings_from("oslo = { misc = { escape_delay = 0 } }");
    assert_eq!(zero.misc.escape_delay, 1, "0 would break every arrow key");
    let (huge, _) = settings_from("oslo = { misc = { escape_delay = 99999 } }");
    assert_eq!(huge.misc.escape_delay, 2000);
    let (slow, _) = settings_from("oslo = { misc = { escape_delay = 300 } }");
    assert_eq!(slow.misc.escape_delay, 300, "an ssh session's value");
}

/// A depth nobody can spell is named, not silently ignored — the whole point of the override is
/// that the user knows better than detection, so a typo must not fall back to detection.
#[test]
fn an_unknown_colour_depth_is_named() {
    let (settings, problems) = settings_from("oslo = { misc = { color_depth = 'lots' } }");
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("lots"), "{problems:?}");
    assert_eq!(settings.misc.color_depth, None);

    let (ok, problems) = settings_from("oslo = { misc = { color_depth = 'truecolor' } }");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(ok.misc.color_depth.as_deref(), Some("truecolor"));
}

/// `$HISTIGNORE`'s list, and the notification's words.
#[test]
fn history_ignore_and_notify_text_are_read() {
    let (settings, problems) = settings_from(
        "oslo = { history = { ignore = {'ls', 'cd *'} },
                  notify = { after = 30, title = '{cmd}', command = 'notify-send x' } }",
    );
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(settings.history.ignore, vec!["ls", "cd *"]);
    assert_eq!(settings.notify.after, 30);
    assert_eq!(settings.notify_text.title.as_deref(), Some("{cmd}"));
    assert_eq!(
        settings.notify_text.command.as_deref(),
        Some("notify-send x")
    );
}

/// A pattern list with something in it that is not a pattern says so.
#[test]
fn a_history_ignore_entry_that_is_not_a_pattern_is_named() {
    let (settings, problems) = settings_from("oslo = { history = { ignore = {'ls', 42} } }");
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(settings.history.ignore, vec!["ls"], "the rest still apply");
}

/// `oslo.abbr` in both its forms. Until this existed the only way to define one was the `abbr`
/// builtin — a shell command, in a shell configured in Lua.
#[test]
fn abbreviations_are_read_in_both_forms() {
    let (settings, problems) = settings_from(
        "oslo = { abbr = {
             gco = 'git checkout',
             gst = 'git status',
             brc = { '~/.config/oslo/config.lua', anywhere = true },
             tmp = { expansion = '/tmp' },
         } }",
    );
    assert!(problems.is_empty(), "{problems:?}");
    // Sorted, because Lua table iteration has no order and two runs must install the same thing.
    assert_eq!(
        settings.abbr,
        vec![
            (
                "brc".to_string(),
                "~/.config/oslo/config.lua".to_string(),
                true
            ),
            ("gco".to_string(), "git checkout".to_string(), false),
            ("gst".to_string(), "git status".to_string(), false),
            ("tmp".to_string(), "/tmp".to_string(), false),
        ]
    );
}

/// An entry that expands to nothing usable is named rather than silently dropped: an abbreviation
/// that does not fire looks exactly like the shell ignoring the config.
#[test]
fn an_abbreviation_without_an_expansion_is_named() {
    let (settings, problems) =
        settings_from("oslo = { abbr = { gco = 'git checkout', bad = { anywhere = true } } }");
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("bad"), "{problems:?}");
    assert_eq!(settings.abbr.len(), 1, "the rest still apply");
}
