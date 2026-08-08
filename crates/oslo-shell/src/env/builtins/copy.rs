//! `copy` — put text on the system clipboard, through the terminal.
//!
//! ```text
//! copy hello world        the arguments
//! ls | copy               standard input
//! copy < notes.txt        a file
//! ```
//!
//! Uses `OSC 52`, which means the terminal does the pasting rather than an X or Wayland helper.
//! Two consequences, and both are the reason to have it:
//!
//! * **It works over SSH.** `xsel` and `wl-copy` talk to a display server that is not there on the
//!   far end of a connection; this travels up the same channel the session does, so text copied on
//!   a remote machine lands on the local clipboard.
//! * **It needs no dependency**, so it works in a container or an initramfs where no clipboard
//!   tool is installed.
//!
//! The catch, stated plainly because it is invisible when it happens: a terminal may have `OSC 52`
//! disabled — several do by default, since a program that can write your clipboard can do so
//! without asking. There is no reply to read, so oslo cannot tell. If nothing arrives, that is
//! why, and the terminal's own setting is the place to fix it.

use crate::env::Environment;
use oslo_base::error::Result;
use oslo_ui::marks;
use std::io::{Read, Write};

pub fn builtin_copy(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let operands = &args[1..];
    if operands.first().is_some_and(|a| a == "--help" || a == "-h") {
        println!("Usage: copy [TEXT...]      with no arguments, copies standard input");
        return Ok(0);
    }

    let text = if operands.is_empty() {
        let mut buffer = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buffer) {
            eprintln!("copy: cannot read standard input: {e}");
            return Ok(1);
        }
        buffer
    } else {
        // Joined with spaces, as `echo` does: `copy hello world` copies what it looks like.
        operands.join(" ")
    };

    // A trailing newline is almost never wanted on a clipboard — pasting it into a shell would run
    // the line — and `ls | copy` produces one for every entry. Only the last is dropped.
    let text = text.strip_suffix('\n').unwrap_or(&text);

    let mut out = std::io::stdout();
    if out.write_all(marks::clipboard(text).as_bytes()).is_err() || out.flush().is_err() {
        eprintln!("copy: cannot write to the terminal");
        return Ok(1);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        std::iter::once("copy")
            .chain(words.iter().copied())
            .map(str::to_string)
            .collect()
    }

    /// Arguments are joined the way `echo` joins them, so what is copied is what it looked like.
    #[test]
    fn arguments_are_joined_with_spaces() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_copy(&mut env, &args(&["hello", "world"])).unwrap(),
            0
        );
        // The sequence itself is what carries the text; checked directly here because the builtin
        // writes to the real stdout.
        assert_eq!(
            marks::clipboard("hello world"),
            "\x1b]52;c;aGVsbG8gd29ybGQ=\x1b\\"
        );
    }

    /// `ls | copy` ends in a newline, and pasting that into a shell would run the line.
    #[test]
    fn a_single_trailing_newline_is_dropped() {
        assert_eq!("a\nb\n".strip_suffix('\n'), Some("a\nb"));
        // Only the last one: a blank final line the user actually typed is theirs to keep.
        assert_eq!("a\n\n".strip_suffix('\n'), Some("a\n"));
    }
}
