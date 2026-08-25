//! `oslo.misc` — and `oslo.transcript`, which is what a finished line leaves behind.
//!
//! Split from [`super`] when that file reached the 600-line limit. Both groups here are about the
//! shell rather than about one of its subsystems, which is what `misc` already said of itself.

use super::InterruptEscape;

/// `oslo.misc` — the handful of settings that are not about any one subsystem.
///
/// A deliberate catch-all rather than a group per switch. A shell accumulates these, and inventing
/// `oslo.startup`, `oslo.banner` and `oslo.greeting` for one boolean each is how a config grows a
/// vocabulary nobody can remember.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Misc {
    /// Whether to print the version banner and the exit hint at startup.
    ///
    /// On by default, because a first-time user needs to be told how to leave — that is the
    /// oldest usability bug in the interactive-program genre. Off is for everybody else, who has
    /// read it a thousand times and would rather have the two rows.
    pub welcome: bool,
    /// Seconds a prompt may sit untouched before `on-idle-timeout` fires. `0` never fires.
    ///
    /// Off by default, and it costs nothing when off *or* when nothing is attached to the hook:
    /// the editor only asks for a timed read when both are true, so an ordinary session still
    /// blocks in one `read` per keystroke rather than waking up to ask whether anyone cared.
    pub idle_timeout: u64,
    /// Printed instead of the banner. fish's `fish_greeting`, which is the setting people
    /// actually reach for — `welcome = false` and then a line of your own is two settings in
    /// fish too, and merging them would mean an empty string had to mean "silent".
    pub greeting: Option<String>,
    /// Milliseconds to wait for the rest of an escape sequence before deciding a lone `ESC` was
    /// the Esc key. fish's `fish_escape_delay_ms`.
    ///
    /// 25 is right on a local terminal and wrong over a slow link, where the bytes of one arrow
    /// key can arrive far enough apart to be read as Esc followed by letters — which in vi mode
    /// means your cursor keys start executing commands. Raising this is the fix, and until now
    /// there was no way to.
    pub escape_delay: u64,
    /// Force a colour depth instead of detecting one: `truecolor`, `256`, `16` or `none`.
    ///
    /// Detection reads `$COLORTERM` and `$TERM`, and both lie in either direction — inside tmux,
    /// over ssh, under a CI runner. A config that knows what it is talking to should be able to
    /// say so.
    pub color_depth: Option<String>,
    /// Whether an interactive oslo started inside another one asks before nesting.
    ///
    /// **On by default**, because a nested shell is invisible: it looks like the shell you were
    /// already at, and the usual way to find out is an `exit` that does not close the terminal.
    /// Off is for somebody who nests deliberately — `$OSLO_NESTED` still counts either way.
    pub nested_ask: bool,
    /// Whether `--help` reports what is wrong with this installation.
    ///
    /// **On by default**, because the things it checks are ones you cannot see from inside a
    /// working shell: `/bin/sh` still pointing at dash, a binary nobody but you can execute. A
    /// user who has read the warning and decided against acting on it turns it off with
    /// `oslo.misc.warnings = false`; the default cannot be silence, or an installation that is
    /// half-finished looks finished.
    pub warnings: bool,
    /// What repeated Ctrl-C should do to a job that will not take one. See [`InterruptEscape`].
    pub interrupt_escape: InterruptEscape,
}

impl Default for Misc {
    fn default() -> Self {
        Misc {
            welcome: true,
            idle_timeout: 0,
            greeting: None,
            // The standard pause: long enough that a real sequence is never split, short enough
            // that Esc feels immediate.
            escape_delay: 25,
            color_depth: None,
            nested_ask: true,
            warnings: true,
            // Off: it costs a process, and it changes what Ctrl-C means.
            interrupt_escape: InterruptEscape::default(),
        }
    }
}

/// `oslo.transcript` — what a finished line leaves on the screen in place of its prompt.
///
/// ```lua
/// oslo.transcript.rule = "- "
/// ```
///
/// **Empty is off, and off is the default.** With a rule set, running a line clears the prompt
/// block and writes the rule, the command, and the rule again; the command's output follows under
/// it. What scrolls back is then a record of *what was run*, not of what the prompt looked like at
/// the time — which is the half of it anybody ever reads, and the half that survives being copied
/// out of a terminal into a bug report.
///
/// The string is a **unit, repeated to the width of the terminal**, so `"- "` is a dashed rule
/// across the screen rather than two characters in the corner. It is drawn plain, in the theme's
/// `prompt.aside` — the slot for text that is there to be looked past.
///
/// A line that is only whitespace leaves nothing: there is no command to frame, and a pair of
/// rules around an empty row is a worse transcript than no rules at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub rule: String,
    /// `oslo.transcript.osc` — the OSC number the frame marks are written with.
    ///
    /// See [`crate::transcript`] for why oslo has one of its own and what is refused.
    pub osc: u32,
}

impl Default for Transcript {
    fn default() -> Self {
        Transcript {
            // Empty: a setting that changes the shape of the scrollback is asked for, never assumed.
            rule: String::new(),
            osc: crate::transcript::DEFAULT_OSC,
        }
    }
}
