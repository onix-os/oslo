//! Asking the terminal a question, and surviving the answer never coming.
//!
//! Every other sequence oslo emits is one-way: write it and forget it. This one is a *query* —
//! `OSC 11 ; ?` asks for the background colour and the terminal replies on **standard input**,
//! the same channel your keystrokes arrive on. That makes it the riskiest thing in the terminal
//! layer, and the failure modes are what this module is shaped around:
//!
//! * **Nothing answers.** Plenty of terminals do not implement it, and a multiplexer may swallow
//!   it. Blocking on a read would hang the shell before its first prompt.
//! * **Something else answers first.** If the user typed while the query was in flight, their
//!   keystroke is sitting in the buffer ahead of the reply.
//! * **The answer arrives late.** Read it too eagerly and half of it is left behind, to appear as
//!   junk at the prompt a moment later.
//!
//! So: a short deadline, and **only bytes that match the reply's shape are consumed**. Anything
//! that is not the beginning of an `OSC 11` reply is left in the buffer for the line editor, which
//! is what keeps a terminal that ignores the query from costing the user a keystroke.

use std::io::Read;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::time::{Duration, Instant};

/// How long to wait for a reply before giving up.
///
/// **This is dead time on a terminal that will never answer**, and it is paid before the first
/// prompt is drawn — so it is the whole of the shell's perceived startup cost on any terminal
/// without `OSC 11`. It was 120ms, which is long enough to feel like the shell is thinking.
///
/// A terminal that does answer does so as fast as it can write to a pty: under a millisecond
/// locally, a few over a slow link. 20ms is many times either, and the read already stops early
/// at the first byte that cannot belong to a reply.
const DEADLINE: Duration = Duration::from_millis(20);

/// What the terminal said its background is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    Dark,
    Light,
}

/// Ask the terminal for its background colour.
///
/// `None` when it did not answer, which is not an error and is the common case on a terminal that
/// does not implement `OSC 11`. The caller keeps its default.
pub fn background() -> Option<Background> {
    // `$COLORFGBG` is the same answer without asking, and costs nothing. Terminals that set it
    // (rxvt, konsole, and anything that inherited the convention) let the whole exchange be
    // skipped — no escape written, no reply waited for.
    if let Some(background) = from_colorfgbg(std::env::var("COLORFGBG").ok().as_deref()) {
        return Some(background);
    }
    if !nix::unistd::isatty(0).unwrap_or(false) || !nix::unistd::isatty(1).unwrap_or(false) {
        return None;
    }
    let reply = ask("\x1b]11;?\x1b\\")?;
    parse_background(&reply)
}

/// Read `$COLORFGBG`, whose last field is the background's ANSI colour number.
///
/// `0`-`6` and `8` are the dark half of the sixteen; `7` and `9`-`15` are the light half. That is
/// the same split every editor that reads this variable uses, and it is what vim's `background`
/// detection has done for decades.
fn from_colorfgbg(value: Option<&str>) -> Option<Background> {
    let number: u8 = value?.rsplit(';').next()?.trim().parse().ok()?;
    Some(match number {
        0..=6 | 8 => Background::Dark,
        _ => Background::Light,
    })
}

/// Write `question` and read what comes back, in raw mode, up to [`DEADLINE`].
fn ask(question: &str) -> Option<String> {
    use nix::sys::termios::{SetArg, tcgetattr, tcsetattr};
    let stdin = std::io::stdin();
    // Raw mode, or the reply sits in the line discipline's buffer until Enter is pressed — which
    // is to say, for ever.
    let original = tcgetattr(&stdin).ok()?;
    let mut raw = original.clone();
    nix::sys::termios::cfmakeraw(&mut raw);
    let _ = tcsetattr(&stdin, SetArg::TCSANOW, &raw);

    let answer = ask_raw(question);

    let _ = tcsetattr(&stdin, SetArg::TCSANOW, &original);
    answer
}

fn ask_raw(question: &str) -> Option<String> {
    use std::io::Write;
    let mut out = std::io::stdout();
    out.write_all(question.as_bytes()).ok()?;
    out.flush().ok()?;

    let fd = std::io::stdin().as_raw_fd();
    let mut collected = String::new();
    let started = Instant::now();

    while started.elapsed() < DEADLINE {
        let remaining = DEADLINE.saturating_sub(started.elapsed());
        // Safety: fd 0 is the process's own standard input and outlives the borrow.
        let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
        let mut fds = [nix::poll::PollFd::new(
            borrowed,
            nix::poll::PollFlags::POLLIN,
        )];
        let timeout = u16::try_from(remaining.as_millis()).unwrap_or(u16::MAX);
        match nix::poll::poll(&mut fds, nix::poll::PollTimeout::from(timeout)) {
            // Nothing came. The terminal is not going to answer.
            Ok(0) => return None,
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return None,
        }

        let mut byte = [0u8; 1];
        match std::io::stdin().read(&mut byte) {
            Ok(1) => {}
            _ => return None,
        }
        let c = byte[0] as char;

        // **One byte at a time, and only while it still looks like a reply.** The first byte that
        // cannot belong to one means somebody typed: stop, and leave the rest alone. Consuming
        // more would eat their keystroke.
        if collected.is_empty() && c != '\x1b' {
            return None;
        }
        collected.push(c);
        if collected.ends_with('\x07') || collected.ends_with("\x1b\\") {
            return Some(collected);
        }
        // A reply is short. Anything longer is not one, and reading on would consume input.
        if collected.len() > 64 {
            return None;
        }
    }
    None
}

/// Read `OSC 11 ; rgb:RRRR/GGGG/BBBB` and decide whether that is a dark background.
///
/// The components may be one to four hex digits each — terminals differ — so each is scaled to a
/// fraction of its own width rather than assumed to be 16-bit.
pub fn parse_background(reply: &str) -> Option<Background> {
    let body = reply.split("rgb:").nth(1)?;
    let mut parts = body.split('/');
    let mut channel = || -> Option<f64> {
        let raw: String = parts
            .next()?
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        if raw.is_empty() {
            return None;
        }
        let value = u32::from_str_radix(&raw, 16).ok()?;
        let max = 16u32.pow(raw.len() as u32) - 1;
        Some(f64::from(value) / f64::from(max))
    };
    let (r, g, b) = (channel()?, channel()?, channel()?);

    // Rec. 601 luma. Green dominates because the eye does: a mid-green background reads as light
    // where the same value in blue does not.
    let luma = 0.299 * r + 0.587 * g + 0.114 * b;
    Some(if luma < 0.5 {
        Background::Dark
    } else {
        Background::Light
    })
}

#[cfg(test)]
mod tests {
    use super::{Background, from_colorfgbg};

    /// The last field is the background, and the dark half of the sixteen colours is 0-6 and 8.
    #[test]
    fn colorfgbg_answers_without_asking_the_terminal() {
        assert_eq!(from_colorfgbg(Some("15;0")), Some(Background::Dark));
        assert_eq!(from_colorfgbg(Some("0;15")), Some(Background::Light));
        assert_eq!(from_colorfgbg(Some("15;8")), Some(Background::Dark));
        assert_eq!(from_colorfgbg(Some("0;7")), Some(Background::Light));
        // Three fields: some terminals put the cursor colour in the middle.
        assert_eq!(from_colorfgbg(Some("15;default;0")), Some(Background::Dark));
        // Nothing usable means fall through to asking, not a guess.
        assert_eq!(from_colorfgbg(None), None);
        assert_eq!(from_colorfgbg(Some("")), None);
        assert_eq!(from_colorfgbg(Some("15;default")), None);
    }

    use super::*;

    /// Terminals differ on how many hex digits they send per channel, so each is scaled to its own
    /// width. Reading `ff` as if it were `ffff` would make every light background look dark.
    #[test]
    fn a_reply_is_read_at_whatever_width_it_arrives_in() {
        // 16-bit, near black.
        assert_eq!(
            parse_background("\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\"),
            Some(Background::Dark)
        );
        // 16-bit, near white.
        assert_eq!(
            parse_background("\x1b]11;rgb:f5f5/f5f5/f5f5\x1b\\"),
            Some(Background::Light)
        );
        // 8-bit, the same two colours.
        assert_eq!(
            parse_background("\x1b]11;rgb:1e/1e/2e\x07"),
            Some(Background::Dark)
        );
        assert_eq!(
            parse_background("\x1b]11;rgb:f5/f5/f5\x07"),
            Some(Background::Light)
        );
    }

    /// Green weighs most, as the eye weighs it: a mid-green is light where the same value in blue
    /// is not.
    #[test]
    fn brightness_is_weighted_the_way_the_eye_weighs_it() {
        // Full green is light (luma 0.587); full blue is not (0.114). Same channel value, both
        // ends of the decision — which is the whole point of weighting.
        assert_eq!(
            parse_background("rgb:0000/ffff/0000"),
            Some(Background::Light)
        );
        assert_eq!(
            parse_background("rgb:0000/0000/ffff"),
            Some(Background::Dark)
        );
        // And a green just under the line stays dark rather than rounding hopefully.
        assert_eq!(
            parse_background("rgb:0000/cccc/0000"),
            Some(Background::Dark)
        );
    }

    /// A light answer must change what the *default* palette is, not install a theme — the config
    /// is read afterwards and merged over the default, and installing here would be overwritten
    /// wholesale. That is what went wrong on the first attempt, and it looked like the query
    /// failing rather than like an ordering bug.
    #[test]
    fn a_light_background_changes_the_default_palette() {
        use crate::interactive::theme::{Syntax, set_background};
        let dark = Syntax::default();
        set_background(Background::Light);
        let light = Syntax::default();
        assert_ne!(dark.error, light.error, "the palette must actually change");
        assert_eq!(light.error, Syntax::for_light_background().error);
        // Put it back, or every later test in this binary sees a light terminal.
        set_background(Background::Dark);
        assert_eq!(Syntax::default().error, dark.error);
    }

    /// Anything that is not a reply answers `None` rather than guessing — the caller then keeps
    /// its default, which is what a terminal that ignored the query should produce.
    #[test]
    fn nonsense_is_not_a_colour() {
        assert_eq!(parse_background(""), None);
        assert_eq!(parse_background("\x1b]11;?\x1b\\"), None);
        assert_eq!(parse_background("rgb:"), None);
        assert_eq!(parse_background("rgb:zz/zz/zz"), None);
        // Two channels is not three.
        assert_eq!(parse_background("rgb:1111/2222"), None);
    }
}
