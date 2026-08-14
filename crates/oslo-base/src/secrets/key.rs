//! Where a store's keys come from.
//!
//! # Two kinds, in a deliberate order
//!
//! A [`KeySource::File`] is read; a [`KeySource::Command`] is *run*, and its standard output is the
//! identity. The second exists because the alternative is compiling every way a person might hold a
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

use age::secrecy::ExposeSecret;
use age::x25519;
use std::path::PathBuf;

/// Where one key comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    File(PathBuf),
    Command(Vec<String>),
}

impl KeySource {
    /// `file PATH`, or `command ARG…`, as written after `key` in `secrets.conf`.
    pub fn parse(rest: &str) -> Result<Self, String> {
        let (kind, rest) = super::conf::split_word(rest);
        match kind {
            "file" if !rest.is_empty() => Ok(KeySource::File(PathBuf::from(rest))),
            "command" if !rest.is_empty() => {
                let argv: Vec<String> = rest.split_whitespace().map(str::to_string).collect();
                Ok(KeySource::Command(argv))
            }
            "file" | "command" => Err(format!("`key {kind}` needs something after it")),
            other => Err(format!("a key is `file` or `command`, not {other:?}")),
        }
    }

    /// How it is written in the file.
    pub fn line(&self) -> String {
        match self {
            KeySource::File(path) => format!("key file {}", path.display()),
            KeySource::Command(argv) => format!("key command {}", argv.join(" ")),
        }
    }

    /// Whether reaching this key means running another program.
    pub fn is_external(&self) -> bool {
        matches!(self, KeySource::Command(_))
    }

    /// The identity, or why not.
    ///
    /// A file that is not there is `Ok(None)` rather than an error: several key sources are a list
    /// of places to look, and a laptop that has one of them is not misconfigured.
    pub fn identity(&self) -> Result<Option<x25519::Identity>, String> {
        match self {
            KeySource::File(path) => match std::fs::read_to_string(path) {
                Ok(text) => parse_identity(&text)
                    .map(Some)
                    .map_err(|e| format!("{}: {e}", path.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(format!("{}: {e}", path.display())),
            },
            KeySource::Command(argv) => KeySource::run(argv).map(Some),
        }
    }

    fn run(argv: &[String]) -> Result<x25519::Identity, String> {
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
            .map_err(|_| format!("{program}: what it printed is not an age identity"))?;
        parse_identity(&text).map_err(|e| format!("{program}: {e}"))
    }
}

/// Whether every external source is to be skipped.
pub fn no_exec() -> bool {
    std::env::var_os("OSLO_SECRET_NO_EXEC").is_some_and(|value| !value.is_empty())
}

/// The first `AGE-SECRET-KEY-1…` in what a file or a program gave us.
///
/// **Comment lines are skipped**, because `age-keygen` writes three of them above the key and
/// telling somebody to strip them by hand would be a step nobody remembers.
fn parse_identity(text: &str) -> Result<x25519::Identity, String> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .ok_or_else(|| "no age identity in it".to_string())?;
    // **The failure worth naming.** A key in hardware has no identity to print, so what a plugin
    // hands out is a stub saying which plugin to run — and "invalid Bech32" would send somebody
    // looking for a typo instead of at the answer, which is to let `age` itself do the crypto.
    if line.starts_with("AGE-PLUGIN-") {
        return Err(format!(
            "{}: an age plugin identity. oslo does not speak the age plugin protocol; \
             hand this store's crypto to `age` itself with `oslo secret cipher`",
            line.split('-').take(3).collect::<Vec<_>>().join("-")
        ));
    }
    line.parse::<x25519::Identity>().map_err(|e| e.to_string())
}

/// Make one where `path` says, mode `0600` from the moment it exists.
pub fn generate(path: &std::path::Path) -> Result<x25519::Identity, String> {
    let directory = path
        .parent()
        .ok_or_else(|| format!("{}: has no directory to be in", path.display()))?;
    std::fs::create_dir_all(directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    let fresh = x25519::Identity::generate();
    let scratch = directory.join("identity.new");
    let mut text = fresh.to_string().expose_secret().to_string();
    text.push('\n');
    super::write_private(&scratch, text.as_bytes())?;
    std::fs::rename(&scratch, path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(fresh)
}
