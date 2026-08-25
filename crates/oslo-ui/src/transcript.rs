//! What a finished line leaves on the screen, and how a terminal is told where it is.
//!
//! The drawing itself is [`crate::edit::screen::transcript`]. This holds the two things around it:
//! who renders the block, and the escape sequence that says "the block starts here".
//!
//! # Why a renderer slot rather than a call
//!
//! The block may be drawn by another program — `oslo.transcript.command` — and running programs is
//! `oslo-runtime`'s business, not the line editor's. So the editor asks a function that startup
//! installed, exactly as [`oslo_base::background`] is given its servicer. With nothing installed,
//! the rule from the settings is used, which is the whole of the simple case.
//!
//! # The marks, and why they are oslo's own number
//!
//! A transcript already sits inside `OSC 133`'s marked region — between `B`, the start of input,
//! and `C`, the start of output — so a terminal can already fold a whole command with `A`…`D`.
//! What it cannot do is tell the *frame* apart from the prompt, which is what folding everything
//! **except** the rule-and-brackets header needs.
//!
//! `OSC 133` is not the place to say that: its vocabulary is shared with every other shell, and a
//! key oslo invented there would be a key those shells' terminals have to guess at. hexe made the
//! same call for its palette protocol and for the same reason — its own number, adjacent to 133,
//! with 133 explicitly reserved. This is the sibling of that decision.
//!
//! ```text
//! ESC ] 1440 ; frame ; begin ; aid=<session> ST
//! ------------------------------[ cargo test --lib ]---
//! ESC ] 1440 ; frame ; end ; aid=<session> ST
//! ```
//!
//! **Verb first, like hexe's.** `frame` is the only one today; a later `fold` or `title` is another
//! verb rather than another number, and a terminal that does not know a verb ignores the sequence
//! whole, which is what every terminal already does with an OSC it has never heard of.

use std::sync::OnceLock;

/// Draws one row of the block, given that row. Installed once, by startup.
type Renderer = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

static RENDERER: OnceLock<Renderer> = OnceLock::new();

/// Install the renderer. The first call wins; a later one is ignored rather than panicking, which
/// is [`oslo_base::background::install`]'s rule and for the same reason.
pub fn install(renderer: impl Fn(&str) -> Option<String> + Send + Sync + 'static) {
    let _ = RENDERER.set(Box::new(renderer));
}

/// What another program says **one row** of the block should read, if one is installed and answered.
///
/// **One row, asked for one at a time.** A renderer is line-oriented — pixy, the case this was
/// built for, refuses a control byte in a rendered string outright — so neither "print the whole
/// block" nor "here is a pasted command, newlines and all" is a contract such a tool can meet. The
/// caller splits first and asks per row; the rule, the brackets and the alignment are never the
/// renderer's. Trailing line endings are cut for the same reason: a program that prints a line ends
/// it, and the caller is about to end it again.
///
/// `None` sends the caller back to the prefix and the row as it was typed, which is also what a
/// renderer that failed or overran means: a transcript is not worth losing a command's frame over.
pub fn rendered(row: &str) -> Option<String> {
    let text = RENDERER.get()?(row)?;
    let text = text.trim_end_matches(['\r', '\n']);
    (!text.is_empty()).then(|| text.to_string())
}

/// The OSC number the marks are written with.
///
/// `$OSLO_TRANSCRIPT_OSC` first, so a terminal that has claimed 1440 for something else can be
/// worked around without editing a config — the same escape hatch hexe gives its palette number.
/// Then the setting, then 1440.
pub fn osc() -> u32 {
    if let Ok(text) = std::env::var("OSLO_TRANSCRIPT_OSC")
        && let Ok(number) = text.trim().parse::<u32>()
        && usable(number)
    {
        return number;
    }
    let configured = crate::settings::current().transcript.osc;
    match usable(configured) {
        true => configured,
        false => DEFAULT_OSC,
    }
}

/// Adjacent to `OSC 133`, whose region a transcript sits inside, and clear of hexe's `1330`.
pub const DEFAULT_OSC: u32 = 1440;

/// Whether a number may be used for this.
///
/// **The reserved list is short and it is about breakage, not taste.** Claiming a number a terminal
/// already acts on does not add a mark — it takes away whatever that number did, and the failures
/// are silent and far from the config line that caused them: `0` stops titles, `7` stops the
/// working directory a mux reads, `133` stops the very marks this sits beside.
fn usable(code: u32) -> bool {
    !matches!(
        code,
        0..=2 | 4 | 5 | 7 | 9 | 99 | 104 | 105 | 133 | 777 | 1337
    )
}

/// The sequence that opens or closes a transcript block. Empty when marks are off.
pub fn mark(begin: bool) -> String {
    if !crate::marks::enabled() {
        return String::new();
    }
    let edge = match begin {
        true => "begin",
        false => "end",
    };
    format!(
        "\x1b]{};frame;{edge};aid={}\x1b\\",
        osc(),
        crate::marks::session_aid()
    )
}

#[cfg(test)]
#[path = "transcript/tests.rs"]
mod tests;

/// How the command before this one ended, for the frame that follows its output.
///
/// **The frame cannot report its own command.** It is drawn between Enter and the command starting,
/// so at that moment there is no status to show — the shell learns it once the command has ended
/// and the output has already scrolled past the frame.
///
/// What it can report is the command *above* it, which is why the mark goes at the left-hand end of
/// the rule: that end sits directly under the last line of the previous command's output, and reads
/// as closing it off. Put beside the command instead it would say something false.
///
/// `None` until a command has run, so the first frame of a session carries nothing.
static LAST: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(NOTHING_YET);

/// No command has ended in this session yet. Outside the range a real status can take.
const NOTHING_YET: i64 = -1;

/// Record how a command ended. Called once per command, by the loop that ran it.
pub fn ended(status: i32) {
    LAST.store(status as i64, std::sync::atomic::Ordering::Relaxed);
}

/// The status to open a frame with, or `None` before anything has run.
pub fn last() -> Option<i32> {
    match LAST.load(std::sync::atomic::Ordering::Relaxed) {
        NOTHING_YET => None,
        status => Some(status as i32),
    }
}
