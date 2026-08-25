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
//! **except** the `- - -`/command/`- - -` header needs.
//!
//! `OSC 133` is not the place to say that: its vocabulary is shared with every other shell, and a
//! key oslo invented there would be a key those shells' terminals have to guess at. hexe made the
//! same call for its palette protocol and for the same reason — its own number, adjacent to 133,
//! with 133 explicitly reserved. This is the sibling of that decision.
//!
//! ```text
//! ESC ] 1440 ; frame ; begin ; aid=<session> ST
//! - - - - - - - - - - - -
//! cargo test --lib
//! - - - - - - - - - - - -
//! ESC ] 1440 ; frame ; end ; aid=<session> ST
//! ```
//!
//! **Verb first, like hexe's.** `frame` is the only one today; a later `fold` or `title` is another
//! verb rather than another number, and a terminal that does not know a verb ignores the sequence
//! whole, which is what every terminal already does with an OSC it has never heard of.

use std::sync::OnceLock;

/// Draws the block, given the command that was run. Installed once, by startup.
type Renderer = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

static RENDERER: OnceLock<Renderer> = OnceLock::new();

/// Install the renderer. The first call wins; a later one is ignored rather than panicking, which
/// is [`oslo_base::background::install`]'s rule and for the same reason.
pub fn install(renderer: impl Fn(&str) -> Option<String> + Send + Sync + 'static) {
    let _ = RENDERER.set(Box::new(renderer));
}

/// The header another program says the block should start with, if one is installed and it answered.
///
/// **One line, and the caller adds the rule.** A renderer is line-oriented — pixy, the case this
/// was built for, refuses a control byte in a rendered string outright — so a contract of "print
/// the whole block" is one such a tool cannot meet. Trailing line endings are cut for the same
/// reason: a program that prints a line ends it, and the caller is about to end it again.
///
/// `None` sends the caller back to the prefix and the command, which is also what a renderer that
/// failed or overran means: a transcript is not worth losing a command's frame over.
pub fn rendered(command: &str) -> Option<String> {
    let text = RENDERER.get()?(command)?;
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
