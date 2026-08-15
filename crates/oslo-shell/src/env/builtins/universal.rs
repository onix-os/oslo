//! `universal` — a variable that outlives this shell and reaches the others.
//!
//! ```text
//! universal THEME=dark          set it here, everywhere, and next time
//! universal -x EDITOR=hx        and export it to children
//! universal                     list them
//! universal -e THEME            erase
//! ```
//!
//! fish spells this `set -U`, which oslo cannot: `set` is POSIX's, and its options are shell
//! options and positional parameters. A separate word is clearer anyway — "universal" says what is
//! unusual about the variable, where `-U` says nothing to somebody reading a config.
//!
//! Setting one applies it to **this** shell immediately as well as writing the file, because the
//! alternative is a command that appears to do nothing until you open a new terminal.
//!
//! See [`crate::env::universal`] for the file and why it is locked.

use super::variables::quoting::single_quoted;
use crate::env::Environment;
use crate::env::announce::{Change, Scope, Source, announce};
use crate::env::scope::is_valid_identifier;
use crate::env::universal;
use oslo_base::error::Result;

pub fn builtin_universal(env: &mut Environment, args: &[String]) -> Result<i32> {
    let operands = &args[1..];
    if operands.first().is_some_and(|a| a == "--help" || a == "-h") {
        usage();
        return Ok(0);
    }

    if operands.is_empty() {
        // Listed in a form that can be pasted back, like `export -p`. Sorted, because the store is
        // a `BTreeMap` and a listing that changed order between runs could not be diffed.
        for (name, var) in universal::load() {
            let flag = if var.exported { "-x " } else { "" };
            println!("universal {flag}{name}={}", single_quoted(&var.value));
        }
        return Ok(0);
    }

    if operands[0] == "-e" || operands[0] == "--erase" {
        let names = &operands[1..];
        if names.is_empty() {
            eprintln!("universal: -e: a name is required");
            return Ok(2);
        }
        let mut status = 0;
        for name in names {
            match universal::erase(name) {
                Ok(true) => {
                    // Gone from the file *and* from this shell, or the value would linger here
                    // and nowhere else — the most confusing of the possible outcomes.
                    env.unset_var(name);
                    universal::forget_applied(name);
                    announce(name, Change::Erased, Scope::Universal, Source::Local);
                }
                Ok(false) => {
                    eprintln!("universal: {name}: not a universal variable");
                    status = 1;
                }
                Err(why) => {
                    eprintln!("universal: {why}");
                    status = 1;
                }
            }
        }
        return Ok(status);
    }

    let mut exported = false;
    let mut rest = operands;
    if rest[0] == "-x" || rest[0] == "--export" {
        exported = true;
        rest = &rest[1..];
    }
    if rest.is_empty() {
        eprintln!("universal: -x: an assignment is required");
        return Ok(2);
    }

    let mut status = 0;
    for word in rest {
        let Some((name, value)) = word.split_once('=') else {
            // A bare name promotes what this shell already has, which is what `universal PATH`
            // reads as and what `export NAME` does for its own axis.
            let Some(current) = env.get_var(word).map(str::to_string) else {
                eprintln!("universal: {word}: not set; write {word}=VALUE to give it one");
                status = 1;
                continue;
            };
            if let Err(why) = universal::set(word, &current, exported) {
                eprintln!("universal: {why}");
                status = 1;
            }
            continue;
        };
        if !is_valid_identifier(name) {
            eprintln!("universal: {name}: not a valid name");
            status = 1;
            continue;
        }
        if let Err(why) = universal::set(name, value, exported) {
            eprintln!("universal: {why}");
            status = 1;
            continue;
        }
        // Applied here too, so the command that just ran has visibly done something — and
        // *recorded*, so a later erase from another terminal knows this value is the file's rather
        // than something the user typed, and may take it away again.
        env.set_var(name, value, exported);
        universal::note_applied(name, value);
        announce(
            name,
            Change::Set { exported },
            Scope::Universal,
            Source::Local,
        );
    }
    Ok(status)
}

fn usage() {
    println!("Usage: universal [-x] NAME[=VALUE]...");
    println!("       universal -e NAME...    erase");
    println!("       universal               list them");
    println!();
    println!("A universal variable is written to a file, so it is still there next session, and");
    println!("every running oslo picks it up before its next prompt. `-x` also exports it to");
    println!("children. fish spells this `set -U`.");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        std::iter::once("universal".to_string())
            .chain(words.iter().map(|w| w.to_string()))
            .collect()
    }

    fn run(env: &mut Environment, words: &[&str]) -> i32 {
        builtin_universal(env, &argv(words)).expect("universal never fails fatally")
    }

    /// Each test gets its own store. Through the module's own seam rather than an environment
    /// variable: writing the process environment from a test races every sibling thread.
    fn isolated() -> (std::sync::MutexGuard<'static, ()>, tempfile::TempDir) {
        universal::isolate_for_tests()
    }

    /// Setting one must take effect **here** as well as in the file, or the command looks like it
    /// did nothing until a new terminal is opened.
    #[test]
    fn setting_one_applies_to_this_shell_too() {
        let _store = isolated();
        let mut env = Environment::new();
        assert_eq!(run(&mut env, &["UTEST_THEME=dark"]), 0);
        assert_eq!(env.get_var("UTEST_THEME"), Some("dark"));
        assert_eq!(universal::load()["UTEST_THEME"].value, "dark");
    }

    /// Erasing removes it from both places, for the same reason.
    #[test]
    fn erasing_removes_it_here_and_in_the_file() {
        let _store = isolated();
        let mut env = Environment::new();
        run(&mut env, &["UTEST_GONE=1"]);
        assert_eq!(run(&mut env, &["-e", "UTEST_GONE"]), 0);
        assert_eq!(env.get_var("UTEST_GONE"), None);
        assert!(!universal::load().contains_key("UTEST_GONE"));
        // Erasing what was never there says so rather than succeeding quietly.
        assert_eq!(run(&mut env, &["-e", "UTEST_GONE"]), 1);
    }

    /// A bare name promotes the value this shell already has, which is what makes
    /// `EDITOR=hx; universal EDITOR` work the way it reads.
    #[test]
    fn a_bare_name_promotes_what_is_already_set() {
        let _store = isolated();
        let mut env = Environment::new();
        env.set_var("UTEST_EDITOR", "hx", false);
        assert_eq!(run(&mut env, &["UTEST_EDITOR"]), 0);
        assert_eq!(universal::load()["UTEST_EDITOR"].value, "hx");
        // A name with no value anywhere is an error, not an empty universal variable.
        assert_eq!(run(&mut env, &["UTEST_NOTHING"]), 1);
    }

    /// `-x` is the difference between "every shell sees it" and "every shell's children see it".
    #[test]
    fn the_export_flag_is_recorded() {
        let _store = isolated();
        let mut env = Environment::new();
        run(&mut env, &["-x", "UTEST_SHOWN=1"]);
        run(&mut env, &["UTEST_QUIET=1"]);
        let vars = universal::load();
        assert!(vars["UTEST_SHOWN"].exported);
        assert!(!vars["UTEST_QUIET"].exported);
    }

    /// A name a shell could not have assigned is refused rather than written to a file that every
    /// other shell will then try to load.
    #[test]
    fn an_invalid_name_is_refused() {
        let _store = isolated();
        let mut env = Environment::new();
        assert_eq!(run(&mut env, &["not-a-name=1"]), 1);
        assert!(!universal::load().contains_key("not-a-name"));
        assert_eq!(run(&mut env, &["-e"]), 2, "erase needs a name");
    }
}
