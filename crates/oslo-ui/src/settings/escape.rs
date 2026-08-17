//! `oslo.misc.interrupt_escape` — the way out of a job that ignores Ctrl-C.
//!
//! Its own file because it is a subject rather than a knob: a count, an action and a notice, each
//! of which the watcher in `oslo_shell::exec::job::sentinel` reads. `settings/mod.rs` holds the
//! flat preferences; this is the one that grew a shape.

/// `oslo.misc.interrupt_escape` — the way out of a job that ignores Ctrl-C.
///
/// ```lua
/// oslo.misc.interrupt_escape = 3                            -- the short form
/// oslo.misc.interrupt_escape = { after = 3, action = "stop", notify = true }
/// ```
///
/// **For the job that will not take a Ctrl-C.** A shell doing job control is not in the terminal's
/// foreground process group, so it never sees the keystroke at all — a program that traps `INT` and
/// keeps going leaves you with a wedged terminal and no way out but a new window. With this set, a
/// watcher inside that group counts the interrupts and acts on the *n*th; see
/// `exec::job::sentinel` for why that takes a second process.
///
/// **Off by default**, because it costs that process and because it changes what Ctrl-C means,
/// which nobody should discover by accident. `0` — the default — forks nothing.
///
/// None of it can rescue a process wedged in an uninterruptible kernel call: every signal waits for
/// the syscall to return, `SIGKILL` included. Nothing can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptEscape {
    /// How many interrupts in one command before acting. `0` is off, and is the default.
    pub after: u64,
    /// What to do when the count is reached.
    pub action: EscapeAction,
    /// Whether the press *before* the last one says what the next will do.
    ///
    /// **On by default**, because a feature nobody knows fired is a feature nobody has. The
    /// second Ctrl-C of three is exactly the moment a person is deciding whether anything is
    /// listening, and one line answers it.
    pub notify: bool,
}

impl Default for InterruptEscape {
    fn default() -> Self {
        InterruptEscape {
            // Off. The whole feature is opt-in.
            after: 0,
            action: EscapeAction::Stop,
            // On once the feature is: a person who has turned it on wants to know it fired.
            notify: true,
        }
    }
}

/// What the watcher does to a job that has been interrupted enough times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EscapeAction {
    /// `SIGSTOP` — the default, and the only one that destroys nothing.
    ///
    /// It cannot be caught or ignored, so it works on exactly the programs this exists for, and
    /// `waitpid` already reports it — so the shell's own Ctrl-Z path takes over, the job lands in
    /// the job table, and `fg`, `bg` and `kill %1` all still mean something.
    #[default]
    Stop,
    /// `SIGKILL` — for somebody who would rather the job were simply gone.
    Kill,
    /// `SIGHUP` — what a closing terminal sends, which many daemons treat as "reload or exit".
    Hup,
    /// `SIGQUIT` — Ctrl-\'s signal, which dumps core where that is enabled.
    Quit,
}

impl EscapeAction {
    /// The name a config writes, and the name a hook reports.
    pub fn name(self) -> &'static str {
        match self {
            EscapeAction::Stop => "stop",
            EscapeAction::Kill => "kill",
            EscapeAction::Hup => "hup",
            EscapeAction::Quit => "quit",
        }
    }

    /// The action a config named, or `None` if it named nothing known.
    pub fn from_name(name: &str) -> Option<EscapeAction> {
        match name {
            "stop" => Some(EscapeAction::Stop),
            "kill" => Some(EscapeAction::Kill),
            "hup" => Some(EscapeAction::Hup),
            "quit" => Some(EscapeAction::Quit),
            _ => None,
        }
    }

    /// Every name, for a diagnostic that has to list them.
    pub const NAMES: [&'static str; 4] = ["stop", "kill", "hup", "quit"];
}
