//! `mark` — remember the directory you are standing in as `@name`, and forget it by typing it again.
//!
//! ```console
//! ~/data/code/tools/rush $ mark
//! marked @rush
//! ~/data/code/tools/rush $ cd /tmp && ls @rush/crates
//! ~/data/code/tools/rush $ mark
//! unmarked @rush
//! ```
//!
//! **A toggle, because the question is about one directory and has two answers.** A pair of verbs
//! would be two names to remember for a thing you do without thinking, and every mark is made and
//! unmade from inside the directory it is about — so the shell already knows which one you mean.
//!
//! The name is the directory's own last component. That is what somebody typing `@rush` a week
//! later will reach for, and choosing it for them is what makes the bare `mark` worth having;
//! `mark NAME` is there for when it is not.
//!
//! **A word rather than the `@` sigil**, though `@name` is what it writes. A leading symbol is
//! reserved for something else, so no builtin may take the start of a line.

use crate::env::Environment;
use crate::env::origin_now;
use oslo_base::dirs;
use oslo_base::error::Result;

pub fn builtin_mark(env: &mut Environment, args: &[String]) -> Result<i32> {
    // `args[0]` is the name the builtin was reached by, as it is for every builtin here.
    let args = args.get(1..).unwrap_or(&[]);
    match args.first().map(String::as_str) {
        Some("-h" | "--help") => {
            println!("{USAGE}");
            Ok(0)
        }
        Some("-l" | "--list") => {
            list();
            Ok(0)
        }
        Some("-d" | "--delete") => match args.get(1) {
            Some(name) => remove(name),
            None => {
                eprintln!("{}mark: -d takes the name of a mark", origin_now());
                Ok(2)
            }
        },
        Some(flag) if flag.starts_with('-') => {
            eprintln!("{}mark: {flag}: not an option\n{USAGE}", origin_now());
            Ok(2)
        }
        chosen => toggle(env, chosen),
    }
}

const USAGE: &str = "\
usage: mark              mark this directory, or unmark it if it is marked
       mark NAME         the same, under a name you choose
       mark -l           list every mark
       mark -d NAME      forget a mark from anywhere";

/// Mark the current directory, or unmark it if this shell already knows it.
///
/// **The path decides, not the name.** Typing `mark` twice in one directory has to undo itself
/// whatever the mark ended up called — including a mark made earlier under `mark othername`, which a
/// name-keyed toggle would leave behind and then refuse to replace.
fn toggle(env: &mut Environment, chosen: Option<&str>) -> Result<i32> {
    let Some(here) = current_directory(env) else {
        eprintln!("{}mark: cannot tell which directory this is", origin_now());
        return Ok(1);
    };
    if let Some(existing) = dirs::mark_of(&here) {
        // A different name for a directory already marked is a rename, not a second mark: two names
        // for one path is a thing the listing cannot explain and `@` could never undo.
        if let Some(name) = chosen.filter(|name| *name != existing) {
            return match dirs::unmark(&existing).and_then(|_| dirs::mark(name, &here)) {
                Ok(()) => {
                    println!("marked @{name} (was @{existing})");
                    Ok(0)
                }
                Err(problem) => {
                    eprintln!("{}mark: {problem}", origin_now());
                    Ok(1)
                }
            };
        }
        return match dirs::unmark(&existing) {
            Ok(_) => {
                println!("unmarked @{existing}");
                Ok(0)
            }
            Err(problem) => {
                eprintln!("{}mark: {problem}", origin_now());
                Ok(1)
            }
        };
    }

    let name = match chosen {
        Some(name) => name.to_string(),
        None => match basename(&here) {
            Some(name) => name,
            None => {
                eprintln!(
                    "{}mark: {here} has no name to mark it by; `mark NAME` chooses one",
                    origin_now()
                );
                return Ok(1);
            }
        },
    };
    // **A name already meaning somewhere else is refused, not overwritten.** The bare `mark` picked
    // this name rather than being told it, so silently moving somebody's `@src` because they walked
    // into a second `src` is the one outcome nobody asked for.
    if let Some(taken) = dirs::named_dir(&name)
        && taken != here
    {
        eprintln!(
            "oslo: mark: @{name} is already {taken}; `mark NAME` marks this one under another name"
        );
        return Ok(1);
    }
    match dirs::mark(&name, &here) {
        Ok(()) => {
            println!("marked @{name}");
            Ok(0)
        }
        Err(problem) => {
            eprintln!("{}mark: {problem}", origin_now());
            Ok(1)
        }
    }
}

fn remove(name: &str) -> Result<i32> {
    match dirs::unmark(name) {
        Ok(true) => {
            println!("unmarked @{name}");
            Ok(0)
        }
        Ok(false) => {
            eprintln!("{}mark: @{name} is not marked", origin_now());
            Ok(1)
        }
        Err(problem) => {
            eprintln!("{}mark: {problem}", origin_now());
            Ok(1)
        }
    }
}

/// Every name and where it goes, the declared ones included — `@name` reaches both, so a listing
/// that showed only the marked half would be answering a narrower question than the sigil does.
fn list() {
    let all = dirs::named_dirs();
    let width = all.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
    for (name, path) in all {
        println!("@{name:<width$}  {path}", width = width);
    }
}

/// Where the shell thinks it is, preferring `$PWD` so a path reached through a symlink is marked
/// the way it was walked rather than the way it resolves.
fn current_directory(env: &Environment) -> Option<String> {
    env.get_var("PWD")
        .map(str::to_string)
        .filter(|pwd| pwd.starts_with('/'))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
        })
}

/// The last component, which is what a person would call this directory.
fn basename(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    let name = trimmed.rsplit('/').next()?;
    dirs::valid_mark_name(name).then(|| name.to_string())
}

#[cfg(test)]
mod tests;
