//! Where the command history lives, how much of it is kept, and the `history` builtin
//! (PLAN R9.11).
//!
//! Three separate defects were behind this file. rustyline's default `max_history_size` is 100,
//! so a session silently forgot its 101st command; `history_ignore_space` was off, so the
//! ` password …` convention did nothing; and the whole file was rewritten on exit, so two
//! sessions open at once each ended with only their own commands.
//!
//! The `history` builtin has to read what the *line editor* holds, and `BuiltinFn` is a bare
//! `fn` pointer with nowhere to put a captured editor. So the REPL publishes a snapshot here
//! after every line, and the builtin reads that. The alternative — teaching the library about
//! the editor — would put rustyline in the dependency path of every `Environment`.

/// How many jobs `\j` in a `$PS1` reports.
///
/// Zero, and honestly so: [`crate::exec::job::JobManager`] installs the signal handlers but keeps
/// no table of running jobs, so there is nothing to count. Reporting zero is what a shell with no
/// background jobs would say anyway; when job tracking arrives this is the one place to change.
pub fn job_count() -> usize {
    0
}

use oslo::Environment;
use oslo::error::Result;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// The default number of entries kept when `$HISTSIZE` says nothing.
///
/// bash's own default is 500; 10000 is the number nearly every distribution's default profile
/// sets instead, and history that ends before yesterday's command is history nobody searches.
const DEFAULT_HISTSIZE: usize = 10_000;

/// What the editor's history is configured from.
#[derive(Debug, PartialEq, Eq)]
pub struct Settings {
    /// The file history is loaded from and appended to, or `None` when history is not saved.
    pub file: Option<PathBuf>,
    /// Maximum entries held in memory and written to the file.
    pub max_size: usize,
}

/// Read `$HISTFILE` and `$HISTSIZE`.
///
/// An explicitly *empty* `HISTFILE` disables the file, which is the documented way to run a
/// session that leaves no trace; an unset one falls back to `~/.oslo_history`.
pub fn settings(env: &Environment) -> Settings {
    let file = match env.get_var("HISTFILE") {
        Some("") => None,
        Some(path) => Some(PathBuf::from(path)),
        None => home(env).map(|h| h.join(".oslo_history")),
    };
    Settings {
        file,
        max_size: histsize(env.get_var("HISTSIZE")),
    }
}

/// `$HISTSIZE` as a size, falling back to the default for anything that is not a number.
///
/// A negative or unparseable `HISTSIZE` means "the user did not say", not "keep nothing":
/// silently disabling history because of a typo in a profile is the worse failure. Zero is
/// honoured, because writing `HISTSIZE=0` is unambiguous.
fn histsize(raw: Option<&str>) -> usize {
    match raw {
        Some(text) => text.trim().parse::<usize>().unwrap_or(DEFAULT_HISTSIZE),
        None => DEFAULT_HISTSIZE,
    }
}

fn home(env: &Environment) -> Option<PathBuf> {
    let home = env
        .get_var("HOME")
        .map(str::to_string)
        .or_else(|| std::env::var("HOME").ok())?;
    if home.is_empty() {
        None
    } else {
        Some(PathBuf::from(home))
    }
}

/// A line the editor must not remember.
///
/// rustyline applies `history_ignore_space` in `History::add`, which only sees what we hand it —
/// and what we hand it is the line *after* history expansion, which never starts with a space.
/// So the test has to be made against the line as typed, here, or ` secret` is stored despite
/// the leading space that exists precisely to prevent that.
pub fn is_secret(raw_line: &str) -> bool {
    raw_line.starts_with(|c: char| c.is_whitespace())
}

/// What the `history` builtin prints, published by the REPL after every line.
static SNAPSHOT: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Set by `history -c`, cleared by the REPL once it has emptied the editor's own history.
static CLEAR_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Publish the editor's current history for the builtin to read.
pub fn publish(entries: Vec<String>) {
    if let Ok(mut guard) = SNAPSHOT.lock() {
        *guard = entries;
    }
}

/// Whether `history -c` ran since the last check, clearing the flag.
pub fn take_clear_request() -> bool {
    CLEAR_REQUESTED.swap(false, Ordering::SeqCst)
}

/// Make `history` a builtin of this shell.
///
/// Registered for non-interactive shells too, so that `type history` and completion agree with
/// what the shell can actually run; there it reports an empty history, exactly as bash does.
pub fn register(env: &mut Environment) {
    env.register_custom_builtin("history", builtin_history);
}

fn builtin_history(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let entries = SNAPSHOT.lock().map(|g| g.clone()).unwrap_or_default();

    match args.get(1).map(String::as_str) {
        None => print_entries(&entries, entries.len()),
        Some("-c") => {
            if let Ok(mut guard) = SNAPSHOT.lock() {
                guard.clear();
            }
            CLEAR_REQUESTED.store(true, Ordering::SeqCst);
        }
        Some(other) if other.starts_with('-') && other.len() > 1 => {
            eprintln!("oslo: history: {}: invalid option", other);
            eprintln!("history: usage: history [n] | history -c");
            return Ok(2);
        }
        Some(count) => match count.parse::<usize>() {
            Ok(n) => print_entries(&entries, n),
            Err(_) => {
                eprintln!("oslo: history: {}: numeric argument required", count);
                return Ok(1);
            }
        },
    }
    Ok(0)
}

/// Print the last `count` entries, numbered from 1 as bash numbers them.
///
/// The numbers are what `!42` refers to, so they count from the start of the session's history
/// and not from the start of the printed slice.
fn print_entries(entries: &[String], count: usize) {
    let start = entries.len().saturating_sub(count);
    for (i, entry) in entries.iter().enumerate().skip(start) {
        println!("{:5}  {}", i + 1, entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histsize_defaults_and_survives_nonsense() {
        assert_eq!(histsize(None), DEFAULT_HISTSIZE);
        assert_eq!(histsize(Some("nope")), DEFAULT_HISTSIZE);
        assert_eq!(histsize(Some("-5")), DEFAULT_HISTSIZE);
        assert_eq!(histsize(Some(" 250 ")), 250);
        assert_eq!(histsize(Some("0")), 0);
    }

    #[test]
    fn an_empty_histfile_disables_the_file() {
        let mut env = Environment::new();
        env.set_var("HISTFILE", "", false);
        assert_eq!(settings(&env).file, None);
    }

    #[test]
    fn histfile_is_taken_from_the_variable() {
        let mut env = Environment::new();
        env.set_var("HISTFILE", "/tmp/xyz_history", false);
        env.set_var("HISTSIZE", "42", false);
        let s = settings(&env);
        assert_eq!(s.file, Some(PathBuf::from("/tmp/xyz_history")));
        assert_eq!(s.max_size, 42);
    }

    #[test]
    fn a_leading_space_marks_a_line_secret() {
        assert!(is_secret(" secret --token x"));
        assert!(is_secret("\techo hi"));
        assert!(!is_secret("echo hi"));
    }

    #[test]
    fn the_clear_flag_is_consumed_once() {
        let mut env = Environment::new();
        publish(vec!["echo one".to_string()]);
        assert_eq!(
            builtin_history(&mut env, &["history".into(), "-c".into()]).unwrap(),
            0
        );
        assert!(SNAPSHOT.lock().unwrap().is_empty());
        assert!(take_clear_request());
        assert!(!take_clear_request());
    }

    #[test]
    fn an_invalid_option_is_a_usage_error() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_history(&mut env, &["history".into(), "-z".into()]).unwrap(),
            2
        );
        assert_eq!(
            builtin_history(&mut env, &["history".into(), "two".into()]).unwrap(),
            1
        );
    }
}
