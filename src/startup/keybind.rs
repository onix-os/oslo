//! Applying `oslo.keys` to the line editor.
//!
//! Split from `repl` because it is the one place the editor's key map is touched, and because the
//! language toggle needs a different kind of binding from everything else: rustyline has no
//! command that hands control back to the caller, so the toggle is a conditional handler that sets
//! a flag the loop reads.

use super::mode::{self, ToggleRequest};
use super::repl::Repl;
use oslo::Environment;
use std::sync::{Arc, Mutex};

/// Apply `oslo.keys`, plus the language toggle.
///
/// The toggle is bound last so a config that puts something else on the same key wins: the config
/// is a later, more specific statement than the default.
pub fn apply(rl: &mut Repl, env_struct: &Arc<Mutex<Environment>>, toggle: &ToggleRequest) {
    let settings = oslo::interactive::settings::current();
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
