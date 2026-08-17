//! `oslo config` — what the shell read, and which file said what.
//!
//! # Why provenance needed a command
//!
//! A session's configuration now comes from three places: `init.lua`, every `conf.d/*.lua` before
//! it, and — since plugins — code somebody else wrote. "Why is my keybinding not working" had no
//! answer at all, and the honest one is usually "something later set it again".
//!
//! Neovim answers this with `:verbose set x?`, which names the file and line. This is the same
//! question with the same shape of answer, and it is a *tool* rather than a builtin because it has
//! to read the files rather than ask the running shell: `oslo config` is a different process from
//! your session, and a shell that could be interrogated from outside would be a stranger thing than
//! one that reproduces the load.

use crate::cli::help::Paint;
use crate::cli::help::menu::{CALL, Menu, SUBCOMMANDS as HEADING, Sub};
use oslo_base::value::Value;
use oslo_luavm::Host;
use std::path::PathBuf;

pub(crate) const MENU: Menu = Menu {
    path: &["config"],
    call: CALL,
    heading: HEADING,
    subs: SUBCOMMANDS,
    notes: &["oslo config which vi.enabled"],
    nested: &[],
};

const SUBCOMMANDS: &[Sub] = &[
    Sub {
        name: "files",
        args: "",
        about: "every file a session reads, in order",
        flags: &[],
        note: "Read in that order, and the last to set something wins — which is the whole of why \
               a keybinding you set stops working when a plugin arrives.",
    },
    Sub {
        name: "timing",
        args: "",
        about: "what each configuration file costs at startup",
        flags: &[],
        note: "The files are loaded to measure them, in this process rather than in your session.",
    },
    Sub {
        name: "which",
        args: "SETTING",
        about: "which file last set a setting, and to what",
        flags: &[],
        note: "Written as it is in `init.lua`, dots and all: `oslo config which vi.enabled`. The \
               load is reproduced here rather than asked of the running shell, so the answer is \
               what a *new* session would see.",
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
        Some("files") => MENU.extra("files", args, 0).unwrap_or_else(files),
        Some("timing") => MENU.extra("timing", args, 0).unwrap_or_else(timing),
        Some("which") => match args.get(1) {
            Some(key) => MENU.extra("which", args, 1).unwrap_or_else(|| which(key)),
            None => MENU.missing("which needs a setting, as in `oslo config which vi.enabled`"),
        },
        Some(other) => MENU.unknown(other),
    }
}

/// Every file a session would read, in the order it reads them.
fn files() -> i32 {
    let found = config_files();
    if found.is_empty() {
        println!("no configuration files");
        return 0;
    }
    for path in &found {
        println!("{}", path.display());
    }
    println!();
    println!(
        "{}",
        Paint::detect().dim("Read in this order; the last to set something wins.")
    );
    0
}

/// What each file and plugin costs a session at startup.
///
/// **The same load the shell does, measured.** A session now reads `conf.d/*.lua`, `init.lua`,
/// every installed plugin's index entry and whatever those register — five suspects when a shell
/// feels slow, and until this there was no instrument at all. Neovim grew `--startuptime` for the
/// same reason: the alternative is commenting lines out until it stops.
fn timing() -> i32 {
    let files = config_files();
    let engine = match oslo_runtime::LuaEngine::new() {
        Ok(engine) => engine,
        Err(problem) => {
            eprintln!("oslo config: lua: {problem}");
            return 1;
        }
    };
    let env = std::sync::Arc::new(std::sync::Mutex::new(oslo::env::Environment::new()));

    // The bindings themselves are the floor: everything below is measured against a shell that has
    // already paid for them, so a config that looks expensive is expensive *for a config*.
    let started = std::time::Instant::now();
    if !oslo_runtime::startup::lua_init::install_bindings(&engine, env) {
        return 1;
    }
    let bindings = started.elapsed();

    let mut measured: Vec<(String, std::time::Duration)> = Vec::new();
    for path in &files {
        let at = std::time::Instant::now();
        oslo_runtime::startup::lua_init::load_config(&engine, path);
        measured.push((path.display().to_string(), at.elapsed()));
    }

    let paint = Paint::detect();
    println!("{}", paint.head("STARTUP"));
    println!(
        "  {:>8.3} ms  {}",
        ms(bindings),
        paint.dim("the oslo API itself")
    );
    let mut total = bindings;
    for (what, took) in &measured {
        total += *took;
        println!("  {:>8.3} ms  {what}", ms(*took));
    }
    println!("  {:>8.3} ms  {}", ms(total), paint.key("total"));
    if measured.is_empty() {
        println!("\n  {}", paint.dim("no configuration files"));
    }
    // **Not the whole story, and it says so.** A plugin loads when something names it, which is
    // after this — so a slow plugin shows up in the command that woke it, not here.
    println!(
        "\n  {}",
        paint
            .dim("Plugins load on first use, so their cost is not here; see `oslo plugin doctor`.")
    );
    0
}

fn ms(took: std::time::Duration) -> f64 {
    took.as_secs_f64() * 1000.0
}

/// Which file last set `key`, and to what.
///
/// **The files are loaded cumulatively, exactly as a session loads them**, and the value is read
/// after each. A file that set something an earlier one had already set is the answer, because that
/// is what the session ends up with — reading the files separately would name every file that
/// mentions the key and answer a different question.
fn which(key: &str) -> i32 {
    let files = config_files();
    if files.is_empty() {
        println!("no configuration files");
        return 1;
    }
    let engine = match oslo_runtime::LuaEngine::new() {
        Ok(engine) => engine,
        Err(problem) => {
            eprintln!("oslo config: lua: {problem}");
            return 1;
        }
    };
    let env = std::sync::Arc::new(std::sync::Mutex::new(oslo::env::Environment::new()));
    if !oslo_runtime::startup::lua_init::install_bindings(&engine, env) {
        return 1;
    }

    let mut was = read(&engine, key);
    let mut setter: Option<(PathBuf, Option<String>)> = None;
    for path in &files {
        oslo_runtime::startup::lua_init::load_config(&engine, path);
        let now = read(&engine, key);
        if now != was {
            setter = Some((path.clone(), now.clone()));
            was = now;
        }
    }

    match setter {
        Some((path, value)) => {
            println!("{key} = {}", value.unwrap_or_else(|| "nil".to_string()));
            println!("set by {}", path.display());
            0
        }
        None => {
            // Not the same as "nothing sets it": a file that set it to the value it already had
            // cannot be told apart from one that never mentioned it, and saying so is better than
            // naming a file on a guess.
            println!("{key} = {}", was.unwrap_or_else(|| "nil".to_string()));
            println!("no configuration file changed it");
            0
        }
    }
}

/// The value at a dotted path under `oslo`, rendered so two readings can be compared.
fn read(_engine: &oslo_runtime::LuaEngine, key: &str) -> Option<String> {
    // Through the interpreter on this thread, which `install_bindings` put there — the engine owns
    // it and does not hand it out, and reaching for a global is not something a *session* ever does.
    let interp = oslo_runtime::lua::engine::interpreter_handle()?;
    let mut value = interp.global("oslo");
    for step in key.split('.') {
        let Value::Table(table) = value else {
            return None;
        };
        let next = table.borrow().get(&Value::str(step));
        value = next;
    }
    match value {
        Value::Nil => None,
        Value::Str(text) => Some(text.to_string()),
        // No setting is bytes, and one that has been made so is worth saying rather than rendering
        // lossily into something that looks like a value somebody chose.
        Value::Bytes(b) => Some(format!("<{} bytes, not text>", b.len())),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Function(_) => Some("<a function>".to_string()),
        Value::Table(table) => {
            // Enough to tell one table from another without pretending to serialise it: a setting
            // that is a list is usually being compared for having changed at all.
            let table = table.borrow();
            let mut fields: Vec<String> = table
                .pairs()
                .into_iter()
                .map(|(k, _)| match k {
                    Value::Str(name) => name.to_string(),
                    other => format!("{other:?}"),
                })
                .collect();
            fields.sort();
            Some(format!("<a table: {}>", fields.join(", ")))
        }
    }
}

/// The files a session would read, asked of the same code that reads them.
fn config_files() -> Vec<PathBuf> {
    let env = oslo::env::Environment::new();
    oslo_runtime::startup::lua_init::config_files(&env)
}

#[cfg(test)]
#[path = "config/tests.rs"]
mod tests;
