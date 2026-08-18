//! The suggestion source lists, one per prompt.

use super::{Source, Suggest};

/// **The two prompts have separate source lists, and Lua's default holds no history.**
///
/// A Lua prompt should behave like an editor: it offers what *exists* in the session, not what was
/// typed last week. Sharing one list meant a config tuned for the shell — three of whose sources
/// answer with shell and cannot be written in Lua at all — silently decided what the Lua prompt
/// did, and what it usually decided was nothing.
#[test]
fn the_lua_prompt_has_its_own_sources_and_no_history() {
    let suggest = Suggest::default();
    assert_eq!(suggest.lua_sources, vec![Source::Completion]);
    assert!(
        !suggest.lua_sources.contains(&Source::History),
        "a Lua prompt is not a history prompt"
    );
    assert_eq!(
        suggest.sh_sources,
        vec![Source::History, Source::Completion, Source::Path]
    );
}

/// **`completion` is one source with two answers**, not two sources.
///
/// "Complete what is being typed" is the same idea at both prompts; only what it completes differs
/// — a command name at a shell prompt, a Lua name at a Lua one. There was briefly a separate
/// `names` source for the Lua half, which meant a config had to know which prompt it was writing
/// for to choose the right word. The Lua spellings are kept as aliases.
#[test]
fn the_lua_spellings_name_the_same_source() {
    for name in ["completion", "completions", "names", "name", "globals"] {
        assert_eq!(Source::parse(name), Some(Source::Completion), "{name}");
    }
    assert_eq!(Source::parse("nmaes"), None, "a typo is still reported");
}
