//! Startup files and prompt strings (PLAN R9.10).
//!
//! Before this, the only thing a new shell read was `~/.config/oslo/init.lua`, and only in the
//! REPL: there was no way to define an alias or a function for an interactive session, and `-c`
//! and script shells read no configuration at all. Two files fix that, and they are deliberately
//! different in kind:
//!
//! * `~/.oslorc` is **Lua**, and is loaded by [`super::lua_init`] rather than sourced here. It
//!   used to be shell syntax; one config file in one language is the decision, and the shell
//!   half of it is gone.
//! * `$ENV` is POSIX's own hook and stays *shell* syntax, because POSIX defines it that way and
//!   oslo has to be a real `/bin/sh`. Its value is subject to parameter expansion before use,
//!   which is why it goes through the expander rather than being taken literally.

use oslo::Environment;
use oslo::env::builtins::builtin_source;
use oslo::error::ShellError;
use oslo::expand::expand_word_to_string;
use oslo::interactive::prompt::render_default_left_prompt;
use oslo::lexer::parse_single_word;
use std::path::PathBuf;

/// A startup file asked the shell to end: `exit 3` in `.oslorc` is still an `exit`.
pub type ExitRequest = Option<i32>;

/// Read the files a shell of this kind reads before its first command.
///
/// Returns the status the shell must exit with, if a startup file ran `exit`.
///
/// Nothing here is fatal. A startup file that does not exist is not an error (that is the normal
/// case), and one that fails half way through leaves the shell running with whatever it managed
/// to set — a broken rc file must not cost you your shell.
pub fn load_startup_files(env: &mut Environment, interactive: bool) -> ExitRequest {
    let sourced: Vec<PathBuf> = Vec::new();

    // `~/.oslorc` is **Lua** and is loaded by `super::lua_init`, not sourced here — see
    // `config_path`. What remains in this function is POSIX's own hook.
    let _ = interactive;

    if let Some(path) = env_file(env)
        && !sourced.contains(&path)
        && let Some(status) = source_if_present(env, &path)
    {
        return Some(status);
    }

    None
}

/// The file `$ENV` names, after parameter expansion, or `None` when the variable is unset,
/// empty, or must not be trusted.
///
/// POSIX defines `$ENV` as a *word* that is expanded, so `ENV=$HOME/.shrc` is the documented
/// spelling and taking the value literally would break it. The privilege check is the rule every
/// other shell applies: a set-user-ID shell that sourced a file named by the invoking user's
/// environment would hand that user the owner's privileges, so it reads nothing.
fn env_file(env: &mut Environment) -> Option<PathBuf> {
    let raw = env.get_var("ENV").map(str::to_string)?;
    if raw.is_empty() || !running_unprivileged() {
        return None;
    }
    let expanded = expand_prompt_free_text(env, &raw)?;
    if expanded.is_empty() {
        return None;
    }
    Some(PathBuf::from(expanded))
}

fn running_unprivileged() -> bool {
    nix::unistd::getuid() == nix::unistd::geteuid()
        && nix::unistd::getgid() == nix::unistd::getegid()
}

/// Source `path` if it exists, reporting nothing when it does not.
///
/// Returns `Some(status)` only when the file ran `exit`, which ends the shell there and then —
/// `exit 0` at the end of an rc file is a legitimate, if unusual, thing to write.
fn source_if_present(env: &mut Environment, path: &std::path::Path) -> ExitRequest {
    if !path.is_file() {
        return None;
    }
    let args = vec!["source".to_string(), path.display().to_string()];
    match builtin_source(env, &args) {
        // `source` prints its own diagnostics for a file that will not read or will not parse.
        Ok(_) => None,
        Err(ShellError::Exit(code)) => Some(code),
        Err(e) => {
            eprintln!(
                "oslo: {}: {}",
                oslo::interactive::marks::path(&path.display().to_string()),
                e
            );
            None
        }
    }
}

/// The primary prompt: `$PS1` when the user set one, the built-in prompt otherwise.
///
/// `last_status` only reaches the default renderer — a user-supplied `PS1` says what it wants
/// about `$?` by writing `$?` in it, which the expansion below resolves.
pub fn ps1(env: &mut Environment, last_status: i32) -> String {
    match env.get_var("PS1").map(str::to_string) {
        Some(raw) => expand_prompt(env, &raw),
        // The shell language's own name for the segment. A `PS1` that the user wrote wins
        // above; this is the default, and it says which language the line will be read as.
        None => render_default_left_prompt(last_status, "sh"),
    }
}

/// The continuation prompt, shown for the second and later lines of one command.
pub fn ps2(env: &mut Environment) -> String {
    match env.get_var("PS2").map(str::to_string) {
        Some(raw) => expand_prompt(env, &raw),
        None => "> ".to_string(),
    }
}

/// Turn a prompt string into what is printed.
///
/// The order is bash's: backslash escapes are decoded first, then the result goes through
/// parameter expansion and command substitution. Doing it the other way round is not a
/// refinement but a bug — `parse_single_word` reads a backslash as *quoting*, so `PS1='\w'`
/// would expand to the letter `w`.
pub fn expand_prompt(env: &mut Environment, raw: &str) -> String {
    let decoded = decode_escapes(env, raw);
    expand_prompt_free_text(env, &decoded).unwrap_or(decoded)
}

/// Run one string through the shell's own expander, or `None` if it will not expand.
///
/// A prompt that fails to expand must not kill the shell or the loop, so every caller has a
/// fallback; this returns `None` rather than reporting, because an error printed once per
/// keystroke-worth of prompt would be unusable.
fn expand_prompt_free_text(env: &mut Environment, text: &str) -> Option<String> {
    let word = parse_single_word(text).ok()?;
    expand_word_to_string(env, &word).ok()
}

/// Decode the `\w`-style escapes bash defines for prompts.
///
/// A deliberately small set: the ones that describe *where and who you are*, which is all a
/// prompt can say that a plain expansion cannot. An escape that is not in the table keeps its
/// backslash, so a prompt written for bash degrades to something readable rather than to
/// silence.
fn decode_escapes(env: &Environment, raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            None => out.push('\\'),
            Some('\\') => out.push('\\'),
            Some('a') => out.push('\u{7}'),
            Some('e') => out.push('\u{1b}'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('s') => out.push_str("oslo"),
            Some('$') => out.push(if nix::unistd::geteuid().is_root() {
                '#'
            } else {
                '$'
            }),
            Some('u') => out.push_str(&user_name(env)),
            Some('h') => out.push_str(host_name().split('.').next().unwrap_or_default()),
            Some('H') => out.push_str(&host_name()),
            Some('w') => out.push_str(&tilde_pwd(env)),
            Some('W') => {
                let pwd = tilde_pwd(env);
                let base = pwd.rsplit('/').next().unwrap_or(&pwd);
                out.push_str(if base.is_empty() { &pwd } else { base });
            }
            // The clock escapes. Local time, not UTC: a prompt showing the wrong hour is worse
            // than one showing none, and `localtime_r` is where the system keeps the answer.
            Some('t') => out.push_str(&clock("%H:%M:%S")),
            Some('T') => out.push_str(&clock("%I:%M:%S")),
            Some('@') => out.push_str(&clock("%I:%M %p")),
            Some('A') => out.push_str(&clock("%H:%M")),
            Some('d') => out.push_str(&clock("%a %b %e")),
            // `\D{...}` takes a strftime format of its own.
            Some('D') => {
                let mut format = String::new();
                if chars.as_str().starts_with('{') {
                    chars.next();
                    for c in chars.by_ref() {
                        if c == '}' {
                            break;
                        }
                        format.push(c);
                    }
                }
                out.push_str(&clock(if format.is_empty() { "%X" } else { &format }));
            }
            // Which line of history this will be. bash counts from one.
            Some('!') | Some('#') => {
                out.push_str(&(oslo::interactive::recall::len() + 1).to_string())
            }
            // Jobs the shell is tracking.
            Some('j') => out.push_str(&crate::startup::history::job_count().to_string()),
            // The terminal's basename, as bash reports it.
            Some('l') => out.push_str(
                &nix::unistd::ttyname(std::io::stdin())
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_default(),
            ),
            Some('v') | Some('V') => out.push_str(env!("CARGO_PKG_VERSION")),
            // `\nnn` is an octal byte, which is how a prompt reaches a character it cannot type.
            Some(d @ '0'..='7') => {
                let mut digits = String::from(d);
                while digits.len() < 3 {
                    match chars.clone().next() {
                        Some(next @ '0'..='7') => {
                            chars.next();
                            digits.push(next);
                        }
                        _ => break,
                    }
                }
                match u8::from_str_radix(&digits, 8) {
                    Ok(byte) => out.push(byte as char),
                    Err(_) => out.push_str(&digits),
                }
            }
            // `\[` and `\]` bracket non-printing runs so bash can measure the prompt's width.
            // rustyline measures it itself, so the markers are dropped rather than emitted:
            // passing them through would print two stray control characters.
            Some('[') | Some(']') => {}
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

/// The current local time, formatted.
///
/// Local rather than UTC — the rest of oslo's date handling refuses timezones on the grounds that a
/// plausible-but-wrong timestamp is worse than none, which is right for a *script*. A prompt clock
/// is the opposite case: it is read at a glance and only useful if it agrees with the wall.
fn clock(format: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut tm: nix::libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `now` is a valid `time_t` and `tm` is owned here for the whole call.
    if unsafe { nix::libc::localtime_r(&now, &mut tm) }.is_null() {
        return String::new();
    }
    let mut out = vec![0u8; 128];
    let c_format = match std::ffi::CString::new(format) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    // SAFETY: a buffer this call owns, its own length, and a NUL-terminated format.
    let written =
        unsafe { nix::libc::strftime(out.as_mut_ptr().cast(), out.len(), c_format.as_ptr(), &tm) };
    out.truncate(written);
    String::from_utf8(out).unwrap_or_default()
}

fn user_name(env: &Environment) -> String {
    env.get_var("USER")
        .map(str::to_string)
        .or_else(|| std::env::var("USER").ok())
        .or_else(|| std::env::var("LOGNAME").ok())
        .unwrap_or_else(|| nix::unistd::getuid().to_string())
}

fn host_name() -> String {
    if let Ok(name) = std::env::var("HOSTNAME")
        && !name.is_empty()
    {
        return name;
    }
    // `gethostname(2)` rather than /proc/sys/kernel/hostname: one syscall, no file to read and
    // no trailing newline to remember to strip.
    nix::unistd::gethostname()
        .ok()
        .and_then(|n| n.into_string().ok())
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "localhost".to_string())
}

/// The working directory with `$HOME` written as `~`, as every prompt shows it.
fn tilde_pwd(env: &Environment) -> String {
    let pwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".to_string());
    let home = env
        .get_var("HOME")
        .map(str::to_string)
        .or_else(|| std::env::var("HOME").ok())
        .unwrap_or_default();
    if !home.is_empty() && pwd == home {
        return "~".to_string();
    }
    match pwd.strip_prefix(&format!("{}/", home)) {
        Some(rest) if !home.is_empty() => format!("~/{}", rest),
        _ => pwd,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(vars: &[(&str, &str)]) -> Environment {
        let mut env = Environment::new();
        for (k, v) in vars {
            env.set_var(k, v, false);
        }
        env
    }

    #[test]
    fn ps1_falls_back_to_the_built_in_prompt() {
        let mut env = env_with(&[]);
        env.unset_var("PS1");
        let prompt = ps1(&mut env, 0);
        assert!(prompt.contains('❯'), "{prompt:?}");
    }

    #[test]
    fn ps1_is_used_when_set_and_is_expanded() {
        let mut env = env_with(&[("PS1", "[$MARK] "), ("MARK", "here")]);
        assert_eq!(ps1(&mut env, 0), "[here] ");
    }

    #[test]
    fn ps2_defaults_to_the_posix_string() {
        let mut env = env_with(&[]);
        env.unset_var("PS2");
        assert_eq!(ps2(&mut env), "> ");
    }

    #[test]
    fn ps2_is_used_when_set() {
        let mut env = env_with(&[("PS2", "cont> ")]);
        assert_eq!(ps2(&mut env), "cont> ");
    }

    #[test]
    fn backslash_escapes_are_decoded_before_expansion() {
        // The regression this pins: expanding first turns `\w` into the letter `w`, because a
        // backslash quotes the character after it in word syntax.
        let mut env = env_with(&[("HOME", "/nowhere")]);
        let out = expand_prompt(&mut env, "\\w");
        assert_ne!(out, "w");
        assert!(out.starts_with('/') || out.starts_with('~'), "{out:?}");
    }

    #[test]
    fn unknown_escapes_keep_their_backslash() {
        let env = env_with(&[]);
        assert_eq!(decode_escapes(&env, "\\q"), "\\q");
        assert_eq!(decode_escapes(&env, "a\\\\b"), "a\\b");
        assert_eq!(decode_escapes(&env, "\\[x\\]"), "x");
    }

    #[test]
    fn env_is_expanded_not_taken_literally() {
        let mut env = env_with(&[("ENV", "$HOME/.shrc"), ("HOME", "/tmp/nowhere")]);
        assert_eq!(
            env_file(&mut env),
            Some(PathBuf::from("/tmp/nowhere/.shrc"))
        );
    }

    #[test]
    fn an_unset_or_empty_env_names_no_file() {
        let mut env = env_with(&[]);
        env.unset_var("ENV");
        assert_eq!(env_file(&mut env), None);
        env.set_var("ENV", "", false);
        assert_eq!(env_file(&mut env), None);
    }

    #[test]
    fn a_missing_startup_file_is_not_an_error() {
        let mut env = env_with(&[]);
        assert_eq!(
            source_if_present(&mut env, std::path::Path::new("/nonexistent/rc")),
            None
        );
    }
}
