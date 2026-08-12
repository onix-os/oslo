//! `oslo config` — what the shell read, and which file said what.
//!
//! # Why provenance needed a command
//!
//! A session's configuration now comes from three places: `config.lua`, every `conf.d/*.lua` before
//! it, and — since plugins — code somebody else wrote. "Why is my keybinding not working" had no
//! answer at all, and the honest one is usually "something later set it again".
//!
//! Neovim answers this with `:verbose set x?`, which names the file and line. This is the same
//! question with the same shape of answer, and it is a *tool* rather than a builtin because it has
//! to read the files rather than ask the running shell: `oslo config` is a different process from
//! your session, and a shell that could be interrogated from outside would be a stranger thing than
//! one that reproduces the load.

use crate::cli::help::Paint;
use oslo_lua::Value;
use std::path::PathBuf;

pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") | Some("help") => {
            print!("{}", help(Paint::detect()));
            i32::from(args.is_empty()) * 2
        }
        Some("files") => files(),
        Some("which") => match args.get(1) {
            Some(key) => which(key),
            None => {
                eprintln!(
                    "oslo config: which needs a setting, as in `oslo config which vi.enabled`"
                );
                2
            }
        },
        Some(other) => {
            eprintln!("oslo config: unknown subcommand {other:?}\n");
            eprint!("{}", help(Paint::plain()));
            2
        }
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

pub fn help(paint: Paint) -> String {
    use crate::cli::help::row;
    use std::fmt::Write as _;
    let mut text = String::new();
    let _ = writeln!(text, "{}", paint.head("USAGE"));
    let _ = writeln!(
        text,
        "  {} {} {} {}",
        paint.key("oslo"),
        paint.key("config"),
        paint.slot("<subcommand>"),
        paint.slot("[argument]...")
    );
    let _ = writeln!(text, "\n{}", paint.head("SUBCOMMANDS"));
    text.push_str(&row(
        "files",
        paint.key("files"),
        "every file a session reads, in order",
    ));
    text.push_str(&row(
        "which",
        paint.key("which"),
        "which file last set a setting, and to what",
    ));
    let _ = writeln!(text, "\n  {}", paint.dim("oslo config which vi.enabled"));
    text
}

#[cfg(test)]
#[path = "config/tests.rs"]
mod tests;
