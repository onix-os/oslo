//! Which language the prompt is reading, and how you change it.
//!
//! oslo's prompt reads *one* language at a time. Shift+Tab switches, and the two are never mixed
//! on a line — a line is shell or it is Lua, never a guess. Mixing them would mean deciding what
//! `print(1)` is by looking at it, and a shell whose meaning depends on what happens to be
//! installed is a shell you cannot write scripts against.
//!
//! For the one-off there are prefixes, which do not change the mode:
//!
//! ```text
//! oslo$ =print(1 + 1)      -- one Lua line, from shell mode
//! lua>  !ls -la            -- one shell line, from Lua mode
//! ```
//!
//! **Why Shift+Tab.** `BackTab` is the only key in the Tab family a terminal delivers distinctly.
//! Ctrl+Tab is indistinguishable from Tab in the legacy encoding every terminal still falls back
//! to, so binding it would silently do nothing on a plain tty. It is configurable all the same,
//! because a key that collides with someone's terminal or window manager is worth being able to
//! move.

use oslo::Environment;
use rustyline::{
    Cmd, ConditionalEventHandler, Event, EventContext, KeyCode, KeyEvent, RepeatCount,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The language the next line will be read as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Shell,
    Lua,
}

impl Mode {
    pub fn other(self) -> Mode {
        match self {
            Mode::Shell => Mode::Lua,
            Mode::Lua => Mode::Shell,
        }
    }

    /// The word `$OSLO_MODE` carries, so a prompt function can see which one it is drawing for.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Shell => "sh",
            Mode::Lua => "lua",
        }
    }

    /// The default prompt for this mode, when the user has set none.
    pub fn fallback_prompt(self) -> &'static str {
        match self {
            Mode::Shell => "oslo$ ",
            Mode::Lua => "lua> ",
        }
    }
}

/// What the user typed, once the prefixes have been read off it.
pub enum Line<'a> {
    /// Run it in the mode the prompt is in.
    Normal(&'a str),
    /// A `!` or `=` prefix: run this one line in the other language, then carry on as before.
    OneOff { mode: Mode, text: &'a str },
}

/// Read a leading `!` or `=` off a line typed in `mode`.
///
/// Only at the very start of a *first* line, and only when something follows: a bare `=` is not a
/// Lua chunk, and `!` alone is how history expansion is spelled. A continuation line is never
/// re-examined, because by then the language is already decided.
pub fn classify(mode: Mode, line: &str) -> Line<'_> {
    let escape = match mode {
        Mode::Shell => '=',
        Mode::Lua => '!',
    };
    match line.strip_prefix(escape) {
        Some(rest) if !rest.trim().is_empty() => Line::OneOff {
            mode: mode.other(),
            text: rest,
        },
        _ => Line::Normal(line),
    }
}

/// The mode a session starts in.
///
/// Shell, unless `$OSLO_DEFAULT_MODE` says otherwise. A shell that came up in Lua would break
/// every muscle-memory command anyone has, so the default is not a close call — but a user who
/// lives in Lua should not have to press a key every time they open a terminal.
pub fn starting_mode(env: &Environment) -> Mode {
    match env.get_var("OSLO_DEFAULT_MODE").map(str::trim) {
        Some("lua") => Mode::Lua,
        _ => Mode::Shell,
    }
}

/// The key that toggles, from `$OSLO_TOGGLE_KEY`.
///
/// Spelled as a name rather than an escape sequence — `backtab`, `f2`, `ctrl-o` — because the
/// escape sequence a key produces depends on the terminal, which is the thing the user is trying
/// to work around by rebinding it.
pub fn toggle_key(env: &Environment) -> Option<KeyEvent> {
    let requested = env
        .get_var("OSLO_TOGGLE_KEY")
        .map(|k| k.trim().to_ascii_lowercase());
    let name = requested.as_deref().unwrap_or("backtab");
    Some(match name {
        // The default. Also accepted under the name people actually say out loud.
        "backtab" | "shift-tab" | "s-tab" => KeyEvent(KeyCode::BackTab, rustyline::Modifiers::NONE),
        "none" | "off" => return None,
        "f1" => KeyEvent(KeyCode::F(1), rustyline::Modifiers::NONE),
        "f2" => KeyEvent(KeyCode::F(2), rustyline::Modifiers::NONE),
        "f3" => KeyEvent(KeyCode::F(3), rustyline::Modifiers::NONE),
        "f4" => KeyEvent(KeyCode::F(4), rustyline::Modifiers::NONE),
        _ => match name.strip_prefix("ctrl-").and_then(one_char) {
            Some(c) => KeyEvent::ctrl(c),
            // An unreadable name falls back rather than leaving the shell with no toggle at all —
            // and says so, because a silently ignored setting is worse than a wrong one.
            None => {
                eprintln!("oslo: OSLO_TOGGLE_KEY: cannot read '{name}'; using shift-tab");
                KeyEvent(KeyCode::BackTab, rustyline::Modifiers::NONE)
            }
        },
    })
}

fn one_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// Records that the toggle key was pressed, for the loop to act on.
///
/// rustyline has no command for "hand control back to the caller", so the handler accepts the
/// line and leaves this flag behind. The loop sees the flag, switches mode, and re-opens the
/// editor with whatever was already typed — so pressing the key mid-command changes the language
/// without losing the line.
#[derive(Clone, Default)]
pub struct ToggleRequest(Arc<AtomicBool>);

impl ToggleRequest {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the key was pressed since this was last asked, clearing the flag.
    pub fn take(&self) -> bool {
        self.0.swap(false, Ordering::SeqCst)
    }
}

impl ConditionalEventHandler for ToggleRequest {
    fn handle(&self, _: &Event, _: RepeatCount, _: bool, _: &EventContext) -> Option<Cmd> {
        self.0.store(true, Ordering::SeqCst);
        Some(Cmd::AcceptLine)
    }
}

#[cfg(test)]
mod tests {
    use super::{Line, Mode, classify};

    #[test]
    fn a_prefix_runs_one_line_in_the_other_language() {
        assert!(matches!(
            classify(Mode::Shell, "=print(1)"),
            Line::OneOff {
                mode: Mode::Lua,
                text: "print(1)"
            }
        ));
        assert!(matches!(
            classify(Mode::Lua, "!ls -la"),
            Line::OneOff {
                mode: Mode::Shell,
                text: "ls -la"
            }
        ));
    }

    #[test]
    fn each_mode_only_answers_to_its_own_prefix() {
        // `!` in shell mode is history expansion, and `=` in Lua mode is not a statement — but
        // neither is this module's business, so both pass through untouched.
        assert!(matches!(classify(Mode::Shell, "!!"), Line::Normal("!!")));
        assert!(matches!(classify(Mode::Lua, "=x"), Line::Normal("=x")));
    }

    #[test]
    fn a_bare_prefix_is_not_an_escape() {
        assert!(matches!(classify(Mode::Shell, "="), Line::Normal("=")));
        assert!(matches!(classify(Mode::Lua, "!  "), Line::Normal("!  ")));
    }
}
