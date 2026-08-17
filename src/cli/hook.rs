//! `oslo hook` — what a config can attach to, and whether yours did.
//!
//! # Why this needed a command
//!
//! The main menu has advertised "list and test the shell hooks" since the tool list was written,
//! and the tool answered "none yet — this tool is not implemented". That is the worst of both: a
//! promise in the help nothing keeps, and no way to answer the question the promise implies.
//!
//! The question is a real one. `oslo.on.precmb(f)` is a typo, and a typo attaches to nothing and
//! says nothing — which is indistinguishable from a hook that fires and does not work. `list` shows
//! every name that exists and marks the ones your configuration reached; `test` fires one and shows
//! what happened.
//!
//! # A tool rather than a builtin, and it loads your config to answer
//!
//! Like [`crate::cli::config`], this is a *different process* from your session: it reproduces the
//! load rather than asking the running shell, so what it reports is what a **new** session would
//! see. A shell that could be interrogated from outside would be a stranger thing than one that
//! can be reproduced.

use crate::cli::help::Paint;
use crate::cli::help::menu::{CALL, Menu, SUBCOMMANDS as HEADING, Sub};
use oslo_runtime::lua::api::hooks::{HOOKS, resolve, watched};

pub(crate) const MENU: Menu = Menu {
    path: &["hook"],
    call: CALL,
    heading: HEADING,
    subs: SUBCOMMANDS,
    notes: &[
        "oslo hook list --attached",
        "oslo hook test post-change-dir from=/tmp to=/home",
    ],
    nested: &[],
};

const SUBCOMMANDS: &[Sub] = &[
    Sub {
        name: "list",
        args: "",
        about: "every moment a config can attach to",
        flags: &[("--attached", "only the ones your configuration reached")],
        note: "The mark in the first column says your configuration attached something. A name \
               with nothing on it is the ordinary case; a name you *meant* to attach to with \
               nothing on it is a misspelling, and it is the one this exists to make visible.",
    },
    Sub {
        name: "show",
        args: "NAME",
        about: "one hook: its other spellings, and whether it answers",
        flags: &[],
        note: "Any spelling reaches it — `preexec`, `pre_cmd` and `pre-cmd` are one moment.",
    },
    Sub {
        name: "test",
        args: "NAME [FIELD=VALUE]...",
        about: "fire it against your configuration and report what ran",
        flags: &[],
        note: "Your `init.lua` is loaded into *this* process and the hook is fired there, so a \
               handler that prints prints here. Nothing of your session is touched, and a hook \
               that changes the shell changes this one.",
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
        Some("list") => list(args.iter().any(|a| a == "--attached")),
        Some("show") => match args.get(1) {
            Some(name) => show(name),
            None => MENU.missing("show needs a hook name, as in `oslo hook show pre-cmd`"),
        },
        Some("test") => match args.get(1) {
            Some(name) => test(name, &args[2..]),
            None => MENU.missing("test needs a hook name, as in `oslo hook test pre-prompt`"),
        },
        Some(other) => MENU.unknown(other),
    }
}

/// Load the configuration into this process, so the questions below have something to answer about.
///
/// Answers whether an interpreter exists at all: without one, `watched` is false for everything and
/// saying "nothing is attached" would be a lie about the config rather than a fact about it.
fn with_config() -> Option<oslo_runtime::LuaEngine> {
    let env = oslo::env::Environment::new();
    let files = oslo_runtime::startup::lua_init::config_files(&env);
    let engine = oslo_runtime::LuaEngine::new().ok()?;
    let shared = std::sync::Arc::new(std::sync::Mutex::new(env));
    if !oslo_runtime::startup::lua_init::install_bindings(&engine, shared) {
        return None;
    }
    for path in &files {
        oslo_runtime::startup::lua_init::load_config(&engine, path);
    }
    Some(engine)
}

fn list(attached_only: bool) -> i32 {
    let paint = Paint::detect();
    let loaded = with_config();
    let mut shown = 0;
    println!("{}", paint.head("HOOKS"));
    for (index, hook) in HOOKS.iter().enumerate() {
        let attached = loaded.is_some() && watched(index);
        if attached_only && !attached {
            continue;
        }
        shown += 1;
        let mark = if attached { "●" } else { " " };
        let row = match hook.answers {
            true => format!("{:<22} {}", paint.key(hook.name), paint.dim("answers")),
            false => paint.key(hook.name),
        };
        println!("  {mark} {row}");
    }
    if shown == 0 {
        println!("  {}", paint.dim("nothing attached"));
    }
    println!();
    match loaded.is_some() {
        true => println!(
            "  {}",
            paint.dim("● your configuration attached something here")
        ),
        false => println!(
            "  {}",
            paint.dim("no Lua interpreter, so nothing could be attached")
        ),
    }
    0
}

fn show(name: &str) -> i32 {
    let paint = Paint::detect();
    let Some((index, canonical)) = resolve(name) else {
        return unknown_hook(name);
    };
    let hook = &HOOKS[index];
    let loaded = with_config();
    println!("{}", paint.head("HOOK"));
    println!("  {}", paint.key(canonical));
    println!();
    println!("{}", paint.head("SPELLINGS"));
    let underscored = canonical.replace('-', "_");
    let mut spellings: Vec<String> = vec![canonical.to_string()];
    if underscored != canonical {
        spellings.push(underscored);
    }
    spellings.extend(hook.aliases.iter().map(|a| a.to_string()));
    for spelling in &spellings {
        println!("  oslo.on[\"{spelling}\"](f)");
    }
    println!();
    println!("{}", paint.head("ANSWERS"));
    match hook.answers {
        true => println!(
            "  {}",
            paint
                .dim("yes — the first handler to return a value decides, and the rest are skipped")
        ),
        false => println!(
            "  {}",
            paint.dim("no — every handler runs and what they return is discarded")
        ),
    }
    println!();
    println!("{}", paint.head("YOUR CONFIGURATION"));
    match (loaded.is_some(), watched(index)) {
        (false, _) => println!("  {}", paint.dim("no Lua interpreter")),
        (true, true) => println!("  {}", paint.dim("something is attached")),
        (true, false) => println!("  {}", paint.dim("nothing is attached")),
    }
    0
}

fn test(name: &str, fields: &[String]) -> i32 {
    let paint = Paint::detect();
    let Some((index, canonical)) = resolve(name) else {
        return unknown_hook(name);
    };
    if with_config().is_none() {
        eprintln!("oslo hook: no Lua interpreter, so there is nothing to fire");
        return 1;
    }
    if !watched(index) {
        println!(
            "{}",
            paint.dim(&format!(
                "nothing is attached to {canonical}; firing it anyway"
            ))
        );
    }
    // Split here rather than in the parser: a value may contain `=` and only the first one
    // separates, which is the same rule an assignment follows.
    let pairs: Vec<(&str, &str)> = fields
        .iter()
        .map(|field| match field.split_once('=') {
            Some((key, value)) => (key, value),
            None => (field.as_str(), ""),
        })
        .collect();
    println!("{}", paint.head("FIRING"));
    println!("  {}", paint.key(canonical));
    for (key, value) in &pairs {
        println!("    {key} = {value:?}");
    }
    println!();
    println!("{}", paint.head("OUTPUT"));
    let answered = match HOOKS[index].answers {
        true => oslo_base::hooks::answer_hook_with(
            index,
            vec![oslo_runtime::LuaEngine::hook_fields(
                &pairs
                    .iter()
                    .map(|(k, v)| (*k, oslo_base::value::Value::str(v)))
                    .collect::<Vec<_>>(),
            )],
        ),
        false => {
            oslo_base::hooks::fire_at_here(index, &pairs);
            None
        }
    };
    println!();
    match answered {
        Some(value) => println!("{}\n  {}", paint.head("ANSWER"), plainly(&value)),
        None if HOOKS[index].answers => println!(
            "{}\n  {}",
            paint.head("ANSWER"),
            paint.dim("nothing — no handler decided")
        ),
        None => {}
    }
    0
}

/// A Lua value as a person would write it, rather than as Rust debugs it.
///
/// Only the shapes a hook answers with: a replacement line, a refusal, a status. Anything else is
/// debugged, which is honest about it being unusual rather than pretending to render it.
fn plainly(value: &oslo_base::value::Value) -> String {
    match value {
        oslo_base::value::Value::Str(text) => text.to_string(),
        oslo_base::value::Value::Bool(yes) => yes.to_string(),
        oslo_base::value::Value::Nil => "nil".to_string(),
        other => format!("{other:?}"),
    }
}

/// A name that is not a hook, with the nearest ones that are.
fn unknown_hook(name: &str) -> i32 {
    eprintln!("oslo hook: {name:?}: no such hook");
    let close: Vec<&str> = HOOKS
        .iter()
        .map(|hook| hook.name)
        .filter(|known| known.contains(name) || name.contains(known))
        .collect();
    if !close.is_empty() {
        eprintln!("  did you mean: {}", close.join(", "));
    }
    eprintln!("  `oslo hook list` shows every one");
    2
}
