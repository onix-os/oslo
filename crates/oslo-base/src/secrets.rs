//! Secrets the shell can hold: encrypted at rest with age, decrypted only when something asks.
//!
//! # Why a shell at all
//!
//! Every secret a person keeps ends up in a shell eventually — exported for one command, pasted
//! into a `curl`, read out of a `.env` that should never have been written. The shell is where they
//! are *used*, and it is the one program in the chain that can hold a value in memory, hand it to
//! one command, and never write it down. Anything outside the shell has to put the value somewhere
//! the shell can read it, which means a file, which means a plaintext file.
//!
//! # age, and not a keyring
//!
//! [age](https://github.com/str4d/rage) is a file encryption format with one key type, no options
//! to get wrong, and a Rust implementation with no C in it — which is the only kind that can go in
//! a static musl binary that is also `/bin/sh`. A system keyring is the alternative and is the
//! wrong shape here: it needs a daemon, a session bus and a desktop, none of which exist on a
//! server, in a container, or in the initramfs where this shell is supposed to work.
//!
//! # What is on disk, and why the key is not next to it
//!
//! ```text
//! ~/.local/share/oslo/secrets/NAME.age      one secret, encrypted — safe to commit
//! ~/.local/state/oslo/identity              the key, mode 0600 — never
//! ```
//!
//! **Two directories, because they have opposite rules.** The point of encrypting a store is that
//! the store can then go where files go — a dotfiles repository, a backup, a synced directory. A
//! key sitting inside it turns that from a feature into the worst kind of accident: the ciphertext
//! and the thing that opens it, committed together, in one `git add -A`.
//!
//! So the key lives under `$XDG_STATE_HOME` — a different tree, whose whole definition is state a
//! machine keeps to itself — and `$OSLO_SECRET_IDENTITY` moves it anywhere else: a USB stick, an
//! encrypted volume, `~/.ssh` beside the other private keys. `identity_path` is the one place that
//! decides, and `identity_in_a_repository` is the check that says so out loud when the answer turns
//! out to be under a `.git` anyway — because a home directory that is itself a repository is a
//! thing people do.
//!
//! The identity is a file at all because the alternative is asking for a passphrase on every read,
//! and a shell that asks fifty times a day teaches you to keep the value somewhere else. Protecting
//! it is the filesystem's job, the same as `~/.ssh/id_ed25519`.

use age::secrecy::ExposeSecret;
use age::x25519;
use std::io::{Read, Write};
use std::path::PathBuf;

/// `$XDG_DATA_HOME/oslo/secrets`, or `~/.local/share/oslo/secrets`.
pub fn directory() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(base.join("oslo/secrets"))
}

/// Where one secret lives.
///
/// A name is a filename, so it may not reach out of the directory it is written into: `..` and `/`
/// are refused rather than sanitised, because a secret quietly stored somewhere other than where it
/// was asked for is worse than an error.
pub fn path(name: &str) -> Result<PathBuf, String> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\0')
        || name == ".."
        || name.starts_with('.')
    {
        return Err(format!("{name:?} is not a usable name for a secret"));
    }
    Ok(directory()
        .ok_or("no $XDG_DATA_HOME and no $HOME, so there is nowhere to keep secrets")?
        .join(format!("{name}.age")))
}

/// Where the private key is: `$OSLO_SECRET_IDENTITY`, else `$XDG_STATE_HOME/oslo/identity`, else
/// `~/.local/state/oslo/identity`.
///
/// **Deliberately not under [`directory`].** See the note at the top of this file: the store is
/// meant to be committable, and a key inside it would be committed with it.
pub fn identity_path() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("OSLO_SECRET_IDENTITY")
        && !named.is_empty()
    {
        return Some(PathBuf::from(named));
    }
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    Some(base.join("oslo/identity"))
}

/// The repository the identity is inside, when it is inside one.
///
/// A key under a `.git` is a key one `git add -A` away from being published, and the person it
/// happens to is never the person who chose it — it is somebody who moved their state directory
/// into a dotfiles repository a year later. Walking up costs one `exists` per parent and is only
/// done when a secret is used.
pub fn identity_in_a_repository() -> Option<PathBuf> {
    let path = identity_path()?;
    let mut here = path.parent()?;
    loop {
        if is_repository(&here.join(".git")) {
            return Some(here.to_path_buf());
        }
        here = here.parent()?;
    }
}

/// Whether `.git` is a repository rather than a directory that merely has the name.
///
/// **Measured, because the first version of this cried wolf on the machine it was written on.**
/// `~/.git` there is an empty directory left behind by something, and `git -C ~ rev-parse` answers
/// "not a git repository" — so a bare `exists()` warned that a key was about to be committed to a
/// repository that does not exist. A real one is a directory with `HEAD` in it, or a *file* saying
/// where the directory is, which is what a worktree and a submodule have.
fn is_repository(dot_git: &std::path::Path) -> bool {
    dot_git.is_file() || dot_git.join("HEAD").exists()
}

/// The key this machine encrypts to, read from disk or created on first use.
///
/// Mode `0600` from the moment it exists: written to a temporary file that is already private and
/// then renamed, so there is no instant where the key is readable by anybody else.
fn identity() -> Result<x25519::Identity, String> {
    let path = identity_path()
        .ok_or("no $XDG_STATE_HOME and no $HOME, so there is nowhere to keep a key")?;
    if let Ok(text) = std::fs::read_to_string(&path) {
        return text
            .trim()
            .parse::<x25519::Identity>()
            .map_err(|e| format!("{}: {e}", path.display()));
    }

    let directory = path
        .parent()
        .ok_or_else(|| format!("{}: has no directory to be in", path.display()))?;
    std::fs::create_dir_all(directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    let fresh = x25519::Identity::generate();
    let scratch = directory.join("identity.new");
    write_private(&scratch, fresh.to_string().expose_secret())?;
    std::fs::rename(&scratch, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(fresh)
}

/// Write `text` where only this user can read it, before anything is in it.
fn write_private(path: &std::path::Path, text: &str) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(b"\n")
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Keep `value` under `name`, encrypted to this machine's key.
pub fn set(name: &str, value: &[u8]) -> Result<(), String> {
    let path = path(name)?;
    let identity = identity()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }

    let encryptor = age::Encryptor::with_recipients([&identity.to_public() as _].into_iter())
        .map_err(|e| format!("{e}"))?;
    let mut ciphertext = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut ciphertext)
        .map_err(|e| format!("{e}"))?;
    writer.write_all(value).map_err(|e| format!("{e}"))?;
    writer.finish().map_err(|e| format!("{e}"))?;

    // The ciphertext is not a secret, but the file is still written privately: what it *is* is
    // still information — a size, a modification time, the fact that this name exists at all.
    let scratch = path.with_extension("age.new");
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&scratch)
            .map_err(|e| format!("{}: {e}", scratch.display()))?;
        file.write_all(&ciphertext)
            .map_err(|e| format!("{}: {e}", scratch.display()))?;
    }
    std::fs::rename(&scratch, &path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The value kept under `name`.
pub fn get(name: &str) -> Result<Vec<u8>, String> {
    let path = path(name)?;
    let ciphertext =
        std::fs::read(&path).map_err(|e| format!("{name}: {e}", name = path.display(), e = e))?;
    let identity = identity()?;

    let decryptor = age::Decryptor::new(&ciphertext[..]).map_err(|e| format!("{e}"))?;
    let mut reader = decryptor
        .decrypt([&identity as &dyn age::Identity].into_iter())
        .map_err(|e| format!("{e}"))?;
    let mut value = Vec::new();
    reader.read_to_end(&mut value).map_err(|e| format!("{e}"))?;
    Ok(value)
}

/// Every name kept here, sorted.
pub fn names() -> Vec<String> {
    let Some(directory) = directory() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".age").map(str::to_string)
        })
        .collect();
    names.sort();
    names
}

/// Forget the secret kept under `name`.
pub fn forget(name: &str) -> Result<(), String> {
    let path = path(name)?;
    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
#[path = "secrets/tests.rs"]
mod tests;
