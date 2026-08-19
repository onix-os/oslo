//! `oslo direnv` — the decisions about what a directory may do, from outside a session.
//!
//! # Why this needed a command
//!
//! The main menu has advertised "manage per-directory environments" since the tool list was
//! written, and the tool answered "none yet — this tool is not implemented".
//!
//! The `direnv` **builtin** answers the same questions at an oslo prompt, and cannot answer them
//! anywhere else: it refuses outright in a non-interactive shell, because the state it reads
//! belongs to a session. But the *decisions* — allowed, denied — are not session state. They are
//! tokens on disk, and everything that is not a running shell has a reason to reach them: a
//! provisioning script allowing a checkout it just made, a CI job refusing one, or you, from
//! another shell, wondering why a directory is inert.
//!
//! # What is here and what is not
//!
//! `reload` and `edit` are the builtin's and stay there. Reloading means running `.env.lua` against
//! a live environment, and editing ends by telling the session to notice — neither has a meaning in
//! a process that has no session and exits. Asking for them here says so rather than pretending.

use crate::cli::help::Paint;
use crate::cli::help::menu::{CALL, Menu, SUBCOMMANDS as HEADING, Sub};
use oslo::direnv::allow::{Allow, Status};
use oslo::direnv::find;
use std::path::{Path, PathBuf};

pub(crate) const MENU: Menu = Menu {
    path: &["direnv"],
    call: CALL,
    heading: HEADING,
    subs: SUBCOMMANDS,
    notes: &[
        "oslo direnv status",
        "oslo direnv allow ~/work/project",
        "A directory's `.env.lua` is inert until allowed. Allowing is a statement about the file's \
         *contents*, so editing it revokes the allowance; denying is a statement about the path, \
         and survives any edit.",
    ],
    nested: &[],
};

const SUBCOMMANDS: &[Sub] = &[
    Sub {
        name: "status",
        args: "[PATH]",
        about: "the file that applies here, and whether it may run",
        flags: &[],
        note: "With no PATH, the file that applies to the working directory — found by walking up, \
               exactly as a shell arriving here would.",
    },
    Sub {
        name: "allow",
        args: "[PATH]",
        about: "let it run, as it stands now",
        flags: &[],
        note: "About the *contents*: editing the file afterwards revokes this, which is the whole \
               point of recording a hash rather than a path.",
    },
    Sub {
        name: "deny",
        args: "[PATH]",
        about: "refuse it, whatever it becomes",
        flags: &[],
        note: "About the *path*, so a directory you have refused cannot come back by being edited.",
    },
    Sub {
        name: "prune",
        args: "",
        about: "drop every decision, allowed and denied",
        flags: &[],
        note: "All of them, and it says how many. The tokens are empty files named by hash, so \
               there is nothing in the store to tell a stale decision from a live one — a sweep \
               that claimed to be selective would be guessing.",
    },
];

pub fn run(args: &[String]) -> i32 {
    if let Some(page) = MENU.asked(args, Paint::detect()) {
        print!("{page}");
        return 0;
    }
    match args.first().map(String::as_str) {
        None => {
            print!("{}", MENU.overview(Paint::detect()));
            0
        }
        // Each of these reads one path at most; a second was silently dropped, so a line naming
        // two directories allowed one of them and said nothing about the other.
        Some("status") => MENU
            .extra("status", args, 1)
            .unwrap_or_else(|| status(args.get(1))),
        Some("allow") => MENU
            .extra("allow", args, 1)
            .unwrap_or_else(|| decide(args.get(1), true)),
        Some("deny") => MENU
            .extra("deny", args, 1)
            .unwrap_or_else(|| decide(args.get(1), false)),
        Some("prune") => MENU.extra("prune", args, 0).unwrap_or_else(prune),
        // Named rather than lumped in with a typo: somebody asking for these has the right idea and
        // the wrong process, and the answer is where to go, not "no such subcommand".
        Some(session @ ("reload" | "edit")) => {
            eprintln!(
                "oslo direnv: {session} needs a running shell; type `direnv {session}` at an oslo prompt"
            );
            2
        }
        Some(other) => MENU.unknown(other),
    }
}

/// The store, resolved from this process's environment exactly as a shell resolves it.
fn store() -> Allow {
    Allow::new(
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// The rc file an argument names, or the one that applies to the working directory.
///
/// A directory argument is walked up from, like an arrival; a file argument is taken as itself, so
/// `oslo direnv allow ./.env.lua` means that file.
fn target(argument: Option<&String>) -> Option<PathBuf> {
    let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let Some(argument) = argument else {
        return find::applicable(&here).map(|rc| rc.path);
    };
    let path = Path::new(argument);
    if path.is_file() {
        return Some(path.components().collect());
    }
    find::applicable(path)
        .map(|rc| rc.path)
        .or_else(|| path.is_dir().then(|| path.components().collect()))
}

fn status(argument: Option<&String>) -> i32 {
    let paint = Paint::detect();
    let allow = store();
    let (allowed, denied) = allow.count();
    println!("{}", paint.head("HERE"));
    match target(argument) {
        None => println!("  {}", paint.dim("no directory file applies")),
        Some(path) => {
            println!("  {}", paint.key(&path.display().to_string()));
            let said = match allow.status(&path) {
                Status::Allowed => "allowed",
                Status::Denied => "denied — it will not run, whatever it says",
                Status::NotAllowed => "not allowed — run `oslo direnv allow`",
            };
            println!("  {}", paint.dim(said));
        }
    }
    println!();
    println!("{}", paint.head("DECISIONS"));
    println!(
        "  {}",
        paint.dim(&format!("{allowed} allowed, {denied} denied"))
    );
    0
}

fn decide(argument: Option<&String>, permit: bool) -> i32 {
    let Some(path) = target(argument) else {
        eprintln!("oslo direnv: no directory file here to decide about");
        return 1;
    };
    let allow = store();
    let outcome = match permit {
        true => allow.allow(&path),
        false => allow.deny(&path),
    };
    match outcome {
        Ok(()) => {
            let verb = if permit { "allowed" } else { "denied" };
            println!("{verb}: {}", path.display());
            // A shell already sitting in that directory read the file — or refused to — when it
            // arrived, and nothing here can reach into it. Saying so beats leaving somebody to
            // wonder why their prompt has not changed.
            println!(
                "{}",
                Paint::detect().dim("a shell already there picks this up on its next arrival")
            );
            0
        }
        Err(why) => {
            eprintln!("oslo direnv: {why}");
            1
        }
    }
}

fn prune() -> i32 {
    let dropped = store().prune();
    match dropped {
        0 => println!("no decisions to drop"),
        n => println!("dropped {n} decision(s); every directory file must be allowed again"),
    }
    0
}
