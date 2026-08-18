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
//! So there are three, and they fail in different places — see [`TOGGLE_KEYS`]. **Shift+Tab** is
//! the one to reach for. **Ctrl+Tab** is the one people expect, and like Ctrl+Enter it exists only
//! under the kitty protocol, because Ctrl-I *is* Tab otherwise. **Ctrl+Space** asks the terminal
//! for nothing: it is `NUL` in the legacy encoding and `CSI 32;5u` under the kitty protocol, and
//! both already decoded to `Key::Ctrl(' ')` before any of this was bound. Its own weakness is that
//! an input method may claim it first, which is why none of them is the only one.
//!
//! **Tab twice on an empty line** is the third and the one nothing can take away. Both of the
//! others fail silently on a machine where nothing looks wrong, so this one is always there rather
//! than waiting to be found; it costs Tab at an empty prompt, which otherwise lists every name on
//! `$PATH`. The three named keys are configurable through `oslo.keys`, because a key that collides
//! with someone's terminal or window manager is worth being able to move.
//!
//! **How a Lua block spanning several lines is read** — not a setting, and not the editor's job.
//! Enter always ends the *line*. [`super::read`] accumulates lines into a block and shows the
//! continuation prompt while it wants more, which is how oslo already read an unfinished `for` loop
//! in shell and how every REPL does it. A block that has already asked for more keeps asking until
//! an **empty line** ends it — Python's rule, and there for Python's reason: after
//! `local function f()` the parser is satisfied again at `end`, so running the moment it is
//! satisfied would mean no line after `end` could ever be typed. `oslo.lua.enter = "newline"` makes
//! Enter always start another line, so a block ends only on an empty one.

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
/// > History keeps the characters that **cannot begin a Lua expression** — *and every digit*.
/// > A space is how you say you meant Lua.
///
/// `!!`, `!$`, `!^`, `!*` and `!?str?` stay history because Lua has no `!`, `$` or `?` at all and
/// no expression opens with `^` or `*`. Those need no exception; the digits do.
///
/// **A digit is history, and that is a choice rather than a deduction.** `!5` could be either —
/// event five, or the Lua expression `5` — so no rule about what Lua can parse decides it. What
/// decides it is that `!5` and `!-2` are *forty years of muscle memory* and `!5` as a Lua literal
/// is a thing nobody types on purpose. So a digit immediately after the `!` is an event number, and
/// so is a `-` with a digit behind it.
///
/// **The space is the escape**, and it costs one keystroke in the rarer case:
///
/// ```text
/// !5          → history event 5
/// ! 5 + 5     → Lua, 10
/// !-2         → history, two events back
/// ! -x        → Lua
/// !print(1)   → Lua, because a letter was never ambiguous
/// ```
///
/// It falls out rather than being special-cased: a leading space is not one of the characters
/// history claims, so ` 5 + 5` was already reaching Lua — and Lua does not care about the space.
///
/// What this still costs is `!name`, "the last line starting with *name*". That one cannot come
/// back: `!print(1)` has exactly its shape, and running one Lua line is the whole reason the prefix
/// exists. The history finder searches as you type and shows what it found before running it, which
/// is the better answer to the same question.
///
pub fn classify(mode: Mode, line: &str) -> Line<'_> {
    // A Lua line is a Lua line. Nothing is read off it.
    if mode == Mode::Lua {
        return Line::Normal(line);
    }
    let Some(rest) = line.strip_prefix(LUA_PREFIX) else {
        return Line::Normal(line);
    };
    if rest.trim().is_empty() || opens_a_history_reference(rest) {
        return Line::Normal(line);
    }
    Line::OneOff {
        mode: Mode::Lua,
        text: rest,
    }
}

/// Whether what follows a `!` makes it a history reference rather than one line of Lua.
///
/// **The characters no Lua expression can start with, plus the digits.** `!`, `$` and `?` are not
/// Lua syntax anywhere and `^` and `*` are binary operators, so those five need no argument. A
/// digit does: `!5` reads as Lua just as well as it reads as event five, and it is history because
/// that is what people type it for. `! 5` is the Lua one — see [`classify`].
fn opens_a_history_reference(after: &str) -> bool {
    let mut chars = after.chars();
    match chars.next() {
        Some('!' | '$' | '^' | '*' | '?') => true,
        Some(digit) if digit.is_ascii_digit() => true,
        // `!-2` is two events back. `!-x` is Lua, so the digit has to be there.
        Some('-') => chars.next().is_some_and(|c| c.is_ascii_digit()),
        _ => false,
    }
}

/// The character that runs one line as Lua from a shell prompt.
///
/// **A constant, for the reason `$OSLO_TOGGLE_KEY` is gone**: the carve-out [`classify`] documents
/// is written against this character specifically — which `!` forms history keeps depends on which
/// ones Lua could have meant — so a prefix that moved would take the history rules with it, and the
/// two would be free to disagree.
const LUA_PREFIX: char = '!';

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
pub const TOGGLE_KEYS: &[&str] = &["shift-tab", "ctrl-space", "ctrl-tab"];
