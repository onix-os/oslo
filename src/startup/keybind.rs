//! Applying `oslo.keys` to the line editor.
//!
//! Split from `repl` because it is the one place the editor's key map is touched, and because the
//! language toggle needs a different kind of binding from everything else: rustyline has no
//! command that hands control back to the caller, so the toggle is a conditional handler that sets
//! a flag the loop reads.

use super::mode::{self, ToggleRequest};
use super::repl::Repl;
use oslo::Environment;
use oslo::interactive::vi;
use rustyline::{Cmd, ConditionalEventHandler, Event, EventContext, RepeatCount};
use std::io::Write;
use std::sync::{Arc, Mutex};

/// Watches every keystroke to keep the cursor shape agreeing with the vi mode.
///
/// A wildcard handler that **never handles anything**: it reads the mode rustyline is now in,
/// writes a cursor escape if that changed, and returns `None` so the real binding for the key runs
/// exactly as it would have. This is the only place rustyline exposes the vi mode at all —
/// `EventContext::input_mode` — and it is why fish's "the cursor tells you the mode" is possible
/// here without patching the editor.
struct ViCursor {
    cursors: vi::Cursors,
}

impl ConditionalEventHandler for ViCursor {
    fn handle(&self, event: &Event, _: RepeatCount, _: bool, ctx: &EventContext) -> Option<Cmd> {
        let now = match ctx.input_mode() {
            rustyline::InputMode::Insert => vi::Mode::Insert,
            rustyline::InputMode::Command => vi::Mode::Normal,
            rustyline::InputMode::Replace => vi::Mode::Replace,
        };
        // **The key has not been applied yet**, so `input_mode` is the mode this keystroke is
        // about to leave. Reporting it as-is drew the cursor one key behind: Esc changed nothing
        // until you pressed something else, which is exactly when a mode indicator is useless.
        //
        // So the keys that *enter* a mode are predicted. This is a short list, not the vi keymap:
        // everything else leaves the mode alone, and a key that is mispredicted corrects itself on
        // the very next keystroke because the real mode is read again then.
        let mode = vi::after_key(now, key_char(event));
        // The mode changed, so the prompt is now wrong. rustyline will not redraw it and cannot be
        // asked to, so oslo draws the row again itself from what the highlighter recorded — see
        // `prompt::repaint`. Without this the letter sat there saying `I` while the cursor had
        // already become a block, which is worse than showing nothing.
        if let Some(escape) = vi::observe(mode, &self.cursors) {
            // Where the cursor *actually* is, not where the line ends: the editor hands over the
            // line and the byte position, so the column is exact. Using the end of the line
            // instead dragged the cursor right on every mode change.
            let cursor = oslo::interactive::prompt::printed_width(&ctx.line()[..ctx.pos()]);
            let mut out = std::io::stdout();
            let _ = out.write_all(escape.as_bytes());
            // The row after the cursor shape, so the prompt and the cursor agree in one frame
            // rather than flickering between two.
            let _ =
                out.write_all(oslo::interactive::prompt::repaint(ctx.line(), cursor).as_bytes());
            let _ = out.flush();
        }
        // Declined on purpose: this handler observes, it does not bind.
        None
    }
}

/// The character a key event carries, if it is a plain one.
///
/// Esc arrives as its own key code rather than as a character, so it is reported as `\x1b` — which
/// is what it is, and what [`vi::after_key`] matches on.
fn key_char(event: &Event) -> Option<char> {
    let Event::KeySeq(keys) = event else {
        return None;
    };
    let key = keys.first()?;
    // Only an unmodified key. Ctrl-R arrives as `Char('r')` with a modifier, and reading it as a
    // bare `r` would have it answered by whatever `r` means — a real confusion now that the
    // insert-starting keys are matched on the character alone.
    if key.1 != rustyline::Modifiers::NONE {
        return None;
    }
    match key.0 {
        rustyline::KeyCode::Char(c) => Some(c),
        rustyline::KeyCode::Esc => Some('\x1b'),
        _ => None,
    }
}

/// Apply `oslo.keys`, plus the language toggle.
///
/// The toggle is bound last so a config that puts something else on the same key wins: the config
/// is a later, more specific statement than the default.
pub fn apply(rl: &mut Repl, env_struct: &Arc<Mutex<Environment>>, toggle: &ToggleRequest) {
    let settings = oslo::interactive::settings::current();

    // Bound before `oslo.keys`, because a wildcard that declines every event must not shadow a
    // real binding — and rustyline consults the more specific one first regardless.
    vi::set_enabled(settings.vi.enabled);
    if settings.vi.enabled {
        rl.bind_sequence(
            Event::Any,
            rustyline::EventHandler::Conditional(Box::new(ViCursor {
                cursors: settings.vi.cursors,
            })),
        );
        // **Tab completes in normal mode too.** rustyline's vi keymap leaves Tab unbound there, so
        // pressing it did nothing whatsoever — and since nothing happens, the natural response is
        // to press it again, which is how "the dropdown needs five presses" was really "you were
        // in normal mode and the first four did nothing".
        //
        // Safe to bind: Tab is not a vi command. Nothing is being taken away.
        rl.bind_sequence(
            Event::KeySeq(vec![rustyline::KeyEvent(
                rustyline::KeyCode::Tab,
                rustyline::Modifiers::NONE,
            )]),
            rustyline::EventHandler::Simple(Cmd::Complete),
        );

        // The prompt is drawn before any key is pressed, so the starting shape has to be written
        // by hand — otherwise the first line of the session has whatever cursor the terminal had.
        let mut out = std::io::stdout();
        if std::io::IsTerminal::is_terminal(&out) {
            let _ = out.write_all(settings.vi.cursors.insert.escape().as_bytes());
            let _ = out.flush();
        }
    }

    let (bindings, problems) = oslo::interactive::keys::resolve(&settings.keys);
    for problem in problems {
        eprintln!("oslo: {problem}");
    }

    // `oslo.suggest.accept` / `.accept_word` name the same actions `oslo.keys` can bind, under the
    // names the suggestion settings use. Applied first so an explicit `oslo.keys` entry on the same
    // key still wins: a later, more specific statement beats a general one.
    for (key, action) in [
        (settings.suggest.accept.as_deref(), "accept-suggestion"),
        (
            settings.suggest.accept_word.as_deref(),
            "accept-suggestion-word",
        ),
    ] {
        let Some(key) = key else { continue };
        match oslo::interactive::keys::parse_key(key) {
            Some(event) => {
                if let Some(command) =
                    oslo::interactive::keys::action(action).and_then(|a| a.command())
                {
                    rl.bind_sequence(event, rustyline::EventHandler::Simple(command));
                }
            }
            None => eprintln!("oslo: oslo.suggest: '{key}' is not a key name"),
        }
    }

    let mut toggle_bound = false;
    for (event, action) in bindings {
        match action.command() {
            Some(command) => {
                rl.bind_sequence(event, rustyline::EventHandler::Simple(command));
            }
            // The toggle hands control back to this loop, which no editor command can do.
            None => {
                rl.bind_sequence(
                    event,
                    rustyline::EventHandler::Conditional(Box::new(toggle.clone())),
                );
                toggle_bound = true;
            }
        }
    }

    if !toggle_bound && let Some(key) = mode::toggle_key(&env_struct.lock().unwrap()) {
        rl.bind_sequence(
            key,
            rustyline::EventHandler::Conditional(Box::new(toggle.clone())),
        );
    }
}
