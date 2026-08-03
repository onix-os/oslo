//! `bind`: letting shell code — and the plugins written in it — own a keystroke.
//!
//! oslo already lets a *config* bind a key, through `oslo.keys` and a Lua handler. That is not
//! enough for the ecosystem. Every shell integration worth having is distributed as shell source
//! you `eval`, and each one expects to claim keys the way bash's readline does:
//!
//! ```bash
//! bind -x '"\C-r": __atuin_history'      # atuin's history search
//! bind -x '"\C-d": __hexe_ctrl_d'        # hexe's exit intent
//! ```
//!
//! # The contract a plugin is written against
//!
//! `bind -x` runs a shell command **with the line in hand**. Before the command runs, the shell
//! puts the buffer in `$READLINE_LINE` and the cursor's byte offset in `$READLINE_POINT`; when it
//! returns, the shell reads both back and the line becomes whatever the command left there. That
//! is how a plugin replaces what you typed — atuin's Ctrl-R opens a full-screen picker and hands
//! the chosen command back through `READLINE_LINE`.
//!
//! Two consequences fall out of that, and both are the point rather than a limitation:
//!
//! * the command runs with the terminal **out of raw mode**, so it may draw a full-screen UI, read
//!   from the tty, or run anything else it likes. A handler that could only rewrite a string could
//!   not host a picker, which is what the interesting bindings all are;
//! * the line is *not* submitted. Control returns to the prompt with the buffer the command left.
//!   A plugin that wants to submit sets `READLINE_LINE` and the user presses Enter, exactly as
//!   under bash.
//!
//! # Why a registry rather than binding at startup
//!
//! `bind` is ordinary shell code and runs whenever it is reached — from the rc file, from an
//! `eval` typed at the prompt, from a function. The editor is built once. So bindings live here
//! and carry a generation counter; the read loop re-applies them when it changes. Binding only at
//! startup meant `eval "$(atuin init bash)"` typed into a running shell did nothing until restart.

use rustyline::{KeyCode, KeyEvent, Modifiers};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

/// What a bound key does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bound {
    /// `bind -x`: run this shell command, with the line in `$READLINE_LINE`.
    Command(String),
    /// `bind '"\C-r": "\C-x\C-_A1\a"'`: a **macro** — the key expands into another key
    /// sequence, which is then dispatched as if it had been typed.
    ///
    /// This is not a curiosity. atuin's whole keymap is built out of macros: Ctrl-R expands to a
    /// sequence of private key codes, each of which is a `bind -x` command, and running them in
    /// order is what opens the search. Without macros its `bind` lines are recorded and nothing
    /// ever happens, which is exactly how it behaved before.
    Macro {
        keys: Vec<KeyEvent>,
        /// The sequence as written, so a listing can echo what was asked for rather than a
        /// re-rendering of it. `bind -p` output is read by init scripts and by people.
        text: String,
    },
    /// `bind '"\C-x": backward-word'`: a readline *function* name. Recorded so a listing can
    /// report it; oslo maps the names it has an equivalent for and leaves the rest alone rather
    /// than binding a key to nothing.
    Function(String),
}

/// Which of readline's keymaps a binding belongs to.
///
/// This is not bookkeeping. atuin binds `/` and `k` in the **vi-command** keymap, where they are
/// motion keys and mean nothing while you are typing. Applying them regardless put a command on
/// the `/` in every path: typing `ls /tmp` opened the history search and the shell stopped
/// responding. A keymap oslo cannot express is a binding oslo must not install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keymap {
    /// The default keymap, and the one oslo's editor is in unless vi mode is on.
    Emacs,
    /// vi insert mode — where you type, so a binding here behaves like an emacs one.
    ViInsert,
    /// vi command mode. oslo's editor has no separate keymap for it, so bindings here are
    /// recorded and **not** applied, rather than leaking into the mode where you type.
    ViCommand,
}

impl Keymap {
    /// readline's names for the keymaps, as `bind -m` spells them.
    pub fn parse(name: &str) -> Option<Keymap> {
        Some(match name {
            "emacs" | "emacs-standard" | "emacs-meta" | "emacs-ctlx" => Keymap::Emacs,
            "vi-insert" | "vi" => Keymap::ViInsert,
            "vi-command" | "vi-move" => Keymap::ViCommand,
            _ => return None,
        })
    }

    /// Whether a binding in this keymap should be applied to the editor as it is now.
    ///
    /// oslo's editor is in one keymap at a time and rustyline has no separate vi-command binding
    /// table, so the honest answer for `ViCommand` is always no.
    pub fn is_active(self) -> bool {
        match self {
            Keymap::Emacs => !crate::interactive::vi::enabled(),
            Keymap::ViInsert => crate::interactive::vi::enabled(),
            Keymap::ViCommand => false,
        }
    }
}

/// One binding, as `bind` recorded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The keymap it was bound in. `bind` with no `-m` records the active one.
    pub keymap: Keymap,
    /// The key sequence, already parsed. More than one event for `"\C-x\C-r"`.
    pub keys: Vec<KeyEvent>,
    /// The spec exactly as written, so `bind -r` can match it and a listing can echo it.
    pub spec: String,
    pub bound: Bound,
}

fn registry() -> &'static Mutex<Vec<Entry>> {
    static REGISTRY: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

/// Bumped on every change, so the read loop knows when to re-apply.
static GENERATION: AtomicUsize = AtomicUsize::new(0);

pub fn generation() -> usize {
    GENERATION.load(Ordering::SeqCst)
}

/// Record a binding, replacing any earlier one for the same spec — as rebinding a key does.
pub fn bind(spec: &str, keymap: Keymap, bound: Bound) -> Result<(), String> {
    let keys = parse_sequence(spec).ok_or_else(|| format!("{spec}: cannot parse key sequence"))?;
    let mut entries = registry().lock().map_err(|_| "bind: lock poisoned")?;
    // Per keymap: `/` may be a search in vi-command and an ordinary character in vi-insert, and
    // one replacing the other is how binding a motion key broke typing.
    entries.retain(|e| !(e.keys == keys && e.keymap == keymap));
    entries.push(Entry {
        keymap,
        keys,
        spec: spec.to_string(),
        bound,
    });
    GENERATION.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

/// `bind -r`: forget a binding. True when there was one.
pub fn unbind(spec: &str, keymap: Keymap) -> bool {
    let Some(keys) = parse_sequence(spec) else {
        return false;
    };
    let Ok(mut entries) = registry().lock() else {
        return false;
    };
    let before = entries.len();
    entries.retain(|e| !(e.keys == keys && e.keymap == keymap));
    if entries.len() != before {
        GENERATION.fetch_add(1, Ordering::SeqCst);
        return true;
    }
    false
}

/// How many macros deep an expansion may go before it is called a loop.
///
/// A macro may expand into keys that are themselves macros — atuin's is two levels — so following
/// them is a walk, and `bind '"\C-a": "\C-a"'` is a walk with no end. Eight is far past anything
/// real and far short of a stack problem.
const MAX_MACRO_DEPTH: usize = 8;

/// The commands a key sequence stands for, following macros to the `bind -x` bindings underneath.
///
/// The sequence is segmented by **longest match first**, because the bound sequences are not
/// prefix-free: atuin binds `\C-x\C-_A1\a` and `\C-x\C-_A10\a`, and matching the shorter one
/// first would run the wrong widget and leave `0\a` behind as stray keys.
///
/// Keys that match nothing are dropped rather than reported. A macro is a *key sequence*, and a
/// key nothing is bound to is an ordinary key press with nothing to do — under bash it would
/// insert itself, which is not something a binding should do on the user's behalf.
pub fn expand(keys: &[KeyEvent]) -> Vec<String> {
    let mut commands = Vec::new();
    walk(keys, 0, &mut commands);
    commands
}

fn walk(keys: &[KeyEvent], depth: usize, out: &mut Vec<String>) {
    if depth >= MAX_MACRO_DEPTH {
        eprintln!("oslo: bind: macro expands into itself; stopping");
        return;
    }
    // Only the keymap in force: a macro bound in one keymap expands through the bindings of that
    // keymap, not through every binding the shell has ever been given.
    let table: Vec<Entry> = entries()
        .into_iter()
        .filter(|entry| entry.keymap.is_active())
        .collect();
    let mut at = 0;
    while at < keys.len() {
        let matched = table
            .iter()
            .filter(|entry| !entry.keys.is_empty() && keys[at..].starts_with(&entry.keys))
            .max_by_key(|entry| entry.keys.len());
        let Some(entry) = matched else {
            at += 1;
            continue;
        };
        match &entry.bound {
            Bound::Command(command) => out.push(command.clone()),
            Bound::Macro { keys, .. } => walk(keys, depth + 1, out),
            // A readline function is bound in the editor, not run as a command, so a macro that
            // reaches one has nothing to contribute here.
            Bound::Function(_) => {}
        }
        at += entry.keys.len();
    }
}

/// Every binding, for applying to the editor or for a listing.
pub fn entries() -> Vec<Entry> {
    registry()
        .lock()
        .map(|e| e.clone())
        .unwrap_or_else(|_| Vec::new())
}

/// Forget every binding — used when a config is reloaded.
pub fn clear() {
    if let Ok(mut entries) = registry().lock() {
        entries.clear();
    }
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// A key was pressed and its command has not run yet.
///
/// The editor cannot run shell code: it holds the terminal in raw mode and the handler is called
/// from inside its own read loop. So the handler records what to do and ends the line, and the
/// read loop — which is outside the editor, with the terminal restored — runs it and re-enters.
/// This is the same shape the language toggle uses, for the same reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// In order. A macro expands to more than one — atuin's Ctrl-R is five commands that together
    /// encode which widget to run.
    pub commands: Vec<String>,
    /// The buffer as it stood when the key was pressed.
    pub line: String,
    /// The cursor, as a byte offset — what `$READLINE_POINT` counts, matching bash.
    pub point: usize,
}

fn pending() -> &'static Mutex<Option<Request>> {
    static PENDING: OnceLock<Mutex<Option<Request>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(None))
}

/// Record that a bound key was pressed. The read loop picks this up.
pub fn request(commands: Vec<String>, line: &str, point: usize) {
    if commands.is_empty() {
        return;
    }
    if let Ok(mut slot) = pending().lock() {
        *slot = Some(Request {
            commands,
            line: line.to_string(),
            point,
        });
    }
}

/// Take the pending request, if any.
pub fn take_request() -> Option<Request> {
    pending().lock().ok().and_then(|mut slot| slot.take())
}

/// Parse readline's key-sequence syntax into the events it stands for.
///
/// The forms that appear in the wild, which is what this has to read rather than the whole of
/// readline's grammar:
///
/// ```text
/// "\C-r"        Ctrl-R
/// "\e[A"        the escape sequence an Up arrow sends
/// "\M-a"        Alt-A
/// "\C-x\C-r"    two keys in sequence
/// "\t" "\n"     the named controls
/// ```
///
/// Surrounding double quotes are optional, because both `bind -x '"\C-r": f'` and
/// `bind -x '\C-r: f'` are written.
pub fn parse_sequence(spec: &str) -> Option<Vec<KeyEvent>> {
    let spec = spec.trim();
    let spec = spec
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(spec);
    if spec.is_empty() {
        return None;
    }

    let mut events = Vec::new();
    let mut chars = spec.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            events.push(KeyEvent(KeyCode::Char(c), Modifiers::NONE));
            continue;
        }
        match chars.next()? {
            // `\C-x` and `\M-x`. The `-` is part of the syntax, not the key.
            'C' | 'c' => {
                if chars.peek() == Some(&'-') {
                    chars.next();
                }
                events.push(control(chars.next()?)?);
            }
            'M' | 'm' => {
                if chars.peek() == Some(&'-') {
                    chars.next();
                }
                // `\M-\C-x` is Alt with Ctrl. Rare, but it is what the syntax means.
                if chars.peek() == Some(&'\\') {
                    chars.next();
                    let next = chars.next()?;
                    if next == 'C' || next == 'c' {
                        if chars.peek() == Some(&'-') {
                            chars.next();
                        }
                        let event = control(chars.next()?)?;
                        events.push(KeyEvent(event.0, event.1 | Modifiers::ALT));
                        continue;
                    }
                    return None;
                }
                events.push(KeyEvent::alt(chars.next()?));
            }
            // An escape *sequence* — `\e[A` — is what a terminal sends for a named key, so it is
            // read as that key rather than as Esc followed by two characters. Anything else after
            // `\e` is Alt with that character, which is how readline writes it too.
            'e' | 'E' => match escape_sequence(&mut chars) {
                Some(event) => events.push(event),
                None => events.push(KeyEvent(KeyCode::Esc, Modifiers::NONE)),
            },
            't' => events.push(KeyEvent(KeyCode::Tab, Modifiers::NONE)),
            'n' => events.push(KeyEvent(KeyCode::Enter, Modifiers::NONE)),
            'r' => events.push(KeyEvent(KeyCode::Enter, Modifiers::NONE)),
            'd' => events.push(KeyEvent(KeyCode::Delete, Modifiers::NONE)),
            '\\' => events.push(KeyEvent(KeyCode::Char('\\'), Modifiers::NONE)),
            '"' => events.push(KeyEvent(KeyCode::Char('"'), Modifiers::NONE)),
            '\'' => events.push(KeyEvent(KeyCode::Char('\''), Modifiers::NONE)),
            other => events.push(KeyEvent(KeyCode::Char(other), Modifiers::NONE)),
        }
    }
    (!events.is_empty()).then_some(events)
}

/// `\C-x`. Ctrl-I, Ctrl-M and Ctrl-[ are the keys the terminal cannot tell apart from Tab, Enter
/// and Esc, so they are those keys — binding them as `ctrl-i` would never fire.
fn control(c: char) -> Option<KeyEvent> {
    match c.to_ascii_lowercase() {
        'i' => Some(KeyEvent(KeyCode::Tab, Modifiers::NONE)),
        'm' => Some(KeyEvent(KeyCode::Enter, Modifiers::NONE)),
        '[' => Some(KeyEvent(KeyCode::Esc, Modifiers::NONE)),
        '?' => Some(KeyEvent(KeyCode::Backspace, Modifiers::NONE)),
        other => Some(KeyEvent::ctrl(other)),
    }
}

/// The rest of a `\e...` sequence, when it spells a key a terminal actually sends.
fn escape_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<KeyEvent> {
    let bracket = *chars.peek()?;
    if bracket != '[' && bracket != 'O' {
        // `\ea` is Alt-A, which is what readline means by it.
        let c = chars.next()?;
        return Some(KeyEvent::alt(c));
    }
    chars.next();
    let code = chars.next()?;
    let key = match code {
        'A' => KeyCode::Up,
        'B' => KeyCode::Down,
        'C' => KeyCode::Right,
        'D' => KeyCode::Left,
        'H' => KeyCode::Home,
        'F' => KeyCode::End,
        'P' => KeyCode::F(1),
        'Q' => KeyCode::F(2),
        'R' => KeyCode::F(3),
        'S' => KeyCode::F(4),
        // `\e[3~` is Delete, and the family it belongs to is written the same way.
        digit if digit.is_ascii_digit() => {
            let mut number = digit.to_string();
            while let Some(c) = chars.peek() {
                if c.is_ascii_digit() {
                    number.push(*c);
                    chars.next();
                } else {
                    break;
                }
            }
            if chars.peek() == Some(&'~') {
                chars.next();
            }
            match number.as_str() {
                "1" | "7" => KeyCode::Home,
                "2" => KeyCode::Insert,
                "3" => KeyCode::Delete,
                "4" | "8" => KeyCode::End,
                "5" => KeyCode::PageUp,
                "6" => KeyCode::PageDown,
                "15" => KeyCode::F(5),
                "17" => KeyCode::F(6),
                "18" => KeyCode::F(7),
                "19" => KeyCode::F(8),
                "20" => KeyCode::F(9),
                "21" => KeyCode::F(10),
                "23" => KeyCode::F(11),
                "24" => KeyCode::F(12),
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(KeyEvent(key, Modifiers::NONE))
}

#[cfg(test)]
#[path = "readline/tests.rs"]
mod tests;
