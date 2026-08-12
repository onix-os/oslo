//! `oslo plugin` — install, list, remove, allow.
//!
//! The rules live in [`oslo_runtime::plugin::install`]; this parses words and prints. **oslo's own
//! subcommand, not a plugin's**: a plugin extends the shell you type at, and nothing a plugin
//! declares is reachable from here.

pub mod help;

use crate::cli::help::Paint;
use oslo_runtime::plugin::{index, install, manifest, trust};
use std::path::{Path, PathBuf};

pub fn run(args: &[String]) -> i32 {
    let Some(command) = args.first().map(String::as_str) else {
        print!("{}", help::text(Paint::detect()));
        return 2;
    };
    if matches!(command, "-h" | "--help" | "help") {
        print!("{}", help::text(Paint::detect()));
        return 0;
    }
    // Handled before the subcommand parses its own arguments, the way `history` does it.
    let rest = &args[1..];
    if rest
        .first()
        .is_some_and(|a| matches!(a.as_str(), "-h" | "--help"))
        && let Some(help) = help::subcommand(command, Paint::detect())
    {
        print!("{help}");
        return 0;
    }
    match command {
        "list" => list(),
        "install" => match rest.first() {
            Some(source) => install_one(source, rest.iter().any(|a| a == "--yes")),
            None => usage("install needs something to install"),
        },
        "remove" => match rest.first() {
            Some(name) => remove_one(name),
            None => usage("remove needs a plugin name"),
        },
        "allow" => match rest.first() {
            Some(name) => allow_one(name),
            None => usage("allow needs a plugin name"),
        },
        other => usage(&format!("unknown subcommand {other:?}")),
    }
}

fn usage(message: &str) -> i32 {
    eprintln!("oslo plugin: {message}\n");
    eprint!("{}", help::text(Paint::plain()));
    2
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

/// Copy or clone a plugin in, then record what it declared and what it hashed to.
fn install_one(source: &str, assume_yes: bool) -> i32 {
    let source = match install::Source::parse(source) {
        Ok(source) => source,
        Err(problem) => return usage(&problem),
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
    println!(
        "{} {} reserves: {}",
        planned.manifest.name,
        planned.manifest.version,
        planned
            .manifest
            .names()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );
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
