//! Where a store's keys come from.
//!
//! # Two kinds, in a deliberate order
//!
//! A [`KeySource::File`] is read; a [`KeySource::Command`] is *run*, and its standard output is the
//! key. The second exists because the alternative is compiling every way a person might hold a
//! key into a shell that is meant to be `/bin/sh` — a password manager, a smartcard wrapper,
//! whatever they already use. It costs nothing when it is not configured.
//!
//! Native sources are always tried first, and a store that decrypts with a file key never runs
//! anything: `age`'s decryptor stops at the first identity that matches, so the ordering is a
//! latency property and a security one at once.
//!
//! # What is fenced, and how
//!
//! * **argv, never a shell string.** Nothing reaches `/bin/sh`, so there is no quoting layer to get
//!   wrong and no `$(…)` in a configuration file.
//! * **Never in a `plugin.*` store.** A plugin's own store cannot fork; that is a decision its
//!   owner makes at a command line.
//! * **`$OSLO_SECRET_NO_EXEC`** set to anything non-empty skips every command source and names it
//!   in the failure. Exported once by a cron job or a container, inherited by every child, it makes
//!   "this will not fork" something to assert rather than infer.

use std::path::PathBuf;

/// Where one key comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    /// The profile's own key, with this store's key derived from it.
    ///
    /// **The default, and the reason a fresh install needs to be told nothing.** A profile already
    /// has a key — it is what lets two machines agree they share a history — so a store derives its
    /// own from that rather than inventing a second secret to keep track of. Carrying the profile
    /// to another machine therefore carries the secrets with it, which is the same one step.
    Profile,
    File(PathBuf),
    Command(Vec<String>),
}

impl KeySource {
    /// `file PATH`, or `command ARG…`, as written after `key` in `secrets.conf`.
    pub fn parse(rest: &str) -> Result<Self, String> {
        let (kind, rest) = super::conf::split_word(rest);
        match kind {
            "profile" => Ok(KeySource::Profile),
            "file" if !rest.is_empty() => Ok(KeySource::File(PathBuf::from(rest))),
            "command" if !rest.is_empty() => {
                let argv: Vec<String> = rest.split_whitespace().map(str::to_string).collect();
                Ok(KeySource::Command(argv))
            }
            "file" | "command" => Err(format!("`key {kind}` needs something after it")),
            other => Err(format!(
                "a key is `profile`, `file` or `command`, not {other:?}"
            )),
        }
    }

    /// How it is written in the file.
    pub fn line(&self) -> String {
        match self {
            KeySource::Profile => "key profile".to_string(),
            KeySource::File(path) => format!("key file {}", path.display()),
            KeySource::Command(argv) => format!("key command {}", argv.join(" ")),
        }
    }

    /// Whether reaching this key means running another program.
    pub fn is_external(&self) -> bool {
        matches!(self, KeySource::Command(_))
    }

    /// The key, or why not.
    ///
    /// A file that is not there is `Ok(None)` rather than an error: several key sources are a list
    /// of places to look, and a laptop that has one of them is not misconfigured.
    #[cfg(feature = "crypt")]
    pub fn key(&self) -> Result<Option<[u8; 32]>, String> {
        match self {
            KeySource::Profile => Ok(None),
            KeySource::File(path) => match std::fs::read_to_string(path) {
                Ok(text) => super::native::read_secret(&text)
                    .map(Some)
                    .map_err(|e| format!("{}: {e}", path.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(format!("{}: {e}", path.display())),
            },
            KeySource::Command(argv) => KeySource::run(argv).map(Some),
        }
    }

    #[cfg(feature = "crypt")]
    fn run(argv: &[String]) -> Result<[u8; 32], String> {
        if no_exec() {
            return Err(format!(
                "$OSLO_SECRET_NO_EXEC is set, so `{}` was not run",
                argv.join(" ")
            ));
        }
        let (program, rest) = argv.split_first().ok_or("a key command with no program")?;
        let output = std::process::Command::new(program)
            .args(rest)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("{program}: {e}"))?;
        if !output.status.success() {
            let said = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "{program}: exited {}{}",
                output.status.code().unwrap_or(-1),
                if said.trim().is_empty() {
                    String::new()
                } else {
                    format!(": {}", said.trim())
                }
            ));
        }
        let text = String::from_utf8(output.stdout)
            .map_err(|_| format!("{program}: what it printed is not a key"))?;
        super::native::read_secret(&text).map_err(|e| format!("{program}: {e}"))
    }
}

/// Whether every external source is to be skipped.
pub fn no_exec() -> bool {
    std::env::var_os("OSLO_SECRET_NO_EXEC").is_some_and(|value| !value.is_empty())
}

/// Make one where `path` says, mode `0600` from the moment it exists.
#[cfg(feature = "crypt")]
pub fn generate(path: &std::path::Path) -> Result<[u8; 32], String> {
    let directory = path
        .parent()
        .ok_or_else(|| format!("{}: has no directory to be in", path.display()))?;
    std::fs::create_dir_all(directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    let fresh = super::native::generate_secret()?;
    let scratch = directory.join("key.new");
    super::write_private(&scratch, super::native::write_secret(&fresh).as_bytes())?;
    std::fs::rename(&scratch, path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(fresh)
}
