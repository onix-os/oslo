//! `bind` — the builtin every shell integration reaches for.
//!
//! The form that matters is `bind -x`, which gives a shell command a keystroke:
//!
//! ```bash
//! bind -x '"\C-r": __atuin_history'
//! ```
//!
//! `crate::interactive::readline` owns what that *means* — the registry, the key syntax, and the
//! contract about `$READLINE_LINE`. This file is only the command-line surface: which options
//! exist, how a `"key": action` spec is split, and what a listing prints.
//!
//! # What is deliberately not here
//!
//! `bind '"\C-x": backward-word'` — binding to a **readline function name** rather than a command
//! — is accepted and recorded, but oslo does not implement readline's function set and says so
//! once per unknown name rather than pretending. The alternative was to accept it silently, which
//! is how a user spends an evening working out that their keybinding never existed. `oslo.keys` is
//! the supported way to bind an editing action, and it names actions oslo actually has.

use crate::env::Environment;
use crate::error::Result;
use crate::interactive::readline::{self, Bound, Keymap};

pub fn builtin_bind(env: &mut Environment, args: &[String]) -> Result<i32> {
    let _ = env;
    let mut operands = args[1..].iter().map(String::as_str).peekable();
    let mut run_command = false;
    let mut status = 0;
    // `bind` with no `-m` binds in the keymap that is in force, which is what an init script
    // means when it omits it.
    let mut keymap = if crate::interactive::vi::enabled() {
        Keymap::ViInsert
    } else {
        Keymap::Emacs
    };

    while let Some(arg) = operands.next() {
        match arg {
            // `-x` applies to the spec that follows it, and is written both as `bind -x 'spec'`
            // and as `bind -x -x 'a' 'b'` by generated init scripts.
            "-x" => run_command = true,
            "-r" | "-u" => {
                let Some(spec) = operands.next() else {
                    eprintln!("oslo: bind: {arg}: option requires an argument");
                    status = 1;
                    continue;
                };
                if !readline::unbind(spec, keymap) {
                    eprintln!("oslo: bind: {spec}: cannot unbind");
                    status = 1;
                }
            }
            // The listings. `-X` is the one a plugin actually reads back: it asks for the `-x`
            // bindings in re-inputtable form, which is how a second `init` avoids double-binding.
            "-X" => list(true),
            "-p" | "-P" | "-s" | "-S" => list(false),
            "-l" | "-v" | "-V" => {}
            // The keymap the following specs belong to. Honouring this is not optional: atuin
            // binds `/` and `k` in vi-command, where they are motions, and installing those
            // globally put a command on the `/` of every path.
            "-m" => match operands.next() {
                Some(name) => match Keymap::parse(name) {
                    Some(parsed) => keymap = parsed,
                    None => {
                        eprintln!("oslo: bind: {name}: unknown keymap");
                        status = 1;
                    }
                },
                None => {
                    eprintln!("oslo: bind: -m: option requires an argument");
                    status = 1;
                }
            },
            // A readline variable or function file. oslo has its own settings and does not
            // implement readline's, so these are accepted and ignored rather than reported: init
            // scripts set them unconditionally and a diagnostic per line would bury the ones that
            // matter.
            "-f" | "-q" => {
                operands.next();
            }
            "--" => {}
            other if other.starts_with('-') && other.len() > 1 => {
                eprintln!("oslo: bind: {other}: invalid option");
                status = 2;
            }
            spec => {
                if !apply(spec, keymap, run_command) {
                    status = 1;
                }
                // `-x` binds the one spec that follows it, as bash's does.
                run_command = false;
            }
        }
    }
    Ok(status)
}

/// Record one `"key": action` spec. False on anything that could not be read.
fn apply(spec: &str, keymap: Keymap, run_command: bool) -> bool {
    // `set editing-mode vi` and friends arrive here as a bare operand. oslo has `oslo.vi` for
    // that and readline's variables are not implemented, so they are dropped quietly — see the
    // module note on why this one is silent.
    if spec.starts_with("set ") {
        return true;
    }

    let Some((key, action)) = split_spec(spec) else {
        eprintln!("oslo: bind: {spec}: missing colon separator");
        return false;
    };
    let bound = if run_command {
        Bound::Command(action.to_string())
    } else if let Some(keys) = macro_target(action) {
        // A *quoted* action is a macro: the key expands into that key sequence. `bind '"\C-a":
        // "\C-x\C-r"'` and `bind '"\C-a": beginning-of-line'` differ by nothing but the quotes,
        // which is readline's own rule and the one atuin's keymap is written against.
        Bound::Macro {
            keys,
            text: action.trim_matches('"').to_string(),
        }
    } else {
        Bound::Function(action.to_string())
    };
    match readline::bind(&key, keymap, bound) {
        Ok(()) => true,
        Err(message) => {
            eprintln!("oslo: bind: {message}");
            false
        }
    }
}

/// Split `"\C-r": command` at the colon that separates key from action.
///
/// The colon has to be the one *outside* the quoted key, or `bind -x '"\C-x:": f'` — a key that
/// is itself a colon — splits in the wrong place. So a quoted key is scanned to its closing quote
/// first, and only then is a colon looked for.
fn split_spec(spec: &str) -> Option<(String, &str)> {
    let spec = spec.trim();
    let (key_end, key) = if let Some(rest) = spec.strip_prefix('"') {
        let close = rest.find('"')?;
        (close + 2, format!("\"{}\"", &rest[..close]))
    } else {
        let colon = spec.find(':')?;
        (colon, spec[..colon].trim().to_string())
    };
    let after = spec[key_end..].trim_start();
    let action = after.strip_prefix(':')?.trim();
    if action.is_empty() {
        return None;
    }
    Some((key, action))
}

/// The key sequence a quoted action expands to, or `None` when the action is not quoted.
///
/// An empty macro — `bind '"\C-a": ""'` — is a real thing: atuin uses it to *neutralise* a key it
/// no longer wants, and it must record as a macro that does nothing rather than fall through to
/// being read as a function name.
fn macro_target(action: &str) -> Option<Vec<rustyline::KeyEvent>> {
    let inner = action.strip_prefix('"')?.strip_suffix('"')?;
    if inner.is_empty() {
        return Some(Vec::new());
    }
    readline::parse_sequence(inner)
}

/// `bind -X` / `bind -p`: what is bound, in the form that would bind it again.
fn list(commands_only: bool) {
    for entry in readline::entries() {
        match &entry.bound {
            Bound::Command(command) => println!("\"{}\": {}", spelling(&entry.spec), command),
            Bound::Function(name) if !commands_only => {
                println!("\"{}\": {}", spelling(&entry.spec), name)
            }
            Bound::Macro { text, .. } if !commands_only => {
                println!("\"{}\": \"{text}\"", spelling(&entry.spec))
            }
            Bound::Function(_) | Bound::Macro { .. } => {}
        }
    }
}

/// The spec without the quotes it was written with, since the listing adds its own.
fn spelling(spec: &str) -> &str {
    spec.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(spec)
}

#[cfg(test)]
#[path = "bind/tests.rs"]
mod tests;
