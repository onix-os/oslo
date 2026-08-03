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
    /// `bind '"\C-x": text'`: a readline function name or a macro. Recorded so a listing can
    /// report it; oslo maps the function names it knows onto its own actions and says so about
    /// the rest, rather than silently doing nothing.
    Function(String),
}

/// One binding, as `bind` recorded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
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
pub fn bind(spec: &str, bound: Bound) -> Result<(), String> {
    let keys = parse_sequence(spec).ok_or_else(|| format!("{spec}: cannot parse key sequence"))?;
    let mut entries = registry().lock().map_err(|_| "bind: lock poisoned")?;
    entries.retain(|e| e.keys != keys);
    entries.push(Entry {
        keys,
        spec: spec.to_string(),
        bound,
    });
    GENERATION.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

/// `bind -r`: forget a binding. True when there was one.
pub fn unbind(spec: &str) -> bool {
    let Some(keys) = parse_sequence(spec) else {
        return false;
    };
    let Ok(mut entries) = registry().lock() else {
        return false;
    };
    let before = entries.len();
    entries.retain(|e| e.keys != keys);
    if entries.len() != before {
        GENERATION.fetch_add(1, Ordering::SeqCst);
        return true;
    }
    false
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
    pub command: String,
    /// The buffer as it stood when the key was pressed.
    pub line: String,
    /// The cursor, as a byte offset — what `$READLINE_POINT` counts, matching bash.
    pub point: usize,
}

fn pending() -> &'static Mutex<Option<Request>> {
    static PENDING: OnceLock<Mutex<Option<Request>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(None))
}

/// Record that a `bind -x` key was pressed. The read loop picks this up.
pub fn request(command: &str, line: &str, point: usize) {
    if let Ok(mut slot) = pending().lock() {
        *slot = Some(Request {
            command: command.to_string(),
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
