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

use std::sync::Mutex;

/// Every remembered line with the language it was typed in, oldest first.
static REMEMBERED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Replace everything remembered, oldest first.
pub fn seed(entries: Vec<(String, String)>) {
    if let Ok(mut all) = REMEMBERED.lock() {
        *all = entries;
    }
}

/// Remember a line typed this session.
pub fn remember(line: &str, language: &str) {
    if let Ok(mut all) = REMEMBERED.lock() {
        all.push((line.to_string(), language.to_string()));
    }
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

/// The newest line in the current language that starts with `line`, minus what is already typed.
///
/// This is the suggestion the editor's own history hinter would give, except that it answers for
/// the language the prompt is reading *now* rather than the one the line started in.
pub fn suggest(line: &str) -> Option<String> {
    if line.is_empty() {
        return None;
    }
    let language = super::prompt::language()?;
    let Ok(all) = REMEMBERED.lock() else {
        return None;
    };
    all.iter()
        .rev()
        .find(|(candidate, l)| {
            l == &language
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

    /// The remembered set is one process-wide store and these tests replace it wholesale, so they
    /// cannot run beside each other.
    static SERIAL: Mutex<()> = Mutex::new(());

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
