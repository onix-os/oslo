//! Putting a plugin where the shell will find it, and taking it away again.
//!
//! Everything here is what `oslo plugin` does; the CLI above it parses words and prints. Kept apart
//! so that the rules — what a source may be, when a name collides, what `remove` leaves behind —
//! are testable without spelling them as terminal output.

use super::index::{self, Installed};
use super::{directory, manifest, trust};
use std::path::{Path, PathBuf};

/// Where a plugin is being installed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A directory on this machine, copied in.
    Path(PathBuf),
    /// A git repository at a revision, cloned in.
    ///
    /// **The revision is not optional.** `oslo plugin install github:user/repo` with no revision
    /// would install whatever the branch says today and something else tomorrow, and the trust hash
    /// would refuse to load it the morning after every upstream commit — which teaches people to
    /// run `oslo plugin allow` without reading, and that is worse than having no gate.
    Git { url: String, revision: String },
}

impl Source {
    /// Read the word after `oslo plugin install`.
    pub fn parse(word: &str) -> Result<Source, String> {
        let Some(rest) = word
            .strip_prefix("github:")
            .map(|rest| (rest, "https://github.com/"))
            .map(|(rest, host)| format!("{host}{rest}"))
            .or_else(|| {
                (word.starts_with("git@") || word.starts_with("https://")).then(|| word.to_string())
            })
        else {
            let path = PathBuf::from(word);
            return if path.is_dir() {
                Ok(Source::Path(path))
            } else {
                Err(format!("{word}: not a directory, and not a git URL"))
            };
        };
        let (url, revision) = rest
            .rsplit_once('@')
            // `git@github.com:…` has an `@` of its own, which is not a revision separator.
            .filter(|(url, revision)| !revision.contains('/') && !url.is_empty())
            .ok_or_else(|| {
                format!("{word}: name a revision, as in github:user/repo@<commit-or-tag>")
            })?;
        Ok(Source::Git {
            url: url.to_string(),
            revision: revision.to_string(),
        })
    }
}

/// What an install would do, decided before anything is written.
pub struct Planned {
    pub manifest: manifest::Manifest,
    pub hash: String,
    /// Names this plugin would reserve that another installed one already has.
    pub conflicts: Vec<String>,
}

/// Read and check a candidate plugin directory against what is already installed.
///
/// **Nothing here runs the plugin.** The manifest is evaluated in an interpreter with no `oslo` in
/// it, and the hash is over bytes — so a plugin is inspected, and its names and its hash shown, all
/// before anybody has decided to trust it.
pub fn plan(candidate: &Path, installed: &[Installed]) -> Result<Planned, String> {
    let manifest = manifest::read(candidate)?;
    // **Refused here rather than installed and left to fail later.** A plugin that needs a newer
    // oslo than this one is not going to start working because it was copied in.
    if let Some(requirement) = &manifest.requires
        && !manifest::satisfied(requirement, oslo_base::version::current())?
    {
        return Err(format!(
            "{} needs oslo {requirement} and this is {}",
            manifest.name,
            oslo_base::version::current()
        ));
    }
    let hash = trust::hash_of(candidate)?;
    let conflicts = installed
        .iter()
        .filter(|other| other.name != manifest.name)
        .flat_map(|other| other.names())
        .filter(|name| manifest.names().any(|wanted| wanted == *name))
        .cloned()
        .collect();
    Ok(Planned {
        manifest,
        hash,
        conflicts,
    })
}

/// Copy a directory tree, refusing symlinks.
///
/// Symlinks are skipped for the reason [`trust`] skips them: a link out of the plugin's directory is
/// content the install never saw and the hash cannot cover.
pub fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|error| format!("{}: {error}", to.display()))?;
    let entries =
        std::fs::read_dir(from).map_err(|error| format!("{}: {error}", from.display()))?;
    for entry in entries.flatten() {
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        let (source, destination) = (entry.path(), to.join(entry.file_name()));
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            // `.git` is a repository, not the plugin. Copying it would put a second checkout under
            // the plugin directory and make every hash walk it.
            if entry.file_name() == ".git" {
                continue;
            }
            copy_tree(&source, &destination)?;
        } else {
            std::fs::copy(&source, &destination)
                .map_err(|error| format!("{}: {error}", source.display()))?;
        }
    }
    Ok(())
}

/// Record a plugin in the index, replacing any entry of the same name.
pub fn remember(installed: Installed) -> Result<(), String> {
    let mut entries = index::read();
    entries.retain(|other| other.name != installed.name);
    entries.push(installed);
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    index::write(&entries)
}

/// Take a plugin out of the index and delete its directory.
///
/// **Its database is left where it is**, and the caller says so. That file is the user's data: a
/// plugin manager that deleted a vault because somebody reinstalled the plugin that filled it is one
/// nobody should run. `oslo.db` will hand the same database back if it is installed again.
pub fn remove(name: &str) -> Result<PathBuf, String> {
    let mut entries = index::read();
    if !entries.iter().any(|installed| installed.name == name) {
        return Err(format!("{name}: not installed"));
    }
    entries.retain(|installed| installed.name != name);
    index::write(&entries)?;
    let directory = directory()
        .ok_or_else(|| "nowhere to look".to_string())?
        .join(name);
    if directory.is_dir() {
        std::fs::remove_dir_all(&directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?;
    }
    Ok(directory)
}

#[cfg(test)]
#[path = "install/tests.rs"]
mod tests;
