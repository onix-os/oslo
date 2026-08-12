//! What is wrong, for the question every plugin system is asked most: *it is installed and nothing
//! happens.*
//!
//! The plugin design added three new ways to reach that question — the trust hash refused, the name
//! was already taken, the plugin registered nothing — and each currently arrives as a line on stderr
//! that goes past while you are reading something else. This is where they are all asked at once.
//!
//! Modelled on `vim.health`, which exists in neovim for the same reason: a check a *plugin* writes
//! is the only one that knows what that plugin needs — whether `age` is installed, whether its
//! database is writable — and no amount of checking by the shell can guess it.

use super::{index, manifest, trust};

/// How one check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Ok,
    /// It works, but something about it is worth saying.
    Warn,
    /// It does not work.
    Bad,
}

/// One line of a report.
#[derive(Debug, Clone)]
pub struct Finding {
    pub plugin: String,
    pub state: State,
    pub says: String,
}

impl Finding {
    fn new(plugin: &str, state: State, says: impl Into<String>) -> Finding {
        Finding {
            plugin: plugin.to_string(),
            state,
            says: says.into(),
        }
    }
}

/// Check everything installed.
///
/// **Static checks only, in this pass.** Nothing here loads a plugin: a report should be safe to ask
/// for when a plugin is the thing misbehaving, and loading is what a plugin's own health check needs
/// — see [`self::checks_from`].
pub fn report(taken: impl Fn(&str) -> bool) -> Vec<Finding> {
    let entries = index::read();
    if entries.is_empty() {
        return vec![Finding::new("", State::Ok, "no plugins installed")];
    }

    let mut found = Vec::new();
    let mut claimed: Vec<String> = Vec::new();
    for installed in &entries {
        let name = installed.name.as_str();
        let Some(directory) = installed.directory() else {
            found.push(Finding::new(name, State::Bad, "nowhere to look for it"));
            continue;
        };
        if !directory.is_dir() {
            found.push(Finding::new(
                name,
                State::Bad,
                format!("{} is not there; reinstall it", directory.display()),
            ));
            continue;
        }

        // The manifest, re-read: it is the thing a person edits, and the index is generated from it.
        match manifest::read(&directory) {
            Ok(read) => {
                if read.builtins != installed.builtins || read.tools != installed.tools {
                    found.push(Finding::new(
                        name,
                        State::Warn,
                        "its manifest declares different names than the index has; reinstall it",
                    ));
                }
                if let Some(requirement) = &read.requires
                    && !manifest::satisfied(requirement, oslo_base::version::current())
                        .unwrap_or(false)
                {
                    found.push(Finding::new(
                        name,
                        State::Bad,
                        format!(
                            "it needs oslo {requirement} and this is {}",
                            oslo_base::version::current()
                        ),
                    ));
                }
            }
            Err(problem) => found.push(Finding::new(name, State::Bad, problem)),
        }

        match trust::unchanged(&directory, &installed.hash) {
            Ok(true) => {}
            Ok(false) => found.push(Finding::new(
                name,
                State::Bad,
                format!("it has changed since you allowed it; `oslo plugin allow {name}`"),
            )),
            Err(problem) => found.push(Finding::new(name, State::Bad, problem)),
        }

        // A name something else answers to is the failure that looks most like nothing happening:
        // the plugin is installed, is trusted, and its command runs somebody else's code.
        for wanted in installed.names() {
            if taken(wanted) {
                found.push(Finding::new(
                    name,
                    State::Warn,
                    format!("`{wanted}` is already a command; this plugin will not get it"),
                ));
            }
            if claimed.contains(wanted) {
                found.push(Finding::new(
                    name,
                    State::Warn,
                    format!("`{wanted}` is claimed by another plugin too"),
                ));
            }
            claimed.push(wanted.clone());
        }

        if !found.iter().any(|f| f.plugin == name) {
            found.push(Finding::new(
                name,
                State::Ok,
                format!(
                    "{} — {}",
                    installed.names().cloned().collect::<Vec<_>>().join(", "),
                    directory.display()
                ),
            ));
        }
    }
    found
}

/// What a plugin's own health checks answered, having loaded it to ask.
///
/// **This loads the plugin, and says so at the call site.** A check a plugin writes can only run
/// once its Lua has; that is the whole difference between this and [`report`], which is why they are
/// two functions and not one with a flag.
pub fn checks_from(name: &str) -> Vec<Finding> {
    let entries = index::read();
    let Some(installed) = entries.iter().find(|entry| entry.name == name) else {
        return vec![Finding::new(name, State::Bad, "not installed")];
    };
    if let Err(problem) = super::load_one(installed) {
        return vec![Finding::new(name, State::Bad, problem)];
    }
    super::health::run(name)
        .into_iter()
        .map(|(state, says)| Finding::new(name, state, says))
        .collect()
}

#[cfg(test)]
#[path = "doctor/tests.rs"]
mod tests;
