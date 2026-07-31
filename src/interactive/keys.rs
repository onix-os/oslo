//! Turning `oslo.keys` into bindings the line editor understands.
//!
//! Keys are named — `ctrl-r`, `shift-tab`, `f2` — rather than written as escape sequences,
//! because the sequence a key produces depends on the terminal, which is the thing a user
//! rebinding it is usually trying to work around.
//!
//! An unrecognised key or action is *reported*, never ignored. A binding that silently does
//! nothing is indistinguishable from a shell that ignores the config, and it is the failure mode
//! that costs the most time to diagnose.

use rustyline::{Cmd, KeyCode, KeyEvent, Modifiers};

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
}

impl Action {
    fn parse(name: &str) -> Option<Action> {
        match name {
            "toggle-language" | "toggle-mode" => Some(Action::ToggleLanguage),
            "clear-screen" => Some(Action::ClearScreen),
            "history-search" | "history-search-backward" => Some(Action::HistorySearchBackward),
            "accept-suggestion" => Some(Action::AcceptSuggestion),
            "accept-suggestion-word" | "accept-word" => Some(Action::AcceptSuggestionWord),
            "interrupt" => Some(Action::Interrupt),
            "complete" => Some(Action::Complete),
            _ => None,
        }
    }

    /// The editor command, or `None` for an action the loop handles itself.
    pub fn command(self) -> Option<Cmd> {
        Some(match self {
            Action::ToggleLanguage => return None,
            Action::ClearScreen => Cmd::ClearScreen,
            Action::HistorySearchBackward => Cmd::ReverseSearchHistory,
            Action::AcceptSuggestion => Cmd::CompleteHint,
            Action::AcceptSuggestionWord => Cmd::Move(rustyline::Movement::ForwardWord(
                1,
                rustyline::At::AfterEnd,
                rustyline::Word::Emacs,
            )),
            Action::Interrupt => Cmd::Interrupt,
            Action::Complete => Cmd::Complete,
        })
    }
}

/// A key written the way a person says it: `ctrl-r`, `alt-f`, `shift-tab`, `f2`, `enter`.
pub fn parse_key(name: &str) -> Option<KeyEvent> {
    let lower = name.trim().to_ascii_lowercase();

    // The named keys first, so `shift-tab` is not read as shift plus the letter `t`.
    match lower.as_str() {
        "backtab" | "shift-tab" | "s-tab" => {
            return Some(KeyEvent(KeyCode::BackTab, Modifiers::NONE));
        }
        "tab" => return Some(KeyEvent(KeyCode::Tab, Modifiers::NONE)),
        "enter" | "return" => return Some(KeyEvent(KeyCode::Enter, Modifiers::NONE)),
        "esc" | "escape" => return Some(KeyEvent(KeyCode::Esc, Modifiers::NONE)),
        "up" => return Some(KeyEvent(KeyCode::Up, Modifiers::NONE)),
        "down" => return Some(KeyEvent(KeyCode::Down, Modifiers::NONE)),
        "left" => return Some(KeyEvent(KeyCode::Left, Modifiers::NONE)),
        "right" => return Some(KeyEvent(KeyCode::Right, Modifiers::NONE)),
        "home" => return Some(KeyEvent(KeyCode::Home, Modifiers::NONE)),
        "end" => return Some(KeyEvent(KeyCode::End, Modifiers::NONE)),
        "delete" => return Some(KeyEvent(KeyCode::Delete, Modifiers::NONE)),
        "backspace" => return Some(KeyEvent(KeyCode::Backspace, Modifiers::NONE)),
        _ => {}
    }
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok())
        && (1..=12).contains(&n)
    {
        return Some(KeyEvent(KeyCode::F(n), Modifiers::NONE));
    }

    // Then the modifier forms.
    for (prefix, build) in [
        ("ctrl-", 0u8),
        ("c-", 0),
        ("alt-", 1),
        ("m-", 1),
        ("meta-", 1),
    ] {
        if let Some(rest) = lower.strip_prefix(prefix)
            && let Some(c) = one_char(rest)
        {
            return Some(if build == 0 {
                KeyEvent::ctrl(c)
            } else {
                KeyEvent::alt(c)
            });
        }
    }

    // A bare character binds itself, which is how a config rebinds punctuation.
    one_char(&lower).map(|c| KeyEvent(KeyCode::Char(c), Modifiers::NONE))
}

fn one_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// Resolve `oslo.keys` into bindings, and the complaints about what could not be read.
pub fn resolve(pairs: &[(String, String)]) -> (Vec<(KeyEvent, Action)>, Vec<String>) {
    let mut bindings = Vec::new();
    let mut problems = Vec::new();
    for (key, action) in pairs {
        let Some(event) = parse_key(key) else {
            problems.push(format!("oslo.keys: '{key}' is not a key name"));
            continue;
        };
        let Some(action) = Action::parse(action) else {
            problems.push(format!(
                "oslo.keys['{key}']: '{action}' is not an action; the actions are \
                 toggle-language, clear-screen, history-search, accept-suggestion, \
                 accept-suggestion-word, interrupt and complete"
            ));
            continue;
        };
        bindings.push((event, action));
    }
    (bindings, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_named_the_way_people_say_them() {
        assert_eq!(
            parse_key("shift-tab"),
            Some(KeyEvent(KeyCode::BackTab, Modifiers::NONE))
        );
        assert_eq!(parse_key("backtab"), parse_key("shift-tab"));
        assert_eq!(parse_key("ctrl-r"), Some(KeyEvent::ctrl('r')));
        assert_eq!(parse_key("c-r"), parse_key("ctrl-r"));
        assert_eq!(parse_key("alt-f"), Some(KeyEvent::alt('f')));
        assert_eq!(
            parse_key("f2"),
            Some(KeyEvent(KeyCode::F(2), Modifiers::NONE))
        );
        assert_eq!(
            parse_key("Enter"),
            Some(KeyEvent(KeyCode::Enter, Modifiers::NONE))
        );
    }

    /// `shift-tab` must not be read as shift plus the letter `t`, which is what a naive
    /// prefix-then-character parse does.
    #[test]
    fn a_named_key_wins_over_a_modifier_reading() {
        assert_eq!(
            parse_key("tab"),
            Some(KeyEvent(KeyCode::Tab, Modifiers::NONE))
        );
        assert_ne!(parse_key("shift-tab"), parse_key("tab"));
    }

    /// A binding that silently does nothing is indistinguishable from a shell ignoring the config.
    #[test]
    fn an_unreadable_key_or_action_is_reported_by_name() {
        let (bindings, problems) = resolve(&[
            ("ctrl-l".into(), "clear-screen".into()),
            ("wibble".into(), "clear-screen".into()),
            ("ctrl-x".into(), "make-coffee".into()),
        ]);
        assert_eq!(bindings.len(), 1);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("wibble"), "{problems:?}");
        assert!(problems[1].contains("make-coffee"), "{problems:?}");
        // And the message says what the actions actually are.
        assert!(problems[1].contains("clear-screen"), "{problems:?}");
    }

    /// The toggle is the loop's, not the editor's: rustyline has no command that hands control
    /// back, so it is bound separately.
    #[test]
    fn the_language_toggle_has_no_editor_command() {
        assert_eq!(Action::ToggleLanguage.command(), None);
        assert!(Action::ClearScreen.command().is_some());
    }
}
