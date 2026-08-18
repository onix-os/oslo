//! Which language the prompt is reading, and how you change it.
//!
//! oslo's prompt reads *one* language at a time. Shift+Tab switches, and the two are never mixed
//! on a line — a line is shell or it is Lua, never a guess. Mixing them would mean deciding what
//! `print(1)` is by looking at it, and a shell whose meaning depends on what happens to be
//! installed is a shell you cannot write scripts against.
//!
//! **One prefix, and it goes one way**: `!` runs a single line as Lua from a shell prompt.
//!
//! ```text
//! oslo$ !print(1 + 1)      -- one Lua line, without leaving the shell
//! lua>  print(1 + 1)       -- the Lua prompt is a REPL; there is no prefix
//! ```
//!
//! **Why `!` and not `=`.** `=` is a character a shell already spends: `FOO=bar` is an assignment
//! in every shell there is, `=cmd` is a real expansion in zsh, and oslo's own `=grep` answers
//! where a program lives — a leading `=` was three things at once, and the prompt had to pick.
//! `!` is the shell's own reach-back character, and it only has to share with history expansion,
//! which is a smaller and much clearer split. See [`classify`].
//!
//! **And it is a setting**, because the choice is a taste and the rest of the prompt does not
//! depend on it — see [`lua_prefix`]. Most of the free characters are freer than `!`: `,` and `+`
//! are claimed by neither bash nor oslo, so choosing one of those retires the history carve-out
//! [`classify`] documents and leaves `!!`, `!name` and `!5` doing what they do in bash.
//!
//! **Why it goes one way.** A shell prompt is where you run programs, and reaching for Lua for one
//! quick thing is exactly what an escape is for. A Lua prompt is not the mirror of that: it is a
//! REPL, `oslo.run{"ls", "-la"}` already runs a program from it, and a second syntax for that job
//! is a second thing to know and one more way for a line to mean something you did not type. So
//! the Lua side has no prefixes at all — every line there is Lua, and Shift+Tab is how you leave.
//!
//! **Why two keys switch it.** `BackTab` is the only key in the Tab family a terminal delivers
//! distinctly — Ctrl+Tab is indistinguishable from Tab in the legacy encoding, so binding it would
//! silently do nothing on a plain tty. But "delivers distinctly" still asks something of the
//! terminal, and not every one answers: Alacritty without the kitty keyboard protocol reports no
//! modifier for Shift+Tab, which left no way to change language at all.
//!
//! So there are two, and they fail in different places — see [`TOGGLE_KEYS`]. **Shift+Tab** is the
//! one to reach for. **Ctrl+Space** asks the terminal for nothing: it is `NUL` in the legacy
//! encoding and `CSI 32;5u` under the kitty protocol, and both already decoded to `Key::Ctrl(' ')`
//! before either was bound to anything. Its own weakness is that an input method may claim it
//! first, which is why neither is the only one.
//!
//! **Tab twice on an empty line** is the third and the one nothing can take away — see
//! [`double_tab`]. Both of the others fail silently on a machine where nothing looks wrong, so the
//! fallback is on by default rather than waiting to be found. All three are configurable, because a
//! key that collides with someone's terminal or window manager is worth being able to move.

use oslo_shell::Environment;
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

/// What the user typed, once the prefix has been read off it.
pub enum Line<'a> {
    /// Run it in the mode the prompt is in.
    Normal(&'a str),
    /// A `!` prefix at a shell prompt: run this one line as Lua, then carry on as before.
    OneOff { mode: Mode, text: &'a str },
}

/// Read the Lua prefix off a line typed at a **shell** prompt.
///
/// Only at the very start of a *first* line, and only when something follows: a bare prefix is not
/// a command. A continuation line is never re-examined, because by then the language is decided.
///
/// A Lua line is never examined at all — see the module docs.
///
/// # The one thing `!` has to share
///
/// This part is the price of `!` specifically, and it is charged only when `!` is the prefix. In
/// shell mode `!` also opens a **history reference**, and `!!` is the most-typed two characters in
/// any shell. The line between the two is drawn where it can be drawn without guessing:
///
/// > History keeps the characters that **cannot begin a Lua expression**. Everything that can, is
/// > Lua.
///
/// So `!!`, `!$`, `!^`, `!*` and `!?str?` stay history — Lua has no `!`, `$` or `?` at all, and no
/// expression opens with `^` or `*`. And `!5 + 5`, `!-x` and `!print(1)` are Lua, because every
/// one of those is something a person might reasonably type and mean.
///
/// What that costs is bash's `!5` and `!-2`, the numbered events, and `!name` — "the last line
/// starting with *name*". All three are ambiguous by construction, and all three have the same
/// better answer in this shell: the history finder searches as you type and shows you what it
/// found *before* it runs. A numbered event you have to count to is the form nobody misses.
///
/// Set [`lua_prefix`] to a character history does not want and the whole section above stops
/// applying: `,print(1)` is Lua, and every `!` form is bash's again.
pub fn classify(mode: Mode, line: &str, prefix: Option<char>) -> Line<'_> {
    // A Lua line is a Lua line. Nothing is read off it.
    if mode == Mode::Lua {
        return Line::Normal(line);
    }
    let Some(prefix) = prefix else {
        return Line::Normal(line);
    };
    let Some(rest) = line.strip_prefix(prefix) else {
        return Line::Normal(line);
    };
    if rest.trim().is_empty() || (prefix == '!' && opens_a_history_reference(rest)) {
        return Line::Normal(line);
    }
    Line::OneOff {
        mode: Mode::Lua,
        text: rest,
    }
}

/// Whether what follows a `!` makes it a history reference rather than one line of Lua.
///
/// **Only the characters no Lua expression can start with.** `!`, `$` and `?` are not Lua syntax
/// anywhere; `^` and `*` are binary operators, so a line cannot open with one. A digit or a `-`
/// *can* open a Lua expression — `5 + 5`, `-x` — so those are Lua, which costs bash's numbered
/// events and buys a rule with no guessing in it.
fn opens_a_history_reference(after: &str) -> bool {
    matches!(after.chars().next(), Some('!' | '$' | '^' | '*' | '?'))
}

/// The character that runs one line as Lua from a shell prompt.
///
/// `!` unless `$OSLO_LUA_PREFIX` says otherwise, and `None` — no escape at all, every shell line
/// is shell — when it is set to an empty string or to `none`.
///
/// **One character, and it must be punctuation.** A letter would make `x = 1` unreachable the
/// moment someone picked `x`, and a multi-character prefix is a small language of its own to parse
/// against history and quoting both. Anything else is ignored rather than half-honoured, because a
/// prompt that silently reads a different language than the one configured is the failure this
/// whole module exists to avoid.
pub fn lua_prefix(env: &Environment) -> Option<char> {
    let Some(setting) = env.get_var("OSLO_LUA_PREFIX") else {
        return Some('!');
    };
    let setting = setting.trim();
    if setting.is_empty() || setting == "none" {
        return None;
    }
    let mut chars = setting.chars();
    match (chars.next(), chars.next()) {
        (Some(one), None) if one.is_ascii_punctuation() => Some(one),
        _ => Some('!'),
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

/// What Enter does at a **Lua** prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enter {
    /// Send when the block is finished, add a line when it is not. The default.
    Smart,
    /// Always add a line. Ctrl+Enter or Alt+Enter sends.
    Newline,
}

/// Read `$OSLO_LUA_ENTER`.
///
/// **`smart` is the default, and the reason is that the alternative can lock you out.** With
/// `newline`, the only way to send is Ctrl+Enter or Alt+Enter — and Ctrl+Enter does not exist on a
/// terminal without the kitty keyboard protocol, because in the legacy encoding Ctrl+Enter *is*
/// Enter (Ctrl-M is CR). Alt+Enter is decoded in both encodings, so `newline` always has one way
/// out; it is still not a default worth choosing for someone who did not ask.
///
/// `smart` is what a Lua REPL usually does and what oslo has always done: a finished block runs, an
/// unfinished one asks for more. `newline` is for writing a function at the prompt, where every
/// Enter meaning "run this" is an interruption.
pub fn enter_key(env: &Environment) -> Enter {
    match env
        .get_var("OSLO_LUA_ENTER")
        .map(|value| value.trim().to_string())
    {
        Some(value) if value == "newline" => Enter::Newline,
        _ => Enter::Smart,
    }
}

/// Whether Tab twice on an empty line switches language. `$OSLO_DOUBLE_TAB`, on unless `off`.
///
/// **The third way in, and the one that cannot fail.** Shift+Tab needs the terminal to report a
/// modifier, and Alacritty without the kitty keyboard protocol does not; Ctrl+Space needs the
/// terminal to *see* it, and ibus or fcitx claims it as the input-method switch first. Both of
/// those fail silently, on a machine where nothing looks wrong. A plain Tab is a plain Tab
/// everywhere.
///
/// It costs Tab at an empty prompt, which otherwise lists every name on `$PATH`.
pub fn double_tab(env: &Environment) -> bool {
    !matches!(
        env.get_var("OSLO_DOUBLE_TAB")
            .map(|v| v.trim().to_string())
            .as_deref(),
        Some("off" | "0" | "no" | "false")
    )
}

/// The keys that switch the prompt between shell and Lua.
///
/// **Two, because one of them asks something of the terminal.** Shift+Tab has to be *reported* as
/// Shift+Tab, and a terminal that sends a bare Tab for it leaves no way to change language at all —
/// which is what happened in Alacritty. Ctrl+Space asks for nothing: it is `NUL` in the legacy
/// encoding and `CSI 32;5u` under the kitty protocol, and oslo already decoded both to
/// `Key::Ctrl(' ')` before either was bound to anything.
///
/// Ctrl+Space has one cost worth knowing: it is the default input-method switch in ibus and fcitx,
/// and an IME grabs it before the terminal sees it. That is a reason to have two keys, not a reason
/// to prefer either.
///
/// **Constants, not a setting.** There was an `$OSLO_TOGGLE_KEY`; it is gone, because the key
/// bindings already live in one place and a variable that could also set one was a second place
/// for them to disagree from. The config does both jobs:
///
/// ```lua
/// oslo.keys["f2"] = "toggle-language"   -- another key as well
/// oslo.keys["shift-tab"] = "none"       -- and this one turns a default off
/// oslo.keys["ctrl-space"] = "none"      -- as does this
/// ```
pub const TOGGLE_KEYS: &[&str] = &["shift-tab", "ctrl-space"];
