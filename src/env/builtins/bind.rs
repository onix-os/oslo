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
use crate::interactive::readline::{self, Bound};

pub fn builtin_bind(env: &mut Environment, args: &[String]) -> Result<i32> {
    let _ = env;
    let mut operands = args[1..].iter().map(String::as_str).peekable();
    let mut run_command = false;
    let mut status = 0;

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
                if !readline::unbind(spec) {
                    eprintln!("oslo: bind: {spec}: cannot unbind");
                    status = 1;
                }
            }
            // The listings. `-X` is the one a plugin actually reads back: it asks for the `-x`
            // bindings in re-inputtable form, which is how a second `init` avoids double-binding.
            "-X" => list(true),
            "-p" | "-P" | "-s" | "-S" => list(false),
            "-l" | "-v" | "-V" => {}
            // A readline variable — `bind 'set completion-ignore-case on'`. oslo has its own
            // settings and does not implement readline's, so this is accepted and ignored rather
            // than reported: init scripts set these unconditionally and a diagnostic per line
            // would bury the ones that matter.
            "-m" | "-f" | "-q" => {
                operands.next();
            }
            "--" => {}
            other if other.starts_with('-') && other.len() > 1 => {
                eprintln!("oslo: bind: {other}: invalid option");
                status = 2;
            }
            spec => {
                if !apply(spec, run_command) {
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
fn apply(spec: &str, run_command: bool) -> bool {
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
    } else {
        Bound::Function(action.to_string())
    };
    match readline::bind(&key, bound) {
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

/// `bind -X` / `bind -p`: what is bound, in the form that would bind it again.
fn list(commands_only: bool) {
    for entry in readline::entries() {
        match &entry.bound {
            Bound::Command(command) => println!("\"{}\": {}", spelling(&entry.spec), command),
            Bound::Function(name) if !commands_only => {
                println!("\"{}\": {}", spelling(&entry.spec), name)
            }
            Bound::Function(_) => {}
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
