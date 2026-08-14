//! Handing the crypto itself to another program.
//!
//! # What this is for
//!
//! A key in hardware never leaves the hardware. That is the whole point of it, and it is why
//! [`KeySource::Command`](super::KeySource::Command) cannot reach one: there is no identity to
//! print. age solves this with a plugin protocol — an external `age-plugin-yubikey` that does the
//! crypto on the device — and oslo does not speak that protocol.
//!
//! But `age` itself does. So a store can hand the whole operation to it:
//!
//! ```text
//! [yubi]
//! encrypt command age -R /home/you/recipients.txt
//! decrypt command age --decrypt --identity /home/you/yubikey-identity.txt
//! ```
//!
//! Plaintext goes in on standard input, ciphertext comes back on standard output, and the reverse
//! for reading. oslo's own `age` is not used for that store at all; the recipients, the plugin, the
//! touch policy and the file format are the other program's business. What oslo keeps is everything
//! around it: the store, the names, the file layout, `oslo secret run`, the lazy variable, the Lua
//! API, and the rule about where keys are allowed to live.
//!
//! It is not only for hardware. `gpg`, a KMS wrapper, a company's own tool — anything that filters
//! bytes both ways works, and none of it is code this shell has to carry.
//!
//! # What it costs, said plainly
//!
//! **The plaintext crosses a pipe to a program oslo did not compile.** An age plugin sees only a
//! wrapped file key; this sees the secret. That is a real difference and the reason this is not the
//! default — but the program is one you named in your own configuration file, which is the same
//! trust `key command` already asks for.
//!
//! # The fences, the same as everywhere here
//!
//! * argv, never a shell string, so nothing reaches `/bin/sh`.
//! * Never in a `plugin.*` store.
//! * `$OSLO_SECRET_NO_EXEC` refuses, and says which command it did not run.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

/// The two halves of a store's crypto, when another program does it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cipher {
    pub encrypt: Option<Vec<String>>,
    pub decrypt: Option<Vec<String>>,
}

impl Cipher {
    /// Whether anything here runs a program.
    pub fn is_external(&self) -> bool {
        self.encrypt.is_some() || self.decrypt.is_some()
    }

    /// How it is written in the file.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(argv) = &self.encrypt {
            lines.push(format!("encrypt command {}", argv.join(" ")));
        }
        if let Some(argv) = &self.decrypt {
            lines.push(format!("decrypt command {}", argv.join(" ")));
        }
        lines
    }
}

/// `command ARG…`, as written after `encrypt` or `decrypt`.
pub fn parse(kind: &str, rest: &str) -> Result<Vec<String>, String> {
    let (word, argv) = super::conf::split_word(rest);
    if word != "command" {
        return Err(format!("`{kind}` takes a command, not {word:?}"));
    }
    if argv.is_empty() {
        return Err(format!("`{kind} command` needs a program to run"));
    }
    Ok(argv.split_whitespace().map(str::to_string).collect())
}

/// Put `input` through `argv` and answer what came back.
///
/// **Standard input is written on a thread**, because a program that starts answering before it has
/// read everything would otherwise fill the pipe and wait for us while we wait for it. `age` on a
/// small value would not, but a wrapper that buffers differently is exactly the kind of program
/// somebody puts here.
pub fn through(argv: &[String], input: &[u8]) -> Result<Vec<u8>, String> {
    if super::key::no_exec() {
        return Err(format!(
            "$OSLO_SECRET_NO_EXEC is set, so `{}` was not run",
            argv.join(" ")
        ));
    }
    let (program, rest) = argv.split_first().ok_or("a command with no program")?;
    let mut child = Command::new(program)
        .args(rest)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;

    let mut stdin = child.stdin.take().ok_or("no standard input to write to")?;
    let written = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&written));

    let mut output = Vec::new();
    let mut said = String::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_end(&mut output)
            .map_err(|e| format!("{program}: {e}"))?;
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut said);
    }
    let status = child.wait().map_err(|e| format!("{program}: {e}"))?;
    // Joined after the wait: a program that exits without reading its input leaves the write
    // failing with a broken pipe, which is its business rather than an error of ours.
    let _ = writer.join();

    if !status.success() {
        return Err(format!(
            "{program}: exited {}{}",
            status.code().unwrap_or(-1),
            match said.trim().is_empty() {
                true => String::new(),
                false => format!(": {}", said.trim()),
            }
        ));
    }
    if output.is_empty() {
        return Err(format!("{program}: gave nothing back"));
    }
    Ok(output)
}
