//! What has already been asked of nix, and how long the answer stays good for.
//!
//! Two caches, because they hold different things for different callers, and one key idea shared
//! between them: **an answer is stale when the flake moved, not when a timer went off.** Editing
//! `flake.nix` re-evaluates on the next call and nothing else does.
//!
//! | | keyed on | kept in |
//! |---|---|---|
//! | the dev shell's environment | argv + the flake files | the project's `.direnv/dev-env.json` |
//! | any `--json` document | argv + the flake files + where you are | `$XDG_CACHE_HOME/oslo/nix/` |
//!
//! Here rather than in `nix_shell.rs` because that file was at 546 of the 600 a file may have, and
//! caching is a subject of its own — see `scripts/check-loc.sh`.

use std::path::{Path, PathBuf};

/// The directory the project's `.direnv` belongs in.
///
/// **The rc file's own, not the shell's.** These paths used to be relative to the working
/// directory, so arriving in a subdirectory of a project scattered a `.direnv` into whichever
/// directory the shell happened to be standing in — and the profile written there was a different
/// GC root each time.
/// **Asked of `direnv` when there is one, and only then.** A project's root is the directory
/// holding the rc file, which is a `direnv` idea — in a build without it there is no rc file to
/// own anything, so the working directory is the honest answer and the only one available.
pub(super) fn root() -> PathBuf {
    let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    #[cfg(feature = "direnv")]
    {
        crate::direnv::find::applicable(&here)
            .and_then(|rc| crate::direnv::find::owner(&rc))
            .unwrap_or(here)
    }
    #[cfg(not(feature = "direnv"))]
    here
}

/// The files that decide what the dev shell contains.
const INPUTS: &[&str] = &["flake.nix", "flake.lock", "shell.nix", "default.nix"];

/// What the cached answer was computed from: the arguments, and every input as it stood.
///
/// Length and mtime, the same pair the rc files are stamped with and for the same reason — mtime
/// alone has one-second granularity on some filesystems.
fn key(root: &Path, args: &[String]) -> String {
    let mut key = format!("1 {}", args.join(" "));
    for name in INPUTS {
        let stamp = std::fs::metadata(root.join(name))
            .ok()
            .map(|meta| {
                let when = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                format!("{}:{when}", meta.len())
            })
            .unwrap_or_else(|| "-".to_string());
        key.push(' ');
        key.push_str(&stamp);
    }
    key
}

/// Where the evaluated environment is kept between runs.
fn cache(root: &Path) -> PathBuf {
    root.join(".direnv/dev-env.json")
}

/// **The evaluation, remembered.** `nix print-dev-env` costs about half a second on a warm store
/// and several on a cold one, and it is asked the same question every time: this project's flake
/// has not moved since the last arrival. That is once per `cd` into the project and once per new
/// shell — a new pane, a nested `oslo` — which is often enough to be the slowest thing the shell
/// does all day.
///
/// Keyed on the inputs rather than timed out, so editing `flake.nix` re-evaluates immediately and
/// nothing else does. `direnv reload` drops it outright, which is the escape hatch for the case
/// this cannot see: nix itself, or something the flake reads that is not one of [`INPUTS`].
pub(super) fn cached(args: &[String]) -> Option<String> {
    let root = root();
    let text = std::fs::read_to_string(cache(&root)).ok()?;
    let (head, body) = text.split_once('\n')?;
    (head == key(&root, args)).then(|| body.to_string())
}

pub(super) fn remember(args: &[String], json: &str) {
    let root = root();
    let path = cache(&root);
    write_privately(&path, &format!("{}\n{json}", key(&root, args)));
}

/// Drop everything remembered about this project, for `direnv reload`.
///
/// **Both caches, which it did not always do.** This removed `dev-env.json` alone, so a reload
/// re-evaluated the dev shell and then went on serving pre-reload answers to `oslo.nix.run{…,
/// cache = true}` — a prompt segment could still report the flake as dirty after it was committed.
/// `reload` has to mean reload, or it means nothing.
pub fn forget() {
    let root = root();
    let _ = std::fs::remove_file(cache(&root));
    // The whole directory: the documents of one project live together precisely so this can drop
    // them without knowing which questions were ever asked.
    if let Some(base) = base() {
        let _ = std::fs::remove_dir_all(project_dir(&base, &root));
    }
}

/// A document `oslo.nix.run{…, cache = true}` asked for, or nothing if it must be fetched.
///
/// **Opt-in, and this is why.** nix keeps an evaluation cache of its own, and it is good: warm,
/// `flake metadata` is 27 ms and `flake show` 34 ms, against 264 and 455 cold. So the default is to
/// ask nix, which is nearly always answering from memory anyway. What this is for is the cold case
/// and the genuinely expensive question — `nix search nixpkgs ripgrep --json` took 46 seconds here.
///
/// A caller that asks `store info` wants the store's answer, not last week's, and cannot be given a
/// cache it did not ask for.
pub fn document(argv: &[String]) -> Option<String> {
    document_in(&base()?, &root(), argv)
}

/// Keep `json` as the answer to `argv` until the flake moves.
pub fn keep(argv: &[String], json: &str) {
    if let Some(base) = base() {
        keep_in(&base, &root(), argv, json);
    }
}

/// [`document`], against named directories.
///
/// **The project root is a parameter for the reason [`key`]'s is**: the process has one working
/// directory and every test in the crate shares it, so a test that moved it to ask this question
/// would answer a different one whenever a sibling test moved it back. That is not hypothetical —
/// these two passed alone and failed in the full suite until they stopped using `cd`.
fn document_in(base: &Path, root: &Path, argv: &[String]) -> Option<String> {
    let text = std::fs::read_to_string(document_path(base, root, argv)).ok()?;
    let (head, body) = text.split_once('\n')?;
    (head == key(root, argv)).then(|| body.to_string())
}

/// [`keep`], against named directories.
fn keep_in(base: &Path, root: &Path, argv: &[String], json: &str) {
    let path = document_path(base, root, argv);
    write_privately(&path, &format!("{}\n{json}", key(root, argv)));
}

/// Where oslo's regenerable data lives: `$XDG_CACHE_HOME`, or `~/.cache`.
///
/// **A cache directory, not the `$XDG_DATA_HOME` the rest of oslo writes to.** What lives there —
/// history, the model, direnv's allow list — is state the user accumulated and would miss. Every
/// byte here can be recreated by asking nix again, which is precisely what `$XDG_CACHE_HOME` is
/// for, and it means the whole directory can be deleted at any moment without losing anything.
///
/// **Not in the project.** The dev-env cache above sits in `.direnv/` because direnv owns that
/// directory and the project already has it. With the `nix` feature alone there is no direnv, so
/// the equivalent would be oslo dropping an unexplained directory into a stranger's repository and
/// leaving them to `.gitignore` it.
fn base() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
}

/// Everything remembered about one project: `<base>/oslo/nix/<digest of its root>`.
///
/// **A directory per project rather than one flat pile**, so [`forget`] can drop a project's
/// answers without enumerating the questions. It also keeps two projects asking the same question
/// apart, instead of one entry that alternates between them.
fn project_dir(base: &Path, root: &Path) -> PathBuf {
    base.join("oslo/nix")
        .join(digest(root.as_os_str().as_encoded_bytes()))
}

/// One document's file, named for the question that produced it.
fn document_path(base: &Path, root: &Path, argv: &[String]) -> PathBuf {
    project_dir(base, root).join(format!("{}.json", digest(argv.join("\u{0}").as_bytes())))
}

/// A short, stable, filesystem-safe name for a byte string.
fn digest(of: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(of);
    hasher
        .finalize()
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Write `text` where only its owner can read it, making the directory if it is missing.
///
/// **Owner-only.** The dev-env dump is a verbatim copy of a dev shell's environment, which for a
/// good many projects means tokens and connection strings, and a `--json` document can be anything
/// a flake evaluates to. The umask would otherwise have left both world-readable on a shared
/// machine.
fn write_privately(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::write(path, text).is_err() {
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(test)]
#[path = "cache/tests.rs"]
mod tests;
