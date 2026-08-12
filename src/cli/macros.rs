//! `oslo macros` — the small named things you keep, in one place.
//!
//! The rules live in [`oslo_base::macros`]; this parses words, opens an editor and prints. Every
//! mutation republishes the snapshot a starting shell reads, so the database and the file cannot
//! drift apart by anything short of editing the file by hand.

pub mod help;
mod list;

use crate::cli::help::Paint;
use oslo::macros::{self, Entry, Kind};

pub fn run(args: &[String]) -> i32 {
    let Some(command) = args.first().map(String::as_str) else {
        print!("{}", help::text(Paint::detect()));
        return 2;
    };
    if matches!(command, "-h" | "--help" | "help") {
        print!("{}", help::text(Paint::detect()));
        return 0;
    }
    let rest = &args[1..];
    // Handled before the subcommand parses its own arguments, the way `history` and `plugin` do it.
    if rest
        .first()
        .is_some_and(|a| matches!(a.as_str(), "-h" | "--help"))
        && let Some(text) = help::subcommand(command, Paint::detect())
    {
        print!("{text}");
        return 0;
    }
    match command {
        "add" => add(rest),
        "remove" | "rm" => remove(rest),
        "show" | "list" => list::show(rest),
        "export" => list::export(rest),
        "import" => list::import(rest),
        other => usage(&format!("unknown subcommand {other:?}")),
    }
}

fn usage(message: &str) -> i32 {
    eprintln!("oslo macros: {message}\n");
    eprint!("{}", help::text(Paint::plain()));
    2
}

fn fail(message: &str) -> i32 {
    eprintln!("oslo macros: {message}");
    1
}

/// The kind a flag asked for, and the words that were not flags.
#[derive(Debug)]
struct Asked {
    kind: Kind,
    edit: bool,
    plain: bool,
    replace: bool,
    words: Vec<String>,
}

fn parse(args: &[String]) -> Result<Asked, String> {
    let mut asked = Asked {
        kind: Kind::Alias,
        edit: false,
        plain: false,
        replace: false,
        words: Vec::new(),
    };
    let mut kind_given = false;
    for arg in args {
        let kind = match arg.as_str() {
            "--abbrev" | "--abbr" => Some(Kind::Abbrev),
            "--func" | "--function" => Some(Kind::Func),
            "--script" => Some(Kind::Script),
            _ => None,
        };
        if let Some(kind) = kind {
            if kind_given {
                return Err("one kind at a time: --abbrev, --func or --script".to_string());
            }
            kind_given = true;
            asked.kind = kind;
            continue;
        }
        match arg.as_str() {
            "--edit" | "-e" => asked.edit = true,
            "--plain" => asked.plain = true,
            "--replace" => asked.replace = true,
            // Only a *leading* dash is an option. A body is arbitrary text and often starts with
            // one — `oslo macros add ll '-la'` is a thing somebody will write.
            other if other.starts_with("--") && asked.words.is_empty() => {
                return Err(format!("unknown option {other:?}"));
            }
            other => asked.words.push(other.to_string()),
        }
    }
    Ok(asked)
}

fn add(args: &[String]) -> i32 {
    let asked = match parse(args) {
        Ok(asked) => asked,
        Err(problem) => return usage(&problem),
    };
    let Some(name) = asked.words.first().cloned() else {
        return usage("add needs a name");
    };
    if !macros::valid_name(&name) {
        return fail(&format!(
            "{name:?} is not a name: it becomes a command word, so no spaces and nothing that \
             could be an operator"
        ));
    }

    let store = match macros::open() {
        Ok(store) => store,
        Err(problem) => return fail(&problem),
    };

    // A body on the command line, or the editor. `--func` and `--script` always take the editor:
    // neither fits on one line, and pretending otherwise invites a function written as one.
    let inline = asked.words[1..].join(" ");
    let wants_editor =
        asked.edit || inline.is_empty() || matches!(asked.kind, Kind::Func | Kind::Script);
    let body = if wants_editor {
        let starting = if inline.is_empty() {
            macros::get(&store, asked.kind, &name)
                .map(|e| e.body)
                .unwrap_or_else(|| starter(asked.kind, &name))
        } else {
            inline
        };
        match super::editor::edit(&starting, asked.kind.extension(&starting)) {
            Ok(Some(body)) => body,
            Ok(None) => {
                println!("unchanged");
                return 0;
            }
            Err(problem) => return fail(&problem),
        }
    } else {
        inline
    };

    if body.trim().is_empty() {
        return fail("nothing to store: the body is empty");
    }
    let entry = Entry {
        kind: asked.kind,
        name: name.clone(),
        body,
    };
    if let Err(problem) = macros::put_and_publish(&store, &entry) {
        return fail(&problem);
    }
    println!("{} {name}", asked.kind.word());
    0
}

/// What an empty function or script starts life as, so the editor opens on something rather than
/// on a blank buffer nobody knows the shape of.
fn starter(kind: Kind, name: &str) -> String {
    match kind {
        Kind::Func => {
            format!("# {name} — a shell function. The body runs with \"$@\" as its arguments.\n\n")
        }
        Kind::Script => {
            format!("#!/bin/sh\n# {name} — a script. Change the shebang for another language.\n\n")
        }
        _ => String::new(),
    }
}

fn remove(args: &[String]) -> i32 {
    let asked = match parse(args) {
        Ok(asked) => asked,
        Err(problem) => return usage(&problem),
    };
    let Some(name) = asked.words.first() else {
        return usage("remove needs a name");
    };
    let store = match macros::open() {
        Ok(store) => store,
        Err(problem) => return fail(&problem),
    };
    match macros::remove_and_publish(&store, asked.kind, name) {
        Ok(true) => {
            println!("removed {} {name}", asked.kind.word());
            0
        }
        Ok(false) => {
            let others = macros::kinds_of(&store, name);
            if others.is_empty() {
                fail(&format!("no {} called {name}", asked.kind.word()))
            } else {
                let words: Vec<&str> = others.iter().map(|k| k.word()).collect();
                fail(&format!(
                    "no {} called {name} — it is a {} ({})",
                    asked.kind.word(),
                    words.join(" and a "),
                    words
                        .iter()
                        .map(|w| format!("--{w}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        }
        Err(problem) => fail(&problem),
    }
}

#[cfg(test)]
#[path = "macros/tests.rs"]
mod tests;
