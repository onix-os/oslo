//! Startup files and prompt strings (PLAN R9.10).
//!
//! Before this, the only thing a new shell read was `~/.config/oslo/init.lua`, and only in the
//! REPL: there was no way to define an alias or a function for an interactive session, and `-c`
//! and script shells read no configuration at all. Two files fix that, and they are deliberately
//! different in kind:
//!
//! * The **config** is Lua, is `$XDG_CONFIG_HOME/oslo/init.lua`, and is loaded by
//!   [`super::lua_init`] rather than sourced here. There is no shell-syntax config file at all:
//!   one file, one language, one place.
//!
//!   This is the *only* rule oslo gets to make. Everything below is POSIX's, and oslo is a
//!   `/bin/sh` before it is anything else.
//! * `$ENV` is POSIX's own hook and stays *shell* syntax, because POSIX defines it that way and
//!   oslo has to be a real `/bin/sh`. Its value is subject to parameter expansion before use,
//!   which is why it goes through the expander rather than being taken literally.

use oslo_base::error::ShellError;
use oslo_shell::Environment;
use oslo_shell::env::builtins::builtin_source;
use oslo_shell::expand::expand_word_to_string;
use oslo_shell::lexer::parse_single_word;
use oslo_ui::prompt::render_default_left_prompt;
use std::path::PathBuf;

/// A startup file asked the shell to end: `exit 3` in `$ENV` is still an `exit`.
pub type ExitRequest = Option<i32>;

/// Read the files a shell of this kind reads before its first command.
///
/// Returns the status the shell must exit with, if a startup file ran `exit`.
///
/// Nothing here is fatal. A startup file that does not exist is not an error (that is the normal
/// case), and one that fails half way through leaves the shell running with whatever it managed
/// to set — a broken rc file must not cost you your shell.
pub fn load_startup_files(
    env: &mut Environment,
    interactive: bool,
    login: bool,
    no_profile: bool,
) -> ExitRequest {
    let mut sourced: Vec<PathBuf> = Vec::new();

    // The config is Lua and is loaded by `super::lua_init`, not sourced here — see `config_path`.
    // What remains in this function is POSIX's own, which is not oslo's to remove.
    let _ = interactive;

    // **A login shell reads the profile files, and only a login shell does.** `-l`, or an `argv[0]`
    // beginning with `-`, which is how `login`, `su -` and every display manager start one.
    //
    // `/etc/profile` first so a user's own file can override the system's, which is the order dash
    // and bash both use — verified by watching what they open rather than from memory.
    //
    // **`/etc/profile.d` is deliberately absent.** `/etc/profile` walks it itself with `run-parts`;
    // a shell that also walked it would source all seventeen of this machine's files twice.
    //
    // **`--noprofile` is honoured here.** A caller handing a script to `sh -l` and asking for no
    // profile wants the script's own behaviour; reading the profile anyway would give it whatever
    // the person at the keyboard put in theirs, which is the failure the flag exists to prevent.
    if login && !no_profile {
        // Built rather than written inline: a shell with no `$HOME` still reads `/etc/profile`,
        // and an early return for the missing one would have skipped it.
        let mut profiles = vec![PathBuf::from("/etc/profile")];
        profiles.extend(home_profile(env));
        for path in profiles {
            if sourced.contains(&path) {
                continue;
            }
            sourced.push(path.clone());
            if let Some(status) = source_if_present(env, &path) {
                return Some(status);
            }
        }
    }

    // Last, because a login shell's `~/.profile` is where `$ENV` is usually set — reading it first
    // would use the value from before the profile ran.
    if let Some(path) = env_file(env)
        && !sourced.contains(&path)
        && let Some(status) = source_if_present(env, &path)
    {
        return Some(status);
    }

    None
}

/// `$HOME/.profile`.
///
/// **Root needs no special case**: `$HOME` is `/root` for root, so this is `/root/.profile` there
/// and `~/.profile` for everybody else, exactly as every other shell resolves it.
///
/// There is no `~/.oslo_profile`. bash looks for `~/.bash_profile` first so that bash-only login
/// setup has somewhere to live, but oslo's own configuration is `init.lua` — a second oslo-only
/// file would be a third place to look and a question about which one wins.
fn home_profile(env: &mut Environment) -> Option<PathBuf> {
    let home = env.get_var("HOME").map(str::to_string)?;
    (!home.is_empty()).then(|| PathBuf::from(home).join(".profile"))
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
                oslo_ui::marks::path(&path.display().to_string()),
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

/// The right-hand prompt, from `$RPS1` — or `$RPROMPT`, which is what zsh calls it.
///
/// bash has no right prompt, so there is no name to inherit and no precedent to follow. Both
/// spellings are accepted because both are already in people's fingers: `RPS1` is zsh's primary
/// name and `RPROMPT` its older one, and an integration that sets either should be seen. `RPS1`
/// wins when both are set, matching zsh.
///
/// `None` when neither is set, so the caller can fall back rather than draw an empty column.
/// A variable set to the empty string is *not* nothing: it is an explicit request for no right
/// prompt, and it suppresses oslo's default.
pub fn rps1(env: &mut Environment) -> Option<String> {
    let raw = env
        .get_var("RPS1")
        .or_else(|| env.get_var("RPROMPT"))
        .map(str::to_string)?;
    Some(expand_prompt(env, &raw))
}

/// The continuation prompt, shown for the second and later lines of one command.
pub fn ps2(env: &mut Environment) -> String {
    ps2_if_set(env).unwrap_or_else(|| "> ".to_string())
}

/// `$PS2`, or `None` when the user never set one.
///
/// The distinction the built-in continuation marker needs: it is a *default*, so it must not
/// override a `$PS2` somebody wrote, and "unset" cannot be told from "set to `> `" otherwise.
pub fn ps2_if_set(env: &mut Environment) -> Option<String> {
    let raw = env.get_var("PS2").map(str::to_string)?;
    Some(expand_prompt(env, &raw))
}

/// Turn a prompt string into what is printed.
///
/// The order is bash's: backslash escapes are decoded first, then the result goes through
/// parameter expansion and command substitution. Doing it the other way round is not a
/// refinement but a bug — `parse_single_word` reads a backslash as *quoting*, so `PS1='\w'`
/// would expand to the letter `w`.
///
/// # What an escape produces is not shell source
///
/// **This was a command injection, and an easy one to meet.** `\w` is the working directory, and
/// the second pass ran command substitution over the result — so a directory named
/// `$(touch /tmp/pwned)` executed that on every prompt, and `cd`-ing into a cloned repository or an
/// unpacked archive was enough to do it. bash does not: with `promptvars` on, which is its default,
/// it expands the string *you wrote* and prints what `\w` produced literally. Measured against
/// bash 5 rather than assumed.
///
/// So every escape whose value comes from outside the prompt string — the directory, the user, the
/// host, the tty — is decoded to an opaque placeholder, and the real text is put back after the
/// expansion has run and can no longer act on it. The escapes that produce a fixed character the
/// author chose (`\n`, `\$`, an octal byte) are left alone: they carry nothing from outside and
/// `PS1='\$USER'` goes on meaning what it always did.
pub fn expand_prompt(env: &mut Environment, raw: &str) -> String {
    let (decoded, values) = decode_escapes(env, raw);
    let expanded = match expand_prompt_free_text(env, &decoded) {
        Some(text) => text,
        None => decoded,
    };
    restore(&expanded, &values)
}

/// The two bytes that bracket a placeholder, and cannot be typed into a prompt by accident.
const HOLD_OPEN: char = '\u{1}';
const HOLD_CLOSE: char = '\u{2}';

/// Stand `value` aside, leaving a marker the expander cannot act on.
fn protect(out: &mut String, values: &mut Vec<String>, value: &str) {
    out.push(HOLD_OPEN);
    out.push_str(&values.len().to_string());
    out.push(HOLD_CLOSE);
    values.push(value.to_string());
}

/// Put the stood-aside values back where their markers are.
///
/// A marker that does not parse is left exactly as it is: the text may have come from the user's
/// own prompt, and eating something because it looked like ours would be the same class of mistake
/// this whole arrangement exists to avoid.
fn restore(text: &str, values: &[String]) -> String {
    if values.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(HOLD_OPEN) {
        out.push_str(&rest[..at]);
        let after = &rest[at + HOLD_OPEN.len_utf8()..];
        match after.find(HOLD_CLOSE) {
            Some(end) => {
                match after[..end]
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| values.get(i))
                {
                    Some(value) => out.push_str(value),
                    None => {
                        out.push(HOLD_OPEN);
                        out.push_str(&after[..end]);
                        out.push(HOLD_CLOSE);
                    }
                }
                rest = &after[end + HOLD_CLOSE.len_utf8()..];
            }
            None => {
                out.push(HOLD_OPEN);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
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
fn decode_escapes(env: &Environment, raw: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(raw.len());
    // What the escapes below produced, stood aside so the expansion that follows cannot act on it.
    // See [`expand_prompt`] — this is a command injection when it is not done.
    let mut values: Vec<String> = Vec::new();
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
            Some('s') => protect(&mut out, &mut values, "oslo"),
            Some('$') => out.push(if nix::unistd::geteuid().is_root() {
                '#'
            } else {
                '$'
            }),
            Some('u') => protect(&mut out, &mut values, &user_name(env)),
            Some('h') => protect(
                &mut out,
                &mut values,
                host_name().split('.').next().unwrap_or_default(),
            ),
            Some('H') => protect(&mut out, &mut values, &host_name()),
            Some('w') => protect(&mut out, &mut values, &tilde_pwd(env)),
            Some('W') => {
                let pwd = tilde_pwd(env);
                let base = pwd.rsplit('/').next().unwrap_or(&pwd);
                protect(
                    &mut out,
                    &mut values,
                    if base.is_empty() { &pwd } else { base },
                );
            }
            // The clock escapes. Local time, not UTC: a prompt showing the wrong hour is worse
            // than one showing none, and `localtime_r` is where the system keeps the answer.
            Some('t') => protect(&mut out, &mut values, &clock("%H:%M:%S")),
            Some('T') => protect(&mut out, &mut values, &clock("%I:%M:%S")),
            Some('@') => protect(&mut out, &mut values, &clock("%I:%M %p")),
            Some('A') => protect(&mut out, &mut values, &clock("%H:%M")),
            Some('d') => protect(&mut out, &mut values, &clock("%a %b %e")),
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
                let shown = clock(if format.is_empty() { "%X" } else { &format });
                protect(&mut out, &mut values, &shown);
            }
            // Which line of history this will be. bash counts from one.
            Some('!') | Some('#') => protect(
                &mut out,
                &mut values,
                &(oslo_ui::recall::len() + 1).to_string(),
            ),
            // Jobs the shell is tracking.
            Some('j') => protect(
                &mut out,
                &mut values,
                &crate::startup::history::job_count().to_string(),
            ),
            // The terminal's basename, as bash reports it.
            Some('l') => {
                let tty = nix::unistd::ttyname(std::io::stdin())
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_default();
                protect(&mut out, &mut values, &tty);
            }
            Some('v') | Some('V') => protect(&mut out, &mut values, oslo_base::version::current()),
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
            // oslo measures what a prompt *prints* — escapes are skipped by `display_width` — so
            // the markers say nothing it does not already know, and are dropped rather than
            // emitted: passing them through would print two stray control characters.
            Some('[') | Some(']') => {}
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    (out, values)
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
        assert!(prompt.contains('>'), "{prompt:?}");
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

    /// **What an escape produced is printed, not run.**
    ///
    /// `\w` is the working directory and the prompt used to go through command substitution
    /// *after* it was substituted in, so a directory named `$(touch /tmp/pwned)` ran that on every
    /// prompt — entering a cloned repository was enough. `\u` is the same door with a settable
    /// value, which is why the test uses it: `$USER` is a variable, `current_dir` is the process.
    ///
    /// bash 5 with `promptvars` on — its default — prints the name literally, and so does this.
    #[test]
    fn an_escape_that_looks_like_a_substitution_is_not_run() {
        let mut env = env_with(&[("USER", "$(echo pwned)")]);
        assert_eq!(expand_prompt(&mut env, "\\u"), "$(echo pwned)");
        assert_eq!(expand_prompt(&mut env, "[\\u]"), "[$(echo pwned)]");

        // Backticks are the other spelling, and were the other half of the same hole.
        let mut env = env_with(&[("USER", "`echo pwned`")]);
        assert_eq!(expand_prompt(&mut env, "\\u"), "`echo pwned`");

        // A variable *in the prompt the author wrote* still expands: that is `promptvars`, and it
        // is the behaviour this must not take away while closing the hole.
        let mut env = env_with(&[("USER", "ada"), ("WHO", "somebody")]);
        assert_eq!(expand_prompt(&mut env, "$WHO"), "somebody");
        assert_eq!(expand_prompt(&mut env, "\\u"), "ada");
    }

    #[test]
    fn unknown_escapes_keep_their_backslash() {
        let env = env_with(&[]);
        assert_eq!(decode_escapes(&env, "\\q").0, "\\q");
        assert_eq!(decode_escapes(&env, "a\\\\b").0, "a\\b");
        assert_eq!(decode_escapes(&env, "\\[x\\]").0, "x");
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
