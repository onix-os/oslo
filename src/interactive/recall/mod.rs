//! Which remembered lines belong to the language being typed.
//!
//! The line editor holds one history. oslo has two languages, and a shell line and a Lua line are
//! not alternatives for the same slot: offering `ls -la` at a Lua prompt suggests something that
//! cannot run.
//!
//! The set lives here, in the library, rather than beside the read loop, because the *suggestion*
//! needs it. Swapping the editor's own history when the language changes is not enough — the
//! language can change in the middle of a line, from a key handler that has no way to reach the
//! editor, and until the line ends the editor is still holding the other language's history.
//!
//! # And where you are standing
//!
//! One history for every directory is the wrong answer to `cargo run --ex`: whichever project you
//! last typed it in wins in all of them. So the suggestion asks three questions in order — what you
//! have run *here*, then what you have run anywhere in *this worktree*, then what you have run at
//! all — and takes the first answer. The first two come from [`crate::track`], which keys a line by
//! the directory it ran in; the third is the flat walk below, which is what oslo did before the
//! store existed and is still what it does in a directory the store has never seen.

mod nearby;

use super::matching::{Fuzzy, fuzzy_score};
use super::prompt;
use crate::track;
use nearby::{forget_answers, forget_answers_for, from_store, place};
use std::sync::Mutex;

/// One lock for every test under `recall`, in this file and in `nearby`.
///
/// The remembered set, the resolved place and the memo of answers are all process-wide, and these
/// tests replace them wholesale, so they cannot run beside each other — in either file.
#[cfg(test)]
static SERIAL: Mutex<()> = Mutex::new(());

/// Every remembered line with the language it was typed in, oldest first.
static REMEMBERED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Replace everything remembered, oldest first.
pub fn seed(entries: Vec<(String, String)>) {
    if let Ok(mut all) = REMEMBERED.lock() {
        *all = entries;
    }
    forget_answers();
}

/// Remember a line typed this session.
pub fn remember(line: &str, language: &str) {
    if let Ok(mut all) = REMEMBERED.lock() {
        all.push((line.to_string(), language.to_string()));
    }
    // The store is about to learn this line too, which is the one thing that can make an answer
    // already given wrong. See [`forget_answers_for`].
    forget_answers_for(line, language);
}

/// Forget everything remembered, in every language.
///
/// `history -c` means "forget the history", and a shell that went on suggesting and recalling the
/// lines it had just been told to forget would be lying — the same reason `hash -r` invalidates the
/// command index.
pub fn clear() {
    if let Ok(mut all) = REMEMBERED.lock() {
        all.clear();
    }
    forget_answers();
}

/// How many lines are remembered, in every language.
///
/// What `\!` in a `$PS1` counts: the number the next command will be given.
pub fn len() -> usize {
    REMEMBERED.lock().map(|all| all.len()).unwrap_or(0)
}

/// Whether nothing at all has been remembered, in any language.
///
/// Distinguishes "this language has no history" from "there is no history" — the first means a
/// recall key should do nothing, the second that it should fall through to the editor's own.
pub fn is_empty() -> bool {
    REMEMBERED.lock().map(|all| all.is_empty()).unwrap_or(true)
}

/// Every line typed in `language`, oldest first.
pub fn for_language(language: &str) -> Vec<String> {
    let Ok(all) = REMEMBERED.lock() else {
        return Vec::new();
    };
    all.iter()
        .filter(|(_, l)| l == language)
        .map(|(line, _)| line.clone())
        .collect()
}

/// The line to offer as ghost text for `line`, minus what is already typed.
///
/// Three sources, narrowest first: this directory, this worktree, then everything remembered. The
/// first two are the store's, and both are silent in a shell that has none — a script, an `oslo
/// -c`, a subshell — so this degrades to exactly the flat walk it has always been rather than going
/// quiet.
///
/// It answers for the language the prompt is reading *now* rather than the one the line started in,
/// which is the whole reason this lives here instead of in the editor's own history hinter. That
/// language is also what the store is keyed by, so a shell line can no more be offered at a Lua
/// prompt from the database than it can from the remembered set.
pub fn suggest(line: &str) -> Option<String> {
    if line.is_empty() {
        return None;
    }
    let language = prompt::language()?;
    if let Some(track) = track::store()
        && let Some(place) = place()
        && let Some(found) = from_store(track, &place, &language, line)
    {
        return Some(found);
    }
    remembered(&language, line)
}

/// The fuzzy suggestion currently on offer, as `(what was typed, the line it would become)`.
///
/// A slot rather than a recomputation at accept time: the hinter has already done the search on
/// this keystroke, and doing it again from the key handler could answer differently — the store or
/// the remembered set may have moved between the two — which would replace the line with something
/// the user was never shown. The typed half is kept so acceptance can refuse a stale offer.
static OFFERED: Mutex<Option<(String, String)>> = Mutex::new(None);

/// Record the fuzzy line being shown, so the accept key can put it in the buffer.
pub fn offer_fuzzy(typed: &str, full: &str) {
    if let Ok(mut slot) = OFFERED.lock() {
        *slot = Some((typed.to_string(), full.to_string()));
    }
}

/// Forget any fuzzy offer. Called whenever the hinter produces something else, or nothing.
pub fn withdraw_fuzzy() {
    if let Ok(mut slot) = OFFERED.lock() {
        *slot = None;
    }
}

/// The line a fuzzy suggestion would put in the buffer, if one is on offer for exactly this text.
///
/// The equality check is what makes this safe: a suggestion offered for `crne` must not be accepted
/// onto `crnex`, and without the check a keystroke arriving between the hint and the key would do
/// exactly that.
pub fn accept_fuzzy(typed: &str) -> Option<String> {
    let slot = OFFERED.lock().ok()?;
    let (offered, full) = slot.as_ref()?;
    (offered == typed).then(|| full.clone())
}

/// The best remembered line that `line` is a gap-capped subsequence of, in full.
///
/// **This returns the whole line, not a remainder, and that is the difference that matters.** A
/// prefix suggestion continues what you typed, so it can be drawn as grey text after the cursor and
/// accepted without reading it. A fuzzy one does not continue anything — `crne` proposing
/// `cargo run --example xyz` replaces every character you typed — so the caller has to draw it as a
/// replacement and accepting it has to overwrite the line rather than extend it.
///
/// Searched over the remembered set rather than the store: this runs per keystroke only once every
/// prefix path has already come back empty, and the remembered set is in memory and already filtered
/// to the current language. A directory-scoped fuzzy pass would need a query that returns candidate
/// rows rather than an answer, which is a range scan turned into a table scan.
pub fn suggest_fuzzy(line: &str, fuzzy: Fuzzy) -> Option<String> {
    if line.is_empty() || fuzzy == Fuzzy::Off {
        return None;
    }
    let language = prompt::language()?;
    let all = REMEMBERED.lock().ok()?;
    all.iter()
        .filter(|(candidate, l)| {
            l == &language
                && candidate != line
                && !candidate.contains('\n')
                // A candidate this line already prefixes is the prefix path's answer, and it has
                // already declined to give it — offering it here would suggest a line the stricter
                // pass deliberately refused.
                && !candidate.starts_with(line)
        })
        .filter_map(|(candidate, _)| {
            fuzzy_score(candidate, line, fuzzy).map(|score| (score, candidate))
        })
        // Newest wins a tie: `max_by_key` keeps the last maximum, and the set is oldest-first.
        .max_by_key(|(score, _)| *score)
        .map(|(_, candidate)| candidate.clone())
}

/// The newest line in `language` that starts with `line`, minus what is already typed.
///
/// What oslo suggested before it had a store, and what it still suggests in a directory the store
/// has never seen. Newest wins, because there is nothing else to rank by here.
fn remembered(language: &str, line: &str) -> Option<String> {
    let Ok(all) = REMEMBERED.lock() else {
        return None;
    };
    all.iter()
        .rev()
        .find(|(candidate, l)| {
            l == language
                && candidate.starts_with(line)
                && candidate != line
                // **Never a multi-line entry.** A command continued over several lines is
                // remembered as one entry with newlines in it, and a suggestion is drawn as ghost
                // text on the row you are typing on. Printing it raw does exactly what the bytes
                // say: the terminal breaks the line and the rest of the entry appears as extra
                // rows under the prompt, stuck to whatever was already there.
                && !candidate.contains('\n')
        })
        .map(|(candidate, _)| candidate[line.len()..].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `suggest` answers for the language the prompt is showing, so a test has to say what that is.
    fn prompt_in(language: &str) {
        crate::interactive::row::note_row(language, 0, 0, true);
    }

    /// A remembered command spanning several lines is never offered as ghost text: it would be
    /// drawn literally, breaking the row and leaving its tail underneath the prompt.
    #[test]
    fn a_multi_line_entry_is_never_suggested() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        prompt_in("sh");
        seed(vec![
            ("ll\nls\nks".to_string(), "sh".to_string()),
            ("llama".to_string(), "sh".to_string()),
        ]);
        // The single-line entry is still offered; the multi-line one is passed over.
        assert_eq!(suggest("ll"), Some("ama".to_string()));
        seed(vec![("ll\nls".to_string(), "sh".to_string())]);
        assert_eq!(suggest("ll"), None);
        seed(Vec::new());
    }

    /// A suggestion never crosses languages, whichever way round they were typed.
    #[test]
    fn a_suggestion_stays_in_its_own_language() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        seed(vec![
            ("echo one".to_string(), "sh".to_string()),
            ("echo two".to_string(), "lua".to_string()),
        ]);
        assert_eq!(for_language("sh"), vec!["echo one".to_string()]);
        assert_eq!(for_language("lua"), vec!["echo two".to_string()]);
        seed(Vec::new());
    }
}
