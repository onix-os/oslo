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
        "edit" => edit(rest),
        "run" => run_macro(rest),
        "publish" => publish(rest),
        "show" | "list" => list::show(rest),
        "off" => switch(rest, false),
        "on" => switch(rest, true),
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
    /// **`None` until a flag says.** Four kinds and a silent default is a trap: `add gs 'git status'`
    /// would quietly make an alias when the fingers meant an abbreviation.
    kind: Option<Kind>,
    edit: bool,
    plain: bool,
    replace: bool,
    session: bool,
    tags: Vec<String>,
    words: Vec<String>,
}

impl Asked {
    /// The kind, or the error that says the four names.
    fn kind(&self) -> Result<Kind, String> {
        self.kind
            .ok_or_else(|| "say which: --alias, --abbrev, --func or --script".to_string())
    }
}

fn parse(args: &[String]) -> Result<Asked, String> {
    let mut asked = Asked {
        kind: None,
        edit: false,
        plain: false,
        replace: false,
        session: false,
        tags: Vec::new(),
        words: Vec::new(),
    };
    let mut waiting_for_tag = false;
    for arg in args {
        if waiting_for_tag {
            waiting_for_tag = false;
            if !macros::valid_tag(arg) {
                return Err(format!("{arg:?} is not a tag: one word, no spaces"));
            }
            asked.tags.push(arg.to_string());
            continue;
        }
        let kind = match arg.as_str() {
            "--alias" => Some(Kind::Alias),
            "--abbrev" | "--abbr" => Some(Kind::Abbrev),
            "--func" | "--function" => Some(Kind::Func),
            "--script" => Some(Kind::Script),
            _ => None,
        };
        if let Some(kind) = kind {
            if asked.kind.is_some_and(|had| had != kind) {
                return Err("one kind at a time: --alias, --abbrev, --func or --script".to_string());
            }
            asked.kind = Some(kind);
            continue;
        }
        match arg.as_str() {
            "--edit" | "-e" => asked.edit = true,
            "--plain" => asked.plain = true,
            "--replace" => asked.replace = true,
            "--session" | "-s" => asked.session = true,
            "--tag" | "-t" => waiting_for_tag = true,
            // Only a *leading* dash is an option. A body is arbitrary text and often starts with
            // one — `oslo macros add --alias ll '-la'` is a thing somebody will write.
            other if other.starts_with("--") && asked.words.is_empty() => {
                return Err(format!("unknown option {other:?}"));
            }
            other => asked.words.push(other.to_string()),
        }
    }
    if waiting_for_tag {
        return Err("--tag needs a tag".to_string());
    }
    Ok(asked)
}

fn add(args: &[String]) -> i32 {
    let asked = match parse(args) {
        Ok(asked) => asked,
        Err(problem) => return usage(&problem),
    };
    let kind = match asked.kind() {
        Ok(kind) => kind,
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

    // **A function and a script are always written in the editor**, and an inline body for one is
    // refused rather than accepted: taking it would store a one-line function because that is what
    // fitted on the command line, which is how you end up with a function written as one line.
    let inline = asked.words[1..].join(" ");
    let editor_only = matches!(kind, Kind::Func | Kind::Script);
    if editor_only && !inline.is_empty() {
        return usage(&format!(
            "a {} is written in the editor: `oslo macros add --{} {name}` and no body",
            kind.word(),
            kind.word()
        ));
    }

    let body = if editor_only || asked.edit || inline.is_empty() {
        let starting = if inline.is_empty() {
            macros::get(&store, kind, &name)
                .map(|e| e.body)
                .unwrap_or_else(|| starter(kind, &name))
        } else {
            inline
        };
        match oslo_runtime::editor::edit(&starting, kind.extension(&starting)) {
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
    // The tags asked for, or — when none were — whatever it already had, so editing a macro does
    // not silently strip its labels.
    let tags = if asked.tags.is_empty() {
        macros::get(&store, kind, &name)
            .map(|old| old.tags)
            .unwrap_or_default()
    } else {
        asked.tags.clone()
    };
    let entry = Entry::new(kind, &name, &body).tagged(&tags);
    if let Err(problem) = macros::put_and_publish(&store, &entry) {
        return fail(&problem);
    }
    println!("{} {name}", kind.word());
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
    // **The kind is only asked for when the name is ambiguous.** `remove gs` is unambiguous
    // whenever one kind has that name, and making somebody name the kind to delete the only thing
    // called `gs` is a question with one possible answer.
    let kind = match asked.kind.or_else(|| {
        let only = macros::kinds_of(&store, name);
        (only.len() == 1).then(|| only[0])
    }) {
        Some(kind) => kind,
        None => match macros::kinds_of(&store, name).as_slice() {
            [] => return fail(&format!("nothing called {name}")),
            several => {
                let words: Vec<String> =
                    several.iter().map(|k| format!("--{}", k.word())).collect();
                return usage(&format!(
                    "{name} is more than one thing — say which: {}",
                    words.join(", ")
                ));
            }
        },
    };
    match macros::remove_and_publish(&store, kind, name) {
        Ok(true) => {
            println!("removed {} {name}", kind.word());
            0
        }
        Ok(false) => {
            let others = macros::kinds_of(&store, name);
            if others.is_empty() {
                fail(&format!("no {} called {name}", kind.word()))
            } else {
                let words: Vec<&str> = others.iter().map(|k| k.word()).collect();
                fail(&format!(
                    "no {} called {name} — it is a {} ({})",
                    kind.word(),
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

/// `publish` — write the derived copies again.
///
/// Every mutation does this already, so the only reason to ask is a directory that was deleted, a
/// `$PATH` that moved, or a database that was filled before the copies existed at all.
fn publish(_args: &[String]) -> i32 {
    let store = match macros::open() {
        Ok(store) => store,
        Err(problem) => return fail(&problem),
    };
    if let Err(problem) = macros::publish(&store) {
        return fail(&problem);
    }
    let scripts = macros::all(&store)
        .iter()
        .filter(|entry| entry.kind == Kind::Script && entry.active)
        .count();
    match macros::bin::directory() {
        Some(dir) => println!("{scripts} scripts written to {}", dir.display()),
        None => println!("{scripts} scripts"),
    }
    0
}

/// `run NAME [ARG]...` — run a stored macro from something that is not oslo.
///
/// **Everything after the name belongs to the macro**, including words that look like options:
/// `oslo macros run deploy --dry-run` passes `--dry-run` to `deploy` and not to oslo. That is the
/// same rule `-c` follows, and for the same reason.
fn run_macro(args: &[String]) -> i32 {
    let Some((name, rest)) = args.split_first() else {
        return usage("run needs a name: `oslo macros run NAME [ARG]...`");
    };
    let mut env = oslo::env::Environment::new();
    match oslo::exec::stored::run_named(&mut env, name, rest) {
        Some(status) => status,
        None => fail(&format!(
            "nothing runnable called {name} — `oslo macros show {name}` says what it is"
        )),
    }
}

/// `edit NAME` — open one in `$EDITOR` and store what comes back.
///
/// `add` already opens the editor for a name it knows, but only by way of adding one; asking to
/// *edit* what you have should not be spelled as adding it again.
fn edit(args: &[String]) -> i32 {
    let asked = match parse(args) {
        Ok(asked) => asked,
        Err(problem) => return usage(&problem),
    };
    let Some(name) = asked.words.first() else {
        return usage("edit needs a name");
    };
    let store = match macros::open() {
        Ok(store) => store,
        Err(problem) => return fail(&problem),
    };
    // The kind is only asked for when the name is more than one thing, exactly as `remove` does it.
    let kind = match asked
        .kind
        .or_else(|| match macros::kinds_of(&store, name).as_slice() {
            [only] => Some(*only),
            _ => None,
        }) {
        Some(kind) => kind,
        None => match macros::kinds_of(&store, name).as_slice() {
            [] => return fail(&format!("nothing called {name}")),
            several => {
                let words: Vec<String> =
                    several.iter().map(|k| format!("--{}", k.word())).collect();
                return usage(&format!(
                    "{name} is more than one thing — say which: {}",
                    words.join(", ")
                ));
            }
        },
    };
    let Some(entry) = macros::get(&store, kind, name) else {
        return fail(&format!("no {} called {name}", kind.word()));
    };

    // **Said before the editor opens, not after it closes.** A function or a script is found only
    // once `$PATH` has failed, so a file of the same name on `$PATH` is what actually runs — and
    // editing the stored one would be editing something the shell never reaches. Whoever asked for
    // this should know that while they still have the choice of editing the other file instead.
    if let Some(path) = shadowing_file(kind, name) {
        eprintln!("oslo macros: {name} runs from {path} — this is the stored copy, and `$PATH`");
        eprintln!(
            "oslo macros: wins over it. Edit that file, or take it off `$PATH`, if you meant"
        );
        eprintln!("oslo macros: the one that runs.");
    }

    match oslo_runtime::editor::edit(&entry.body, kind.extension(&entry.body)) {
        Ok(None) => {
            println!("unchanged");
            0
        }
        Ok(Some(body)) => match macros::put_and_publish(&store, &Entry { body, ..entry }) {
            Ok(()) => {
                println!("{} {name}", kind.word());
                0
            }
            Err(problem) => fail(&problem),
        },
        Err(problem) => fail(&problem),
    }
}

/// The file on `$PATH` that would answer to `name` before a stored macro does.
///
/// Only for the kinds that are *found by name* when a command runs. An alias and an abbreviation
/// are applied before the search happens, so nothing on `$PATH` can shadow one.
fn shadowing_file(kind: Kind, name: &str) -> Option<String> {
    if !matches!(kind, Kind::Func | Kind::Script) {
        return None;
    }
    oslo::env::builtins::hash_lookup(name).map(|path| path.to_string_lossy().into_owned())
}

/// `off` and `on` — the screen's Space and Space ×3, for something that is not a person.
///
/// Both switches have a spelling here because a manager only a person can drive is one you cannot
/// put in a script, and because "off for this session" is the one thing a *hook* might want to say.
fn switch(args: &[String], on: bool) -> i32 {
    let asked = match parse(args) {
        Ok(asked) => asked,
        Err(problem) => return usage(&problem),
    };
    let Some(name) = asked.words.first() else {
        return usage(&format!("{} needs a name", if on { "on" } else { "off" }));
    };
    if asked.session {
        // The session this process is in, which for anything a shell started is the *shell's* —
        // see `$OSLO_SESSION` in `env::scope::seed`.
        let session = oslo::track::session::id();
        return match macros::live::session::set(&session, name, !on) {
            Ok(()) => {
                println!(
                    "{name} is {} in this session",
                    if on { "on" } else { "off" }
                );
                0
            }
            Err(problem) => fail(&problem),
        };
    }

    let store = match macros::open() {
        Ok(store) => store,
        Err(problem) => return fail(&problem),
    };
    let kinds = match asked.kind {
        Some(kind) => vec![kind],
        None => macros::kinds_of(&store, name),
    };
    if kinds.is_empty() {
        return fail(&format!("nothing called {name}"));
    }
    for kind in kinds {
        if macros::set_active(&store, kind, name, on).is_none() {
            return fail(&format!("no {} called {name}", kind.word()));
        }
    }
    if let Err(problem) = macros::publish(&store) {
        return fail(&problem);
    }
    println!("{name} is {}", if on { "on" } else { "off" });
    0
}

#[cfg(test)]
#[path = "macros/tests.rs"]
mod tests;
