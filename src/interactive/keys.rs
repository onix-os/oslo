//! Turning `oslo.keys` into bindings the line editor understands.
//!
//! Keys are named — `ctrl-r`, `shift-tab`, `f2` — rather than written as escape sequences,
//! because the sequence a key produces depends on the terminal, which is the thing a user
//! rebinding it is usually trying to work around.
//!
//! An unrecognised key or action is *reported*, never ignored. A binding that silently does
//! nothing is indistinguishable from a shell that ignores the config, and it is the failure mode
//! that costs the most time to diagnose.

/// What a bound key does.
///
/// A fixed list rather than an open one: an action nothing answers to is a typo, and the point of
/// naming them is that the shell can say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Switch the prompt between shell and Lua. Handled by the loop, not by rustyline.
    ToggleLanguage,
    ClearScreen,
    HistorySearchBackward,
    /// Take the whole ghost suggestion.
    AcceptSuggestion,
    /// Take one word of it.
    AcceptSuggestionWord,
    Interrupt,
    Complete,
    /// A function the config supplied. The function itself lives in [`super::editor`]; this only
    /// records that the key has one, because an `Action` has to stay plain data.
    LuaHandler,
}

/// The action a name stands for, for callers outside `oslo.keys` that bind by name.
pub fn action(name: &str) -> Option<Action> {
    Action::parse(name)
}

impl Action {
    fn parse(name: &str) -> Option<Action> {
        match name {
            "toggle-language" | "toggle-mode" => Some(Action::ToggleLanguage),
            "clear-screen" => Some(Action::ClearScreen),
            super::editor::ACTION => Some(Action::LuaHandler),
            "history-search" | "history-search-backward" => Some(Action::HistorySearchBackward),
            "accept-suggestion" => Some(Action::AcceptSuggestion),
            "accept-suggestion-word" | "accept-word" => Some(Action::AcceptSuggestionWord),
            "interrupt" => Some(Action::Interrupt),
            "complete" => Some(Action::Complete),
            _ => None,
        }
    }
}

/// Whether `name` is a key oslo can recognise, in the spelling a config writes.
///
/// A *name* check, not a parse into an editor event: the editor is oslo's own now and matches on
/// these names directly. This exists so a typo in `oslo.finder.key` is reported next to the line
/// that wrote it rather than leaving the finder quietly unreachable.
pub fn is_key_name(name: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "tab"
            | "shift-tab"
            | "backtab"
            | "s-tab"
            | "enter"
            | "return"
            | "esc"
            | "escape"
            | "up"
            | "down"
            | "left"
            | "right"
            | "home"
            | "end"
            | "pageup"
            | "pagedown"
            | "backspace"
            | "delete"
            | "space"
    ) {
        return true;
    }
    if let Some(rest) = name.strip_prefix("f")
        && rest.parse::<u8>().is_ok_and(|n| (1..=12).contains(&n))
    {
        return true;
    }
    ["ctrl-", "alt-", "meta-", "shift-"].iter().any(|prefix| {
        name.strip_prefix(prefix)
            .is_some_and(|c| c.chars().count() == 1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names a config may write. Checked rather than parsed into an editor event: the editor
    /// is oslo's own and matches on these names, so what matters is that a name is *recognised*.
    #[test]
    fn keys_are_named_the_way_people_say_them() {
        for name in [
            "shift-tab",
            "backtab",
            "s-tab",
            "tab",
            "enter",
            "esc",
            "ctrl-r",
            "alt-f",
            "f2",
            "f12",
            "up",
            "pagedown",
            "space",
        ] {
            assert!(is_key_name(name), "{name} should be a key name");
        }
        // Case does not matter, because a config is written by a person.
        assert!(is_key_name("Ctrl-R"));
        assert!(is_key_name("  enter  "));
    }

    /// A name oslo cannot place is refused, so a typo in `oslo.finder.key` is reported next to
    /// the line that wrote it rather than leaving the finder quietly unreachable.
    #[test]
    fn a_name_oslo_cannot_place_is_refused() {
        for name in ["", "ctrl-", "ctrl-shift-r", "f13", "wiggle", "alt-"] {
            assert!(!is_key_name(name), "{name:?} should not be a key name");
        }
    }

    /// The action vocabulary a config binds a key to. Every spelling here is one somebody has in
    /// a config already.
    #[test]
    fn action_names_resolve() {
        assert_eq!(action("toggle-language"), Some(Action::ToggleLanguage));
        assert_eq!(action("toggle-mode"), Some(Action::ToggleLanguage));
        assert_eq!(action("clear-screen"), Some(Action::ClearScreen));
        assert_eq!(
            action("history-search"),
            Some(Action::HistorySearchBackward)
        );
        assert_eq!(action("accept-suggestion"), Some(Action::AcceptSuggestion));
        assert_eq!(action("accept-word"), Some(Action::AcceptSuggestionWord));
        assert_eq!(action("interrupt"), Some(Action::Interrupt));
        assert_eq!(action("complete"), Some(Action::Complete));
        assert_eq!(action("do-a-barrel-roll"), None);
    }
}
