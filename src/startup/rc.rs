//! Startup files and prompt strings (PLAN R9.10).
//!
//! Before this, the only thing a new shell read was `~/.config/rush/init.lua`, and only in the
//! REPL: there was no way to define an alias or a function for an interactive session, and `-c`
//! and script shells read no configuration at all. Two files fix that, and they are deliberately
//! different in kind:
//!
//! * `~/.rushrc` is *shell* syntax, sourced by an interactive shell through the ordinary
//!   `source` builtin — so an alias, a function and a `PS1=` in it behave exactly as the same
//!   lines typed at the prompt would.
//! * `$ENV` is POSIX's own hook, and its value is subject to parameter expansion before use,
//!   which is why it goes through the expander rather than being taken literally.
//!
//! `init.lua` remains an extra layer on top, not the only one.

use rush::Environment;
use rush::env::builtins::builtin_source;
use rush::error::ShellError;
use rush::expand::expand_word_to_string;
use rush::interactive::prompt::render_default_left_prompt;
use rush::lexer::parse_single_word;
use std::path::PathBuf;

/// A startup file asked the shell to end: `exit 3` in `.rushrc` is still an `exit`.
pub type ExitRequest = Option<i32>;

/// Read the files a shell of this kind reads before its first command.
///
/// Returns the status the shell must exit with, if a startup file ran `exit`.
///
/// Nothing here is fatal. A startup file that does not exist is not an error (that is the normal
/// case), and one that fails half way through leaves the shell running with whatever it managed
/// to set — a broken rc file must not cost you your shell.
pub fn load_startup_files(env: &mut Environment, interactive: bool) -> ExitRequest {
    let mut sourced: Vec<PathBuf> = Vec::new();

    if interactive && let Some(rc) = rushrc_path(env) {
        if let Some(status) = source_if_present(env, &rc) {
            return Some(status);
        }
        sourced.push(rc);
    }

    if let Some(path) = env_file(env)
        && !sourced.contains(&path)
        && let Some(status) = source_if_present(env, &path)
    {
        return Some(status);
    }

    None
}

/// `$HOME/.rushrc`, when `$HOME` says anything usable.
fn rushrc_path(env: &Environment) -> Option<PathBuf> {
    let home = env
        .get_var("HOME")
        .map(str::to_string)
        .or_else(|| std::env::var("HOME").ok())?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".rushrc"))
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
            eprintln!("rush: {}: {}", path.display(), e);
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
        None => render_default_left_prompt(last_status),
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
            Some('s') => out.push_str("rush"),
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
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "localhost".to_string())
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
