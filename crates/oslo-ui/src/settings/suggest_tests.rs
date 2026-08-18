//! The suggestion source lists, one per prompt.

use super::{Source, Suggest};

/// **The two prompts have separate source lists, and Lua's default holds no history.**
///
/// A Lua prompt should behave like an editor: it offers what *exists* in the session, not what
/// was typed last week. Sharing one list meant a config tuned for the shell — three of whose
/// sources answer with shell and cannot be written in Lua at all — silently decided what the
/// Lua prompt did, and what it usually decided was nothing.
#[test]
fn the_lua_prompt_has_its_own_sources_and_no_history() {
    let suggest = Suggest::default();
    assert_eq!(suggest.lua_sources, vec![Source::Names]);
    assert!(
        !suggest.lua_sources.contains(&Source::History),
        "a Lua prompt is not a history prompt"
    );
    // The shell's own list is unchanged, and holds none of the Lua-shaped source.
    assert_eq!(
        suggest.sources,
        vec![Source::History, Source::Completion, Source::Path]
    );
    assert!(!suggest.sources.contains(&Source::Names));
}

/// Every spelling a config may write for the Lua source.
#[test]
fn the_names_source_is_named_the_way_people_say_it() {
    for name in ["names", "name", "globals"] {
        assert_eq!(Source::parse(name), Some(Source::Names), "{name}");
    }
    assert_eq!(Source::parse("nmaes"), None, "a typo is still reported");
}
