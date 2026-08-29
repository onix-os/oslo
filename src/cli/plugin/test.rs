//! `oslo plugin test [DIRECTORY]` — run a plugin's own assertions against a machine with nothing on
//! it.
//!
//! ```text
//! oslo plugin test              the plugin in the current directory
//! oslo plugin test ~/src/notes  one somewhere else
//! ```
//!
//! # A temporary home, which is most of the point
//!
//! A plugin's tests are about a *fresh* install: an empty database, no configuration, none of the
//! author's own data. Running them against the real home would mean the first test to pass is the
//! one that passes because the author already has three notes in there, and the failure a user hits
//! on day one is exactly the one that cannot be reproduced.
//!
//! So `$HOME`, `$XDG_DATA_HOME` and `$XDG_CONFIG_HOME` are pointed at a directory that is deleted
//! afterwards, *before* the interpreter exists — `oslo.db.open` resolves its path on every call, so
//! anything the plugin opens lands there.

use crate::cli::help::Paint;
use oslo_runtime::plugin;
use std::path::Path;

pub fn run(where_: Option<&str>) -> i32 {
    let directory = Path::new(where_.unwrap_or("."));
    // A plugin root is a directory with `plugin/` in it -- the same shape a config root has, which
    // is why one can be developed beside your init.lua and moved into a package later.
    if !directory.join("plugin").is_dir() {
        eprintln!(
            "oslo plugin: {}: no plugin/ directory, so this is not a plugin root",
            directory.display()
        );
        return 2;
    }
    let Ok(home) = tempfile::tempdir() else {
        eprintln!("oslo plugin: nowhere to put a temporary home");
        return 1;
    };

    // Set before the engine is built, and left set: this process runs the tests and exits.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("XDG_DATA_HOME", home.path().join("data"));
        std::env::set_var("XDG_CONFIG_HOME", home.path().join("config"));
    }

    let engine = match oslo_runtime::LuaEngine::new() {
        Ok(engine) => engine,
        Err(problem) => {
            eprintln!("oslo plugin: lua: {problem}");
            return 1;
        }
    };
    let env = std::sync::Arc::new(std::sync::Mutex::new(oslo::env::Environment::new()));
    oslo_runtime::startup::lua_init::install_bindings(&engine, env);

    let name = match plugin::load_from(directory) {
        Ok(name) => name,
        Err(problem) => {
            // A plugin that cannot load is a failed test run, not a usage error: this is the most
            // common thing `oslo plugin test` will report, and it is the report.
            eprintln!("oslo plugin: {problem}");
            return 1;
        }
    };

    report(&name, plugin::test::run(&name))
}

fn report(name: &str, outcomes: Vec<plugin::test::Outcome>) -> i32 {
    let paint = Paint::detect();
    if outcomes.is_empty() {
        // Not a failure. A plugin with no tests is a plugin somebody has not written tests for, and
        // exiting non-zero would make `oslo plugin test` unusable in a loop over several.
        println!(
            "{name}: {}",
            paint.dim("no tests; declare one with oslo.plugin.test")
        );
        return 0;
    }

    let mut failed = 0;
    for outcome in &outcomes {
        if outcome.passed() {
            println!(
                "{}  {} {}",
                paint.key("ok  "),
                outcome.name,
                paint.dim(&format!("({} checked)", outcome.checked))
            );
        } else {
            failed += 1;
            println!("{}  {}", paint.key("FAIL"), outcome.name);
            for failure in &outcome.failures {
                println!("        {}", failure.says);
            }
        }
    }

    println!();
    println!(
        "{name}: {} of {} passed",
        outcomes.len() - failed,
        outcomes.len()
    );
    i32::from(failed > 0)
}
