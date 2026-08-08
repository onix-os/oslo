//! `direnv` — the command that decides what a directory is allowed to do.
//!
//! Names and aliases follow the real direnv, so muscle memory and every README that says
//! `direnv allow` keep working: `allow` (`permit`, `grant`), `deny` (`block`, `revoke`), `reload`,
//! `status`, `prune`, `edit`.
//!
//! With no argument these act on the current directory, as direnv's do.

use crate::direnv::{self, allow::Status, find};
use crate::env::Environment;
use crate::error::Result;
use std::path::{Path, PathBuf};

pub fn builtin_direnv(env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(action) = args.get(1) else {
        return Ok(usage());
    };
    // A shell with no directory environment is a script, and every one of these would be answering
    // about state that does not exist. Said plainly rather than silently succeeding.
    if !direnv::installed() {
        eprintln!("direnv: not available in a non-interactive shell");
        return Ok(1);
    }

    match action.as_str() {
        "allow" | "permit" | "grant" => Ok(permit(args.get(2), true)),
        "deny" | "block" | "revoke" => Ok(permit(args.get(2), false)),
        "reload" => Ok(reload(env)),
        "status" => Ok(status()),
        "prune" => Ok(prune()),
        "edit" => Ok(edit(env, args.get(2))),
        other => {
            eprintln!("direnv: {other}: not a direnv command");
            Ok(usage())
        }
    }
}

fn usage() -> i32 {
    eprintln!(
        "usage: direnv allow|deny|reload|status|prune|edit [path]\n\
         \n\
         A directory's .env.lua is inert until allowed. Allowing is a statement about the file's\n\
         *contents*, so editing it revokes the allowance; denying is a statement about the path,\n\
         and survives any edit."
    );
    2
}

/// The file a path argument refers to, or the one that applies here.
fn target(argument: Option<&String>) -> Option<PathBuf> {
    let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let Some(argument) = argument else {
        return find::applicable(&here).map(|rc| rc.path);
    };
    let path = Path::new(argument);
    // `components()` collapses the `./` in `direnv allow ./.envrc`, which is otherwise carried into
    // every message about the path.
    let path: PathBuf = if path.is_absolute() {
        path.components().collect()
    } else {
        here.join(path).components().collect()
    };
    // A directory names its file; a file names itself.
    if path.is_dir() {
        return find::applicable(&path).map(|rc| rc.path);
    }
    Some(path)
}

fn permit(argument: Option<&String>, allow: bool) -> i32 {
    let Some(path) = target(argument) else {
        eprintln!("direnv: no .env.lua or .envrc here or above");
        return 1;
    };
    // **Only the governing file may be allowed.** A directory with both names reads one and ignores
    // the other, so a record for the ignored one grants nothing — it would sit in the store looking
    // like a decision that had been taken while the file went on not running.
    if let Some(governs) = find::governed_by(&path) {
        eprintln!(
            "direnv: {} is ignored here — {} governs this directory",
            path.display(),
            governs.display()
        );
        return 1;
    }
    let path = &path;
    let mut status = 0;
    {
        let outcome = direnv::with(|direnv| {
            let store = direnv.permissions();
            let result = if allow {
                store.allow(path)
            } else {
                store.deny(path)
            };
            // The "you need to allow this" notice is printed once per path. After a decision that
            // changes the answer, it has to be sayable again.
            direnv.tell_again(path);
            // Either way the loaded state is now stale: an allow means there is something to load,
            // a deny means what is loaded should not be. The next prompt does the work.
            direnv.invalidate();
            direnv::request_reload();
            result
        });
        match outcome {
            Some(Ok(())) => {
                let verb = if allow { "allowed" } else { "denied" };
                println!("direnv: {verb} {}", path.display());
            }
            Some(Err(problem)) => {
                eprintln!("direnv: {}: {problem}", path.display());
                status = 1;
            }
            None => status = 1,
        }
    }
    status
}

/// Drop what is loaded so the next prompt loads it again from scratch.
///
/// Deliberately does *not* unload the environment here: unloading needs the same before/after
/// window the loader uses, and doing half of it from a builtin would leave the shell holding
/// variables with no record of how to remove them. Forgetting is enough — the next directory check
/// finds no loaded state, so it loads, and the load takes its own snapshot.
fn reload(_env: &mut Environment) -> i32 {
    // The remembered dev-shell evaluation too. It is keyed on the flake's own files, so `reload` is
    // the only way to say "something it cannot see has changed" — a new nix, or an input the flake
    // reads that is not one of the files being watched.
    direnv::devshell::forget();
    direnv::with(|direnv| direnv.invalidate());
    direnv::request_reload();
    println!("direnv: reloading");
    0
}

fn status() -> i32 {
    let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    direnv::with(|direnv| {
        match direnv.active() {
            Some(owner) => {
                let held = direnv.holding();
                println!("loaded: {}", owner.display());
                println!("holding {} variable(s): {}", held.len(), held.join(" "));
            }
            None => println!("loaded: nothing"),
        }
        match find::applicable(&here) {
            None => println!("found: nothing here or above"),
            Some(rc) => {
                let state = match direnv.permissions().status(&rc.path) {
                    Status::Allowed => "allowed",
                    Status::Denied => "denied",
                    Status::NotAllowed => "not allowed — run `direnv allow`",
                };
                println!("found: {} ({state})", rc.path.display());
                // Said out loud, because being ignored looks exactly like working until you notice
                // that nothing the file sets is set.
                if let Some(dir) = find::owner(&rc) {
                    for other in find::shadowed(&dir) {
                        println!(
                            "ignored: {} ({} governs this directory)",
                            other.display(),
                            rc.path.display()
                        );
                    }
                }
            }
        }
        let (allowed, denied) = direnv.permissions().count();
        println!("remembered: {allowed} allowed, {denied} denied");
    });
    0
}

fn prune() -> i32 {
    match direnv::with(|direnv| direnv.permissions().prune()) {
        Some(dropped) => {
            // Announced as the blunt instrument it is. direnv's prune keeps the tokens whose file
            // still exists because it stores the path inside them; ours are empty by design, so
            // there is nothing to check them against and pretending otherwise would be a lie.
            println!(
                "direnv: dropped all {dropped} decision(s); every rc file must be allowed again"
            );
            0
        }
        None => 1,
    }
}

fn edit(env: &mut Environment, argument: Option<&String>) -> i32 {
    let Some(path) = target(argument) else {
        eprintln!("direnv: no .env.lua or .envrc here or above");
        return 1;
    };
    let path = &path;
    let editor = env
        .get_var("VISUAL")
        .or_else(|| env.get_var("EDITOR"))
        .unwrap_or("vi")
        .to_string();
    let status = std::process::Command::new(&editor).arg(path).status();
    match status {
        Ok(_) => {
            // The file has almost certainly changed, which revokes its allowance by design. Saying
            // so here saves the next `cd` being a confusing refusal.
            direnv::with(|direnv| {
                direnv.tell_again(path);
                direnv.invalidate();
                direnv::request_reload();
            });
            println!(
                "direnv: {} edited; run `direnv allow` to trust it",
                path.display()
            );
            0
        }
        Err(problem) => {
            eprintln!("direnv: {editor}: {problem}");
            1
        }
    }
}
