//! `keep` — run a command and remember what it printed, for `copy --last`.
//!
//! ```text
//! keep git log --oneline -20      run it, and keep the output
//! keep -e make build              keep what it wrote to stderr as well
//! copy --last                     put that on the clipboard
//! ```
//!
//! # It is a prefix because the shell cannot go back
//!
//! Output goes from the command to the terminal and is gone; nothing in the shell has a copy,
//! because keeping one means standing between the two. `keep` is that decision, taken for one
//! command at a time — the alternative is a shell that stands in the middle of *every* command,
//! which costs the memory of the largest thing you ever run and turns `isatty` false for all of
//! them (colours off, pagers changed, progress bars silent).
//!
//! **You still see it as it happens.** Every chunk read from the command is written straight on to
//! the terminal before it is added to what is kept, so `keep make build` scrolls exactly as
//! `make build` does. What it costs is a pipe: the command's stdout is not a terminal, so a program
//! that colours its output only for a terminal will not colour it here. That is also why what gets
//! kept is usually clean text — and what is left of an escape sequence is taken out before it is
//! stored, since a clipboard full of `\x1b[32m` is nobody's idea of the output.
//!
//! # `keep` is not `tee`
//!
//! `tee` writes to a file you name and is part of the pipeline; this writes to one place, the
//! session's own, which `copy --last` knows how to find without being told. Piping into `copy`
//! directly — `ls | copy` — is still the shorter way when copying is all you wanted.

use crate::env::Environment;
use oslo_base::error::Result;
use std::io::{Read, Write};

/// `keep [-e] command [argument…]`
pub fn builtin_keep(env: &mut Environment, args: &[String]) -> Result<i32> {
    let mut words = args.get(1..).unwrap_or_default();
    let mut merge_stderr = false;
    while let Some(first) = words.first() {
        match first.as_str() {
            "-e" | "--stderr" => merge_stderr = true,
            "-h" | "--help" => {
                println!(
                    "Usage: keep [-e] COMMAND [ARG...]   run it, keep its output for `copy --last`\n\
                     \n  -e, --stderr   keep what it writes to stderr as well"
                );
                return Ok(0);
            }
            "--" => {
                words = &words[1..];
                break;
            }
            _ => break,
        }
        words = &words[1..];
    }
    if words.is_empty() {
        eprintln!("keep: usage: keep [-e] COMMAND [ARG...]");
        return Ok(2);
    }

    let (child, reader) = crate::exec::argv::spawn_reading_streams(env, words, merge_stderr)?;
    let (text, status) = mirror(reader, child);

    match oslo_base::capture::store(&oslo_base::track::session::id(), &text) {
        Ok(true) => eprintln!(
            "keep: over {} MiB of output, kept the last {} MiB",
            oslo_base::capture::MAX / (1024 * 1024),
            oslo_base::capture::MAX / (1024 * 1024)
        ),
        Ok(false) => {}
        Err(e) => eprintln!("keep: {e}"),
    }
    Ok(status)
}

/// Pass the command's output through to the terminal as it arrives, and keep a copy.
///
/// Written out chunk by chunk rather than at the end: a command that takes a minute must not go
/// silent for a minute, and one that never ends must still be watchable.
fn mirror(reader: std::os::fd::OwnedFd, child: nix::unistd::Pid) -> (String, i32) {
    let mut file = std::fs::File::from(reader);
    let mut buffer = [0u8; 16 * 1024];
    let mut kept: Vec<u8> = Vec::new();
    let mut out = std::io::stdout();
    loop {
        match file.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let _ = out.write_all(&buffer[..read]);
                let _ = out.flush();
                // Bounded here as well as on the way to disk: a command that prints for an hour
                // must not grow this shell's memory by everything it printed.
                if kept.len() < oslo_base::capture::MAX * 2 {
                    kept.extend_from_slice(&buffer[..read]);
                }
            }
        }
    }
    let status = crate::exec::argv::reap(child);
    let text = String::from_utf8_lossy(&kept).into_owned();
    (oslo_ui::dropdown::width::without_escapes(&text), status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        std::iter::once("keep")
            .chain(words.iter().copied())
            .map(str::to_string)
            .collect()
    }

    /// A prefix with nothing after it is a usage error, not a command called "".
    #[test]
    fn nothing_to_run_is_refused() {
        let mut env = Environment::new();
        assert_eq!(builtin_keep(&mut env, &args(&[])).unwrap(), 2);
        assert_eq!(builtin_keep(&mut env, &args(&["-e"])).unwrap(), 2);
    }

    /// **The flags are `keep`'s, and the command's flags are the command's.** `keep ls -e` runs
    /// `ls -e`; only a leading `-e` is read here, and `--` ends the question.
    #[test]
    fn only_the_leading_options_are_read() {
        let mut env = Environment::new();
        // `--help` prints and stops, which is how the operand split can be seen without running
        // anything: a `--help` that came after the command name would belong to the command.
        assert_eq!(builtin_keep(&mut env, &args(&["--help"])).unwrap(), 0);
        assert_eq!(builtin_keep(&mut env, &args(&["--"])).unwrap(), 2);
    }

    /// What is kept is the text, not the escape sequences that coloured it on the way past.
    #[test]
    fn colours_do_not_reach_the_clipboard() {
        assert_eq!(
            oslo_ui::dropdown::width::without_escapes("\x1b[32mgreen\x1b[0m text"),
            "green text"
        );
    }
}
