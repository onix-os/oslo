//! Semantic marks: telling the terminal where a command begins and ends.
//!
//! `OSC 133`, the FinalTerm/FTCS shell-integration protocol that kitty, WezTerm, Ghostty, iTerm2,
//! VS Code and tmux all read. The shell says where the prompt starts, where output starts, and
//! what the command exited with; what the terminal *does* with that is the terminal's business.
//!
//! # Why the shell only marks, and does not fold
//!
//! A shell cannot rewrite scrollback. Once bytes are written they belong to whatever owns the
//! grid, and folding a command that has scrolled off means redrawing rows the shell can no longer
//! reach. The thing that *can* do it is the terminal emulator or the multiplexer, which keeps the
//! grid and the history. So the division is: oslo declares the boundaries, and the layer that owns
//! the screen decides whether to draw a fold arrow next to them.
//!
//! # What is emitted
//!
//! | Mark | When | Meaning |
//! |---|---|---|
//! | `OSC 133 ; A ; aid=<n> ST` | before the prompt is drawn | prompt start; `aid` is the block id |
//! | `OSC 133 ; C ; aid=<n> ST` | just before the command runs | output starts here |
//! | `OSC 133 ; D ; <status> ; aid=<n> ST` | once it has finished | command end, with its exit status |
//!
//! `B` — "the prompt ends and typing starts" — is deliberately **not** emitted. It would have to
//! be written between the prompt and the cursor, which means inside the string handed to the line
//! editor, and the editor measures that string to work out where the line begins. An `OSC` in
//! there is counted as visible width and the cursor arithmetic is wrong from the first keystroke.
//! `A`..`C` already delimits the prompt, which is what a folding implementation needs.
//!
//! `aid` is oslo's addition to the standard three: it makes each block nameable, so a reader can
//! match a `D` to the `A` that opened it without relying on them being adjacent in the stream.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Turn marks on for an interactive session that has a terminal to mark.
///
/// Off for every script, `-c`, and test binary: a program reading oslo's output must never find
/// escape sequences the shell invented in it.
pub fn enable(interactive: bool) {
    let on = interactive
        && nix::unistd::isatty(1).unwrap_or(false)
        && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true);
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The id of the block being prompted for.
pub fn current_id() -> u64 {
    NEXT_ID.load(Ordering::Relaxed)
}

/// Take an id and move to the next. Called once per prompt.
fn advance() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// `OSC 7` — where the shell is now.
///
/// A `file://host/path` URL, which is what makes a terminal open a new tab or split in the
/// directory you were in rather than in `$HOME`. Read by kitty, foot, WezTerm, Ghostty and every
/// multiplexer that cares — it is the highest-value sequence a shell emits and costs one write per
/// `cd`.
///
/// The path is percent-encoded, because a URL is not a path: a directory with a space, a `#` or a
/// `%` in it produces a URL that means something else entirely, and directories like that are
/// exactly the ones nobody tests with.
pub fn working_directory(path: &str) -> String {
    if !enabled() {
        return String::new();
    }
    let host = nix::unistd::gethostname()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_default();
    format!("\x1b]7;file://{host}{}\x1b\\", percent_encode(path))
}

/// Percent-encode the parts of a path a URL cannot carry literally.
///
/// Unreserved characters (RFC 3986) plus `/`, which is the path separator and must stay literal.
fn percent_encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// `OSC 0` — the window and tab title.
///
/// Set to the command while one is running and to the directory when the shell is idle, which is
/// fish's behaviour: a row of tabs then says what each one is *doing*, not merely where it is.
///
/// A multiplexer that names its own panes will fight this, and the last writer wins. That is why
/// it is a setting rather than unconditional.
pub fn title(text: &str) -> String {
    if !enabled() {
        return String::new();
    }
    // Control characters would end the sequence early and leave the rest as text on screen.
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    format!("\x1b]0;{clean}\x1b\\")
}

/// `OSC 52` — put `text` on the system clipboard.
///
/// The one way a shell can reach the clipboard **through the terminal**, which means it works over
/// SSH where `xsel` and `wl-copy` cannot: the bytes travel up the same connection the session
/// does, and the terminal at the far end does the pasting.
///
/// Write-only by design. Reading back is supported almost nowhere, and a terminal that does
/// support it usually asks the user first — so a `paste` built on this would work on one machine
/// in ten and hang on the rest.
///
/// Not gated on [`enabled`]: unlike a title or a prompt mark, this is something the user asked for
/// explicitly, and refusing it because stdout is a pipe would be surprising.
pub fn clipboard(text: &str) -> String {
    format!("\x1b]52;c;{}\x1b\\", base64(text.as_bytes()))
}

/// Base64, RFC 4648, which is what `OSC 52` carries.
///
/// Written out rather than pulled in: it is twenty lines, and a dependency for twenty lines is a
/// dependency to audit, to keep current, and to explain in a build that has none.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            // A group short of three bytes pads rather than encoding what it did not have.
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - i * 6)) as usize & 0x3f] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `OSC 8` — `text`, clickable, pointing at `url`.
///
/// Only ever wrapped around text oslo itself prints: a path in a diagnostic, a file in a listing.
/// It cannot make `ls` or `cargo` output clickable, because oslo never sees those bytes.
///
/// A terminal that does not know `OSC 8` shows `text` and drops the rest, so this is safe to emit
/// unconditionally — which is why it is gated on [`enabled`] only to keep it out of scripts.
pub fn hyperlink(url: &str, text: &str) -> String {
    if !enabled() {
        return text.to_string();
    }
    format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
}

/// A path, printed clickable when the terminal is listening and plain when it is not.
///
/// The shape a diagnostic wants: `eprintln!("oslo: {}: {e}", marks::path(p))` reads the same as
/// before and gains a link for free. A config file named in an error is exactly the thing you want
/// to open next.
pub fn path(path: &str) -> String {
    if !enabled() {
        return path.to_string();
    }
    hyperlink(&file_url(path), path)
}

/// A `file://` URL for a path on this machine, for [`hyperlink`].
pub fn file_url(path: &str) -> String {
    let host = nix::unistd::gethostname()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_default();
    format!("file://{host}{}", percent_encode(path))
}

/// `OSC 133 ; A` — a new prompt, and a new block, begins here.
pub fn prompt_start() -> String {
    if !enabled() {
        return String::new();
    }
    format!("\x1b]133;A;aid={}\x1b\\", advance())
}

/// `OSC 133 ; C` — everything after this is the command's output.
pub fn output_start() -> String {
    if !enabled() {
        return String::new();
    }
    // `current_id` and not a fresh one: this closes the prompt `A` opened, so it carries the same
    // id. `A` has already advanced the counter, so the id in force is the one before it.
    format!("\x1b]133;C;aid={}\x1b\\", current_id().saturating_sub(1))
}

/// `OSC 133 ; D` — the command has finished, with this status.
pub fn command_end(status: i32) -> String {
    if !enabled() {
        return String::new();
    }
    format!(
        "\x1b]133;D;{status};aid={}\x1b\\",
        current_id().saturating_sub(1)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Marks are off unless a person is looking at a terminal. A script's output must never carry
    /// escape sequences the shell invented.
    #[test]
    fn nothing_is_emitted_without_a_terminal() {
        enable(false);
        assert_eq!(prompt_start(), "");
        assert_eq!(output_start(), "");
        assert_eq!(command_end(0), "");
    }

    /// A block's three marks carry the same id, so a reader can pair them up without assuming
    /// they arrive next to each other.
    #[test]
    fn one_block_carries_one_id_through_all_three_marks() {
        ENABLED.store(true, Ordering::Relaxed);
        NEXT_ID.store(7, Ordering::Relaxed);

        let a = prompt_start();
        let c = output_start();
        let d = command_end(3);
        assert_eq!(a, "\x1b]133;A;aid=7\x1b\\");
        assert_eq!(c, "\x1b]133;C;aid=7\x1b\\");
        assert_eq!(d, "\x1b]133;D;3;aid=7\x1b\\");

        // The next prompt is the next block.
        assert_eq!(prompt_start(), "\x1b]133;A;aid=8\x1b\\");
        assert_eq!(output_start(), "\x1b]133;C;aid=8\x1b\\");

        ENABLED.store(false, Ordering::Relaxed);
    }

    /// A URL is not a path. A directory with a space or a `#` in it makes a URL that means
    /// something else, and those are exactly the directories nobody tests with.
    #[test]
    fn a_working_directory_is_percent_encoded() {
        ENABLED.store(true, Ordering::Relaxed);
        let osc = working_directory("/home/u/my dir/a#b");
        assert!(osc.contains("/home/u/my%20dir/a%23b"), "{osc:?}");
        // Slashes stay literal, or the path stops being a path.
        assert!(!osc.contains("%2F"), "{osc:?}");
        assert!(osc.starts_with("\x1b]7;file://"), "{osc:?}");
        assert!(osc.ends_with("\x1b\\"), "{osc:?}");
        ENABLED.store(false, Ordering::Relaxed);
    }

    /// Checked against the RFC 4648 vectors, because a base64 that is wrong by one pad character
    /// puts silently corrupted text on the clipboard — which is worse than putting none there.
    #[test]
    fn base64_matches_the_rfc_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // Bytes that are not text at all still encode.
        assert_eq!(base64(&[0xff, 0x00, 0xff]), "/wD/");
    }

    #[test]
    fn the_clipboard_sequence_carries_the_encoded_text() {
        assert_eq!(clipboard("foo"), "\x1b]52;c;Zm9v\x1b\\");
        // Not gated on a terminal: copying is something the user asked for by name.
        assert!(!clipboard("x").is_empty());
    }

    /// A terminal that does not know `OSC 8` shows the text and drops the rest, so the text has to
    /// be there in full either way.
    #[test]
    fn a_hyperlink_wraps_its_text_without_altering_it() {
        ENABLED.store(true, Ordering::Relaxed);
        let link = hyperlink("file://h/etc/foo", "/etc/foo");
        assert!(
            link.starts_with("\x1b]8;;file://h/etc/foo\x1b\\"),
            "{link:?}"
        );
        assert!(link.contains("/etc/foo"));
        assert!(link.ends_with("\x1b]8;;\x1b\\"), "{link:?}");
        ENABLED.store(false, Ordering::Relaxed);

        // With marks off — a script — it is the bare text and nothing else.
        assert_eq!(hyperlink("file://h/x", "/x"), "/x");
    }

    /// A control character in a title would end the sequence early and spill the rest onto the
    /// screen as text.
    #[test]
    fn a_title_carries_no_control_characters() {
        ENABLED.store(true, Ordering::Relaxed);
        let osc = title("build\x07 done\nnow");
        assert_eq!(osc, "\x1b]0;build donenow\x1b\\");
        ENABLED.store(false, Ordering::Relaxed);
    }

    /// Every mark is a complete OSC: introducer, payload, terminator. A half-written one would be
    /// swallowed along with whatever text followed it.
    #[test]
    fn every_mark_is_a_terminated_osc() {
        ENABLED.store(true, Ordering::Relaxed);
        for mark in [prompt_start(), output_start(), command_end(0)] {
            assert!(mark.starts_with("\x1b]133;"), "{mark:?}");
            assert!(mark.ends_with("\x1b\\"), "{mark:?}");
            assert!(
                !mark.contains('\n'),
                "a mark must not move the cursor: {mark:?}"
            );
        }
        ENABLED.store(false, Ordering::Relaxed);
    }
}
