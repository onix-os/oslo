//! `oslo plugin` — install, list, remove, allow.
//!
//! The rules live in [`oslo_runtime::plugin::install`]; this parses words and prints. **oslo's own
//! subcommand, not a plugin's**: a plugin extends the shell you type at, and nothing a plugin
//! declares is reachable from here.

pub mod help;
mod test;

use crate::cli::help::Paint;
use oslo_runtime::plugin::{doctor, index, install, manifest, trust};
use std::path::{Path, PathBuf};

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
        "list" => list(),
        "doctor" => doctor_command(rest.first().map(String::as_str)),
        // The directory is optional: an author runs this from inside the plugin they are writing.
        "test" => test::run(rest.first().map(String::as_str)),
        "install" => match rest.first() {
            Some(source) => install_one(source, rest.iter().any(|a| a == "--yes")),
            None => help::MENU.wrong("install", "needs something to install"),
        },
        "remove" => match rest.first() {
            Some(name) => remove_one(name),
            None => help::MENU.wrong("remove", "needs a plugin name"),
        },
        "allow" => match rest.first() {
            Some(name) => allow_one(name),
            None => help::MENU.wrong("allow", "needs a plugin name"),
        },
        other => help::MENU.unknown(other),
    }
}

/// What is installed, and whether it still hashes to what was allowed.
fn list() -> i32 {
    let entries = index::read();
    if entries.is_empty() {
        println!("no plugins installed");
        return 0;
    }
    for installed in &entries {
        let state = match installed
            .directory()
            .ok_or_else(|| "nowhere to look".to_string())
            .and_then(|directory| trust::unchanged(&directory, &installed.hash))
        {
            Ok(true) => "ok".to_string(),
            Ok(false) => "CHANGED — run `oslo plugin allow`".to_string(),
            Err(problem) => problem,
        };
        let names: Vec<&str> = installed.names().map(String::as_str).collect();
        println!("{:<20} {:<34} {state}", installed.name, names.join(", "));
    }
    0
}

/// Everything that could be wrong, in one place.
///
/// Named for `:checkhealth`, which neovim has for the same reason: "it is installed and nothing
/// happens" is the question a plugin system is asked most, and the answers are scattered across
/// lines on stderr that went past while you were reading something else.
fn doctor_command(one: Option<&str>) -> i32 {
    // The shell's own names, so the doctor can say when a plugin's command is shadowed by a builtin
    // — the failure that looks most exactly like nothing happening. A fresh environment has exactly
    // the native builtins and nothing a config added, which is the right comparison: a config's own
    // builtin is the user's decision, and only oslo's are a name a plugin can never have.
    let shell = oslo::env::Environment::new();
    let taken = |name: &str| shell.is_builtin(name);
    let mut findings = match one {
        Some(name) => doctor::report(taken)
            .into_iter()
            .filter(|f| f.plugin == name)
            .collect(),
        None => doctor::report(taken),
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

/// Copy or clone a plugin in, then record what it declared and what it hashed to.
fn install_one(source: &str, assume_yes: bool) -> i32 {
    let source = match install::Source::parse(source) {
        Ok(source) => source,
        Err(problem) => return help::MENU.wrong("install", &problem),
    };
    let Some(root) = oslo_runtime::plugin::directory() else {
        eprintln!("oslo plugin: no $XDG_DATA_HOME and no $HOME, so there is nowhere to install to");
        return 1;
    };

    // Fetched into a staging directory so that a plugin which fails its checks never appears under
    // the name it wanted — an install either happens or does not.
    let staging = match tempfile::tempdir() {
        Ok(staging) => staging,
        Err(error) => {
            eprintln!("oslo plugin: {error}");
            return 1;
        }
    };
    let candidate = match fetch(&source, staging.path()) {
        Ok(candidate) => candidate,
        Err(problem) => {
            eprintln!("oslo plugin: {problem}");
            return 1;
        }
    };

    let installed = index::read();
    let planned = match install::plan(&candidate, &installed) {
        Ok(planned) => planned,
        Err(problem) => {
            eprintln!("oslo plugin: {problem}");
            return 1;
        }
    };
    if !planned.conflicts.is_empty() {
        eprintln!(
            "oslo plugin: {} would claim {}, which another plugin already has",
            planned.manifest.name,
            planned.conflicts.join(", ")
        );
        return 1;
    }

    // **What it will reserve, before it is trusted.** Nothing of the plugin has run at this point:
    // the manifest was read in an interpreter with no `oslo` in it, and the hash is over bytes.
    let reserves: Vec<String> = planned.manifest.names().cloned().collect();
    println!(
        "{} {} {}",
        planned.manifest.name,
        planned.manifest.version,
        match (reserves.is_empty(), &planned.manifest.load_on) {
            // A plugin with no commands is not a broken one: it watches. Saying "reserves:" with
            // nothing after it reads as a plugin that failed to declare anything.
            (true, Some(hook)) => format!("reserves nothing; loads on `{hook}`"),
            (true, None) => "reserves nothing".to_string(),
            (false, Some(hook)) => {
                format!("reserves: {}; loads on `{hook}`", reserves.join(", "))
            }
            (false, None) => format!("reserves: {}", reserves.join(", ")),
        }
    );
    // **Said before the question, not after.** A plugin that will read your tokens says so in a
    // manifest read with no `oslo` in it, which is the one moment the claim can be seen before its
    // code has had a chance to do anything.
    if !planned.manifest.secrets.is_empty() {
        println!(
            "  secrets: {}   it will be able to read these",
            planned.manifest.secrets.join(", ")
        );
    }
    if !assume_yes && !confirm("install and allow it to run?") {
        println!("not installed");
        return 1;
    }

    let destination = root.join(&planned.manifest.name);
    let _ = std::fs::remove_dir_all(&destination);
    if let Err(problem) = install::copy_tree(&candidate, &destination) {
        eprintln!("oslo plugin: {problem}");
        return 1;
    }
    // Hashed where it landed rather than where it came from: what is loaded is what is recorded.
    let hash = match trust::hash_of(&destination) {
        Ok(hash) => hash,
        Err(problem) => {
            eprintln!("oslo plugin: {problem}");
            return 1;
        }
    };
    if let Err(problem) = install::remember(index::Installed::of(&planned.manifest, hash)) {
        eprintln!("oslo plugin: {problem}");
        return 1;
    }
    println!("installed {}", planned.manifest.name);
    0
}

/// Put the plugin's files where they can be read, and answer the directory holding its manifest.
fn fetch(source: &install::Source, staging: &Path) -> Result<PathBuf, String> {
    match source {
        install::Source::Path(from) => Ok(from.clone()),
        install::Source::Git { url, revision } => {
            let into = staging.join("clone");
            run_git(&["clone", "--quiet", url, &into.to_string_lossy()])?;
            // **Detached at the revision named.** A branch would be a different plugin tomorrow,
            // and the trust hash would refuse to load it every time upstream moved.
            run_git(&[
                "-C",
                &into.to_string_lossy(),
                "checkout",
                "--quiet",
                revision,
            ])?;
            Ok(into)
        }
    }
}

fn run_git(args: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new("git")
        .args(args)
        .output()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => "git is not installed".to_string(),
            _ => error.to_string(),
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
}

fn remove_one(name: &str) -> i32 {
    match install::remove(name) {
        Ok(_) => {
            println!("removed {name}");
            // Said out loud, because the alternative is somebody discovering it later.
            if let Some(path) = oslo_base::store::path_of(name)
                && path.is_file()
            {
                println!("its database is left at {}", path.display());
            }
            0
        }
        Err(problem) => {
            eprintln!("oslo plugin: {problem}");
            1
        }
    }
}

/// Record what the plugin hashes to now, after an update changed it.
fn allow_one(name: &str) -> i32 {
    let entries = index::read();
    let Some(installed) = entries.iter().find(|installed| installed.name == name) else {
        eprintln!("oslo plugin: {name}: not installed");
        return 1;
    };
    let Some(directory) = installed.directory() else {
        eprintln!("oslo plugin: nowhere to look");
        return 1;
    };
    // Re-read the manifest too: an update may declare names the old one did not, and allowing the
    // code without re-reading what it claims would leave the index describing the version before.
    let manifest = match manifest::read(&directory) {
        Ok(manifest) => manifest,
        Err(problem) => {
            eprintln!("oslo plugin: {problem}");
            return 1;
        }
    };
    let hash = match trust::hash_of(&directory) {
        Ok(hash) => hash,
        Err(problem) => {
            eprintln!("oslo plugin: {problem}");
            return 1;
        }
    };
    if hash == installed.hash {
        println!("{name} has not changed");
        return 0;
    }
    if let Err(problem) = install::remember(index::Installed::of(&manifest, hash)) {
        eprintln!("oslo plugin: {problem}");
        return 1;
    }
    println!("allowed {name}");
    0
}

/// Ask, and take anything but `y` as no.
fn confirm(question: &str) -> bool {
    use std::io::Write;
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim(), "y" | "Y" | "yes")
}
