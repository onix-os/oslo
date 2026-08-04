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

    // There was a `fallback_prompt` here, giving Lua its own `lua> `. It is gone because both
    // languages now share one prompt with the language as a segment — a separate prompt meant
    // switching language also threw away the branch, the vi mode and the directory.
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

/// The key that switches the prompt between shell and Lua.
///
/// **A constant, not a setting.** There was an `$OSLO_TOGGLE_KEY`; it is gone, because the key
/// bindings already live in one place and a variable that could also set one was a second place
/// for them to disagree from. The config does both jobs:
///
/// ```lua
/// oslo.keys["f2"] = "toggle-language"   -- another key as well
/// oslo.keys["shift-tab"] = "none"       -- and this one turns the default off
/// ```
pub const TOGGLE_KEY: &str = "shift-tab";
