//! Completion spec files: [carapace-spec](https://github.com/carapace-sh/carapace-spec), read here.
//!
//! ```text
//!   ~/.config/oslo/specs/mycmd.yaml
//!        │
//!        ├─ yaml   the slice of YAML a spec is written in
//!        ├─ read   that document as oslo's own CommandSpec
//!        └─ run    the macros only a shell can answer: $(…), $bash(…)
//! ```
//!
//! # Found by name, not read at startup
//!
//! A directory of specs is a directory of files nobody has typed the name of yet. Reading all of
//! them to start a shell is the cost `carapace` pays by generating a script per shell, and oslo has
//! spent this feature's whole design avoiding it: a spec is looked for the first time its command
//! is completed, and the answer — including "there is none" — is remembered. A machine with no
//! specs pays one `stat` per new command name and nothing after it.
//!
//! # What it deliberately does not do
//!
//! `run:` is not read. A spec file here *describes* a command; it does not become one. That is a
//! whole second feature — carapace-spec ships a binary for it — and it is not what a shell needs
//! from a spec.

#[cfg(feature = "spec")]
pub mod read;
pub mod run;
#[cfg(feature = "spec")]
pub mod yaml;

#[cfg(feature = "spec")]
use oslo_ui::spec::CommandSpec;
#[cfg(feature = "spec")]
use std::path::PathBuf;

/// The directories searched, nearest first.
///
/// `$OSLO_SPECS` before the config directory, so a project can carry its own without installing
/// them; carapace's own directory last, because a machine that has carapace already has specs in
/// it and there is no reason to make somebody copy them.
#[cfg(feature = "spec")]
pub fn directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(listed) = std::env::var("OSLO_SPECS") {
        dirs.extend(
            listed
                .split(':')
                .filter(|p| !p.is_empty())
                .map(PathBuf::from),
        );
    }
    let config = match std::env::var("XDG_CONFIG_HOME") {
        Ok(path) if path.starts_with('/') => PathBuf::from(path),
        _ => match std::env::var("HOME") {
            Ok(home) => PathBuf::from(home).join(".config"),
            Err(_) => return dirs,
        },
    };
    dirs.push(config.join("oslo/specs"));
    dirs.push(config.join("carapace/specs"));
    dirs
}

/// The spec for `command`, if a file anywhere declares one.
///
/// A file that is there and does not parse is reported and then skipped: the alternative is a Tab
/// key that silently does nothing on the one command whose spec you are in the middle of writing.
#[cfg(feature = "spec")]
pub fn find(command: &str) -> Option<CommandSpec> {
    find_in(&directories(), command)
}

/// The same, in directories the caller names. The seam the tests use, so that reading a spec never
/// depends on a process-wide variable two of them would have to take turns with.
#[cfg(feature = "spec")]
pub fn find_in(dirs: &[PathBuf], command: &str) -> Option<CommandSpec> {
    // A name with a path in it is not a spec name, and a leading dot is how `..` reaches out of
    // the directory. A dot elsewhere is ordinary — `python3.11` is a command somebody runs.
    if command.is_empty() || command.contains('/') || command.starts_with('.') {
        return None;
    }
    for dir in dirs {
        for extension in ["yaml", "yml"] {
            let path = dir.join(format!("{command}.{extension}"));
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            match read::spec(&source) {
                Ok(mut spec) => {
                    // The file decides the name it answers to, not the other way round: a spec
                    // whose `name` disagrees with its filename would otherwise be unreachable.
                    spec.name = command.to_string();
                    return Some(spec);
                }
                Err(problem) => oslo_base::messages::say(
                    oslo_base::messages::Level::Warn,
                    "spec".to_string(),
                    format!("{}: {problem}", path.display()),
                ),
            }
        }
    }
    None
}

/// Every command a spec file is there for, for `oslo spec list` and the tests.
#[cfg(feature = "spec")]
pub fn available() -> Vec<String> {
    let mut names: Vec<String> = directories()
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let extension = path.extension()?.to_str()?;
            matches!(extension, "yaml" | "yml")
                .then(|| path.file_stem()?.to_str().map(str::to_string))
                .flatten()
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

#[cfg(all(test, feature = "spec"))]
#[path = "mod/tests.rs"]
mod tests;
