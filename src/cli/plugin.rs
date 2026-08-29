//! `oslo plugin` — what is on the runtimepath, and whether it is working.
//!
//! There is no install verb, no remove verb and no approve verb, because there is nothing for them
//! to do: a plugin is a directory on the path, so installing one is putting it there and removing
//! one is taking it away. What is left is the question that actually gets asked, usually at two in
//! the morning: *is it me or a plugin, and in what order did they run?*

pub mod help;
mod test;

use crate::cli::help::Paint;
use oslo_runtime::plugin::doctor;
use oslo_runtime::runtimepath;

pub fn run(args: &[String]) -> i32 {
    if let Some(page) = help::MENU.asked(args, Paint::detect()) {
        print!("{page}");
        return 0;
    }
    let Some(command) = args.first().map(String::as_str) else {
        print!("{}", help::text(Paint::detect()));
        return 0;
    };
    let rest = &args[1..];
    match command {
        "list" => help::MENU.extra("list", args, 0).unwrap_or_else(list),
        "doctor" => help::MENU
            .extra("doctor", args, 1)
            .unwrap_or_else(|| doctor_command(rest.first().map(String::as_str))),
        // The directory is optional: an author runs this from inside the plugin they are writing.
        "test" => test::run(rest.first().map(String::as_str)),
        other => help::MENU.unknown(other),
    }
}

/// The path, and the files it would run, in the order it would run them.
///
/// Both, because either alone leaves the interesting case unexplained: a plugin that is not running
/// is usually in a directory that is not on the path, and a plugin behaving oddly is usually one
/// that another plugin ran before.
fn list() -> i32 {
    let paint = Paint::detect();
    let roots = runtimepath::roots();

    println!("runtimepath");
    for root in &roots {
        // Saying which of them exist turns "my plugin does not load" into one look: the directory it
        // is in is usually not on the path at all.
        let there = if root.path.is_dir() { " " } else { "-" };
        println!("  {there} {}", root.path.display());
    }

    let files = runtimepath::plugin_files(&roots);
    println!();
    println!("plugins, in load order");
    if files.is_empty() {
        println!("    none");
    }
    for (index, file) in files.iter().enumerate() {
        println!("  {:>3}. {}", index + 1, file.path.display());
    }

    println!();
    println!(
        "{}",
        paint.dim(
            "a `-` marks a directory that is not there; nothing else is needed to install a \
             plugin than putting it on this path. `--noplugin` skips them all."
        )
    );
    0
}

/// Everything that could be wrong, in one place.
///
/// Named for `:checkhealth`, which neovim has for the same reason: "it is installed and nothing
/// happens" is the question a plugin system is asked most, and the answers are scattered across
/// lines on stderr that went past while you were reading something else.
fn doctor_command(one: Option<&str>) -> i32 {
    let mut findings = match one {
        Some(name) => doctor::report()
            .into_iter()
            .filter(|f| f.plugin == name)
            .collect(),
        None => doctor::report(),
    };
    // A plugin's own checks need it loaded, so they are asked for only when one is named — and
    // loading Lua needs an interpreter, which this process has none of: the engine belongs to the
    // interactive loop, and `oslo plugin` is a tool. One is built here for the length of the check.
    if let Some(name) = one {
        findings.extend(with_lua(|| doctor::checks_from(name)));
    }

    let paint = Paint::detect();
    let mut worst = 0;
    for finding in &findings {
        let (mark, code) = match finding.state {
            doctor::State::Ok => (paint.key("ok  "), 0),
            doctor::State::Warn => (paint.slot("warn"), 1),
            doctor::State::Bad => (paint.key("BAD "), 2),
        };
        worst = worst.max(code);
        if finding.plugin.is_empty() {
            println!("{mark}  {}", finding.says);
        } else {
            println!("{mark}  {:<16} {}", finding.plugin, finding.says);
        }
    }
    if one.is_none() {
        println!();
        println!(
            "{}",
            paint.dim("`oslo plugin doctor <name>` loads that plugin and asks its own checks too.")
        );
    }
    // Nothing wrong is 0; a warning is still a working shell, so only a bad finding fails.
    i32::from(worst == 2)
}

/// Run `work` with a Lua interpreter installed on this thread.
///
/// The doctor is the only part of `oslo plugin` that runs any of a plugin's code, and it does so to
/// ask a question rather than to be a shell — so the environment it builds is a fresh one that
/// nothing else sees, and it goes away with the call.
fn with_lua<T>(work: impl FnOnce() -> T) -> T {
    let engine = match oslo_runtime::LuaEngine::new() {
        Ok(engine) => engine,
        Err(problem) => {
            eprintln!("oslo plugin: lua: {problem}");
            return work();
        }
    };
    let env = std::sync::Arc::new(std::sync::Mutex::new(oslo::env::Environment::new()));
    oslo_runtime::startup::lua_init::install_bindings(&engine, env);
    work()
}
