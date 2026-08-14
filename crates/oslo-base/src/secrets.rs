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
//! # A key and an AEAD, not a keyring
//!
//! [age](https://github.com/str4d/rage) is a file encryption format with one key type, no options
//! to get wrong, and a Rust implementation with no C in it — which is the only kind that can go in
//! a static musl binary that is also `/bin/sh`. A system keyring is the alternative and is the
//! wrong shape here: it needs a daemon, a session bus and a desktop, none of which exist on a
//! server, in a container, or in the initramfs where this shell is supposed to work.
//!
//! # A store is three things
//!
//! A [`Store`] is a directory, an ordered list of [`KeySource`]s, and the [`Crypto`] that seals and
//! opens its files — oslo's own, another program, or a Lua hook. Every one of them is configurable
//! in [`conf`]; an install that has configured nothing gets one store called `user` and one key
//! file, and never has to be told anything.
//!
//! # What is on disk, and why the key is not next to it
//!
//! ```text
//! ~/.local/share/oslo/secrets/NAME.sealed   one secret, encrypted — safe to commit
//! ~/.local/state/oslo/key                   the key, mode 0600 — never
//! ~/.local/state/oslo/secrets.conf          which stores exist — beside the key, and why is in `conf`
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

pub mod cipher;
pub mod conf;
pub mod crypto;
pub mod hooked;
pub mod key;
#[cfg(feature = "crypt")]
pub mod native;

use std::io::Write;
use std::path::{Path, PathBuf};

pub use cipher::Cipher;
pub use crypto::Crypto;
pub use key::KeySource;

/// What one secret's file is called, after its name.
///
/// Not `.age`: these have not been age files since oslo grew a mechanism of its own, and a suffix
/// that named a format it is not would send somebody to the wrong tool to open one.
pub const KEPT: &str = ".sealed";

/// The store every install has, and the one `oslo secret` means when nothing says otherwise.
pub const USER: &str = "user";

/// A plugin's own store is named for it, under a prefix Lua cannot type.
pub const PLUGIN: &str = "plugin.";

/// A directory, what opens it, and who it is written for.
#[derive(Debug, Clone)]
pub struct Store {
    pub name: String,
    pub directory: PathBuf,
    pub keys: Vec<KeySource>,
    /// What seals and opens its files: oslo's own age, another program, or a Lua hook.
    pub crypto: Crypto,
}

impl Store {
    /// The store called `name`, as `secrets.conf` has it, filled in with the defaults for whatever
    /// it does not say.
    pub fn named(name: &str) -> Result<Store, String> {
        conf::valid_store_name(name)?;
        let declared = conf::read()?;
        let section = declared.section(name);
        let mut keys = section.map(|s| s.keys.clone()).unwrap_or_default();
        if keys.is_empty() {
            keys.push(KeySource::File(identity_path().ok_or(NOWHERE_FOR_A_KEY)?));
        }
        let cipher = section.map(|s| s.cipher.clone()).unwrap_or_default();
        let crypto = Crypto::of(section.is_some_and(|s| s.hooked), &cipher)
            .map_err(|e| format!("{name}: {e}"))?;
        if name.starts_with(PLUGIN)
            && (keys.iter().any(KeySource::is_external) || cipher.is_external())
        {
            return Err(format!("{name}: a plugin's store may not run a command"));
        }
        Ok(Store {
            directory: match section.and_then(|s| s.directory.clone()) {
                Some(directory) => directory,
                None => default_directory(name)?,
            },
            name: name.to_string(),
            keys,
            crypto,
        })
    }

    /// The store this invocation means: what was asked for, else `$OSLO_SECRET_STORE`, else the
    /// `default` line, else [`USER`].
    pub fn selected(asked: Option<&str>) -> Result<Store, String> {
        if let Some(name) = asked {
            return Store::named(name);
        }
        if let Some(name) = std::env::var_os("OSLO_SECRET_STORE")
            && !name.is_empty()
        {
            return Store::named(&name.to_string_lossy());
        }
        match conf::read()?.default {
            Some(name) => Store::named(&name),
            None => Store::named(USER),
        }
    }

    /// Why this store cannot be used *in this process*, when that is the case.
    ///
    /// **Asked before anything touches a file**, or a hook-backed store in a script would report a
    /// missing file — true, and not the reason. Only `crypto hook` can be unreachable: age and a
    /// command both work wherever the binary does.
    pub fn unreachable_here(&self) -> Option<String> {
        match self.crypto {
            Crypto::Hook => hooked::unreachable_here(&self.name),
            _ => None,
        }
    }

    /// Where one secret lives.
    ///
    /// A name is a filename, so it may not reach out of the directory it is written into: `..` and
    /// `/` are refused rather than sanitised, because a secret quietly stored somewhere other than
    /// where it was asked for is worse than an error.
    pub fn path(&self, name: &str) -> Result<PathBuf, String> {
        if name.is_empty()
            || name.contains('/')
            || name.contains('\0')
            || name == ".."
            || name.starts_with('.')
        {
            return Err(format!("{name:?} is not a usable name for a secret"));
        }
        Ok(self.directory.join(format!("{name}{KEPT}")))
    }

    /// Keep `value` under `name`.
    pub fn set(&self, name: &str, value: &[u8]) -> Result<(), String> {
        if let Some(why) = self.unreachable_here() {
            return Err(why);
        }
        let path = self.path(name)?;
        hooked::touched(true, &self.name, name, true);
        let sealed = self.seal_named(name, value)?;
        hooked::touched(false, &self.name, name, true);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        // The ciphertext is not a secret, but the file is still written privately: what it *is* is
        // still information — a size, a modification time, the fact that this name exists at all.
        let scratch = path.with_extension("new");
        write_private(&scratch, &sealed)?;
        std::fs::rename(&scratch, &path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The value kept under `name`.
    pub fn get(&self, name: &str) -> Result<Vec<u8>, String> {
        if let Some(why) = self.unreachable_here() {
            return Err(why);
        }
        let path = self.path(name)?;
        let ciphertext = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        hooked::touched(true, &self.name, name, false);
        let value = self.unseal_named(name, &ciphertext);
        hooked::touched(false, &self.name, name, false);
        value
    }

    /// Every name kept here, sorted.
    pub fn names(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.strip_suffix(KEPT).map(str::to_string)
            })
            .collect();
        names.sort();
        names
    }

    /// Forget the secret kept under `name`.
    pub fn forget(&self, name: &str) -> Result<(), String> {
        let path = self.path(name)?;
        std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Encrypt to every recipient, without storing it anywhere.
    ///
    /// **A store with an `encrypt command` never reaches oslo's own age**: who that program
    /// encrypts to, and in what format, is its business. That is what makes a key oslo cannot
    /// compute — one held in hardware — usable at all.
    pub fn seal(&self, value: &[u8]) -> Result<Vec<u8>, String> {
        self.seal_named("", value)
    }

    /// The same, telling whatever does the work which secret this is.
    ///
    /// The name is empty for an anonymous seal — `oslo.secret.seal`, which encrypts something the
    /// caller keeps somewhere else of its own.
    pub fn seal_named(&self, name: &str, value: &[u8]) -> Result<Vec<u8>, String> {
        match &self.crypto {
            Crypto::Hook => return hooked::encrypt(&self.name, name, value),
            Crypto::Command(cipher) => {
                if let Some(argv) = &cipher.encrypt {
                    return cipher::through(argv, value);
                }
            }
            Crypto::Native => {}
        }
        self.seal_natively(value)
    }

    /// oslo's own, in this process.
    #[cfg(feature = "crypt")]
    fn seal_natively(&self, value: &[u8]) -> Result<Vec<u8>, String> {
        native::seal(&self.key_to_write_with()?, value)
    }

    /// This build carries no crypto of its own, so a store has to name one.
    #[cfg(not(feature = "crypt"))]
    fn seal_natively(&self, _value: &[u8]) -> Result<Vec<u8>, String> {
        Err(no_native(&self.name))
    }

    /// Decrypt with whichever key opens it.
    ///
    /// **Native sources first, and external ones only if none of them matched.** `age` stops at the
    /// first identity that fits, so a store whose file key opens the file never runs the program
    /// another key source names — no `$PATH` walk, no fork, and a cron job on a machine that cannot
    /// reach the other key degrades instead of hanging on it.
    pub fn unseal(&self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        self.unseal_named("", ciphertext)
    }

    /// The same, telling whatever does the work which secret this is.
    pub fn unseal_named(&self, name: &str, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        match &self.crypto {
            Crypto::Hook => return hooked::decrypt(&self.name, name, ciphertext),
            Crypto::Command(cipher) => {
                if let Some(argv) = &cipher.decrypt {
                    return cipher::through(argv, ciphertext);
                }
            }
            Crypto::Native => {}
        }
        self.unseal_natively(ciphertext)
    }

    /// oslo's own, in this process.
    ///
    /// **File keys before program keys**, so a store that opens with a file never runs the program
    /// another key source names: no `$PATH` walk, no fork, and a cron job on a machine that cannot
    /// reach the other key degrades instead of hanging on it.
    #[cfg(feature = "crypt")]
    fn unseal_natively(&self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        let mut tried = 0;
        for external in [false, true] {
            for key in self.keys_of(external)? {
                tried += 1;
                if let Ok(value) = native::unseal(&key, ciphertext) {
                    return Ok(value);
                }
            }
        }
        Err(self.why_it_did_not_open(tried))
    }

    #[cfg(not(feature = "crypt"))]
    fn unseal_natively(&self, _ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        Err(no_native(&self.name))
    }

    /// Re-encrypt everything here to the recipients as they now stand, and answer how many moved.
    pub fn rotate(&self) -> Result<usize, String> {
        let names = self.names();
        let mut values = Vec::with_capacity(names.len());
        for name in &names {
            values.push((name.clone(), self.get(name)?));
        }
        for (name, value) in &values {
            self.set(name, value)?;
        }
        Ok(values.len())
    }

    /// The keys this store can offer, of one kind.
    #[cfg(feature = "crypt")]
    pub fn keys_of(&self, external: bool) -> Result<Vec<[u8; 32]>, String> {
        let mut found = Vec::new();
        for source in self.keys.iter().filter(|s| s.is_external() == external) {
            match source.key() {
                Ok(Some(key)) => found.push(key),
                Ok(None) => {}
                // A key source that cannot answer is not the end of the list: the next one may be
                // the one that opens this file, and the failure is reported only if none does.
                Err(_) if self.keys.len() > 1 => {}
                Err(e) => return Err(e),
            }
        }
        Ok(found)
    }

    /// The first key file this store would look in, which is where `key init` makes one.
    pub fn key_file(&self) -> Option<PathBuf> {
        self.keys.iter().find_map(|source| match source {
            KeySource::File(path) => Some(path.clone()),
            KeySource::Command(_) => None,
        })
    }

    /// The key this store writes with, made now if this is the first time.
    ///
    /// The first that answers, file keys before program ones — so a store with a spare key on a USB
    /// stick still writes with the one it always used.
    #[cfg(feature = "crypt")]
    pub fn key_to_write_with(&self) -> Result<[u8; 32], String> {
        for external in [false, true] {
            if let Some(key) = self.keys_of(external)?.first() {
                return Ok(*key);
            }
        }
        match self.key_file() {
            Some(path) => key::generate(&path),
            None => Err(format!(
                "{}: no key, and no key file to make one in",
                self.name
            )),
        }
    }

    /// Why nothing opened it, in the terms of what this store was told to try.
    #[cfg(feature = "crypt")]
    fn why_it_did_not_open(&self, tried: usize) -> String {
        let skipped = key::no_exec() && self.keys.iter().any(KeySource::is_external);
        if skipped {
            return format!(
                "no key opened it. $OSLO_SECRET_NO_EXEC is set, so {} of {} key sources were not run",
                self.keys.iter().filter(|s| s.is_external()).count(),
                self.keys.len()
            );
        }
        if tried == 0 {
            return format!("{}: no key to open it with", self.name);
        }
        format!("no key opened it, of the {tried} this store has")
    }
}

/// What a build with no crypto of its own says when a store did not name one.
///
/// **Named rather than implied.** A build without `age` has the whole of the filing — the store, the
/// names, `run`, the lazy variable, the Lua API — and none of the encryption, so the one thing it
/// cannot do has to say which one thing that is.
#[cfg(not(feature = "crypt"))]
fn no_native(store: &str) -> String {
    format!(
        "{store}: this oslo has no built-in crypto. Give the store an `encrypt command` and a \
         `decrypt command` — `age`, `gpg`, `systemd-creds`, whatever this machine already has — \
         or `crypto hook` to do it in Lua"
    )
}

const NOWHERE_FOR_A_KEY: &str =
    "no $XDG_STATE_HOME and no $HOME, so there is nowhere to keep a key";

/// `$XDG_DATA_HOME/oslo`, or `~/.local/share/oslo`.
fn data_directory() -> Option<PathBuf> {
    base("XDG_DATA_HOME", ".local/share").map(|base| base.join("oslo"))
}

/// `$XDG_STATE_HOME/oslo`, or `~/.local/state/oslo`.
fn state_directory() -> Option<PathBuf> {
    base("XDG_STATE_HOME", ".local/state").map(|base| base.join("oslo"))
}

fn base(variable: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
}

/// Where a store keeps its files when it does not say.
///
/// The `user` store keeps the directory it has always had. A named one gets its own under
/// `stores/`, and a plugin's under `plugins/`, beside the `.kv` that `oslo.db` makes for it — so
/// uninstalling a plugin stays an `rm -r` rather than a migration.
fn default_directory(name: &str) -> Result<PathBuf, String> {
    let oslo = data_directory()
        .ok_or("no $XDG_DATA_HOME and no $HOME, so there is nowhere to keep secrets")?;
    Ok(match name {
        USER => oslo.join("secrets"),
        _ => match name.strip_prefix(PLUGIN) {
            Some(plugin) => oslo.join("plugins").join(format!("{plugin}.secrets")),
            None => oslo.join("stores").join(name),
        },
    })
}

/// `$XDG_DATA_HOME/oslo/secrets`, or `~/.local/share/oslo/secrets`.
pub fn directory() -> Option<PathBuf> {
    default_directory(USER).ok()
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
    Some(state_directory()?.join("key"))
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
fn is_repository(dot_git: &Path) -> bool {
    dot_git.is_file() || dot_git.join("HEAD").exists()
}

/// Write bytes where only this user can read them, before anything is in them.
///
/// **Bytes, and nothing appended.** It writes age ciphertext as well as identities, and a newline
/// added to the end of an age file is a file the format does not accept back.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Where one secret lives in the `user` store.
pub fn path(name: &str) -> Result<PathBuf, String> {
    Store::named(USER)?.path(name)
}

/// Keep `value` under `name` in the `user` store.
pub fn set(name: &str, value: &[u8]) -> Result<(), String> {
    Store::named(USER)?.set(name, value)
}

/// The value kept under `name` in the `user` store.
pub fn get(name: &str) -> Result<Vec<u8>, String> {
    Store::named(USER)?.get(name)
}

/// Every name in the `user` store.
pub fn names() -> Vec<String> {
    Store::named(USER)
        .map(|store| store.names())
        .unwrap_or_default()
}

/// Forget the secret kept under `name` in the `user` store.
pub fn forget(name: &str) -> Result<(), String> {
    Store::named(USER)?.forget(name)
}

#[cfg(test)]
#[path = "secrets/tests.rs"]
mod tests;
