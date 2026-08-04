//! The line editor the loop reads through: how it is configured, and what it remembers.
//!
//! Split out of [`super`] rather than left in it because none of this is the loop. Every function
//! here answers "what does rustyline do with a line", the loop answers "what does the shell do with
//! one", and the two questions kept drifting into each other at the top of the same file. The
//! rationale comments are the point of the split as much as the code is: the `keyseq_timeout` and
//! `completion_type` arguments below are each a bug that was chased through the editor, and they
//! belong beside the setting rather than in front of the REPL.

use super::Repl;
use crate::startup::history;
use rustyline::Editor;
use rustyline::history::{History, SearchDirection};
use std::path::PathBuf;

/// R9.11: the editor's history configuration, which used to be left entirely at its defaults.
pub(super) fn build_editor(settings: &history::Settings) -> Repl {
    // History is added by hand in `remember`, not automatically: what belongs in the history is
    // the line *after* history expansion, so that `!!` recalls the command it stood for rather
    // than itself, and a multi-line command belongs there as one entry rather than three.
    // Repeats are kept rather than folded into the previous entry. rustyline drops a consecutive
    // duplicate by default, which would silently renumber every later event and make `!-2` point
    // one line too far back — bash's default `HISTCONTROL` keeps them, and `!n` only means
    // anything if the numbering agrees.
    let config = rustyline::Config::builder()
        .auto_add_history(false)
        // `oslo.history.ignore_dups`. Off by default because dropping a duplicate silently
        // renumbers every later event and makes `!-2` point one line too far back — bash's
        // default `HISTCONTROL` keeps them, and `!n` only means anything if the numbering agrees.
        .history_ignore_dups(settings.ignore_dups)
        .expect("history duplicate policy")
        // rustyline's own default is 100 entries, which loses a working day's commands.
        .max_history_size(settings.max_size)
        .expect("history size")
        // `oslo.history.ignore_space`. Honoured for anything rustyline adds itself;
        // `history::is_secret` covers the entries this file adds by hand, which is all of them.
        .history_ignore_space(settings.ignore_space)
        // `oslo.vi.enabled`. Read here rather than toggled later because the keymap is fixed when
        // the editor is built.
        .edit_mode(if oslo::interactive::settings::current().vi.enabled {
            rustyline::EditMode::Vi
        } else {
            rustyline::EditMode::Emacs
        })
        // **How long Esc waits for a second byte.** rustyline's default is to wait *forever*:
        // Esc is also the first byte of every arrow key and function key, so with no timeout the
        // editor cannot tell "the user pressed Esc" from "a sequence is arriving" until the next
        // byte turns up. In vi mode that means Esc appears to do nothing until you press
        // something else — which is why leaving insert mode felt like it took two presses. It
        // took one; the first just could not be acted on yet.
        //
        // 25ms is far longer than a terminal takes to write the rest of a sequence, which it
        // sends in one burst, and far shorter than a person can notice. fish's `fish_escape_delay`
        // defaults to the same order for the same reason.
        .keyseq_timeout(Some(25))
        // `List`, not `Circular`, and the reason is the dropdown.
        //
        // oslo's completer opens its own menu, waits for a choice, and returns that one candidate
        // already decided. Under `Circular` rustyline then starts a *second* selection loop over
        // that single candidate: it inserts it, waits for a key, and reads Tab as "next" — which
        // with one candidate wraps to the index past the end, whose meaning is *restore the
        // original line*. So accepting a completion and then pressing Tab silently deleted it.
        //
        // `List` applies a lone candidate and returns immediately, leaving Tab to start a fresh
        // completion, which is what the menu having already asked makes correct.
        .completion_type(rustyline::CompletionType::List)
        .build();
    Editor::with_config(config).expect("Failed to initialize line editor")
}

/// Add a command to the history, and to the history *file*, before it runs.
///
/// Appending rather than rewriting is the fix for the third of R9.11's defects: `save_history`
/// writes the whole file, so two sessions open at once each ended with only their own commands.
/// Writing before the command runs is deliberate too — a command that exits the shell, or hangs
/// until it is killed, is exactly the one you want to find in the history afterwards.
pub(super) fn remember(rl: &mut Repl, file: &Option<PathBuf>, text: &str, secret: bool) {
    if secret {
        return;
    }
    let _ = rl.add_history_entry(text);
    publish_history(rl);
    if let Some(path) = file
        && let Err(e) = rl.append_history(path)
    {
        eprintln!(
            "oslo: {}: {}",
            oslo::interactive::marks::path(&path.display().to_string()),
            e
        );
    }
}

pub(super) fn history_entries(rl: &Repl) -> Vec<String> {
    let history = rl.history();
    (0..history.len())
        .filter_map(|i| {
            history
                .get(i, SearchDirection::Forward)
                .ok()
                .flatten()
                .map(|r| r.entry.into_owned())
        })
        .collect()
}

/// Hand the `history` builtin the entries it prints.
pub(super) fn publish_history(rl: &Repl) {
    history::publish(history_entries(rl));
}
