//! `~/.config/direnv/direnvrc` — the file you write once and every project gets.
//!
//! direnv sources this before the `.envrc`, and it is where people put the `use_` and `layout_`
//! functions their projects then call. That is the reason `use` dispatches by name rather than
//! matching a fixed list: a `direnvrc` defining `use_java` makes `use java` work in a repository
//! whose `.envrc` was written by somebody who has never heard of oslo, which is the entire point
//! of reading these files at all.
//!
//! Also read is `direnv/lib/*.sh`, in name order, which is direnv's other extension point and how
//! its plugins are distributed.
//!
//! # Not gated by the allow store
//!
//! Every other file this module runs is asked about first, because it arrives with a repository and
//! a repository can come from anyone. This one is yours: it lives under your own config directory,
//! it is not part of any project, and prompting to allow a file you wrote by hand would train the
//! habit of saying yes — which is the one thing the allow gate cannot afford.

use crate::env::Environment;
use std::path::PathBuf;

/// The directory direnv keeps its configuration in.
///
/// `$DIRENV_CONFIG` wins so that a session can point somewhere else, then `$XDG_CONFIG_HOME`, then
/// `~/.config` — direnv's own order, and the reason a file written for it is found without being
/// moved.
fn config_dir() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("DIRENV_CONFIG")
        && !explicit.is_empty()
    {
        return Some(PathBuf::from(explicit));
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg).join("direnv"));
    }
    let home = std::env::var("HOME").ok().filter(|home| !home.is_empty())?;
    Some(PathBuf::from(home).join(".config/direnv"))
}

/// Every file to source before an `.envrc`, in order.
pub fn files() -> Vec<PathBuf> {
    let Some(dir) = config_dir() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let rc = dir.join("direnvrc");
    if rc.is_file() {
        found.push(rc);
    }
    // Sorted, because a directory listing is in whatever order the filesystem gives it and two
    // plugins that touch the same variable must not depend on that.
    if let Ok(entries) = std::fs::read_dir(dir.join("lib")) {
        let mut libs: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|end| end == "sh"))
            .collect();
        libs.sort();
        found.extend(libs);
    }
    found
}

/// Source them all into `env`, reporting nothing: these are the user's own and are expected to be
/// quiet. A failure in one does not stop the next, because a broken plugin should not take the
/// whole directory environment with it.
pub fn load(env: &mut Environment) -> Vec<PathBuf> {
    let files = files();
    for file in &files {
        let _ = crate::env::builtins::builtin_source(
            env,
            &["source".to_string(), file.to_string_lossy().into_owned()],
        );
    }
    files
}
