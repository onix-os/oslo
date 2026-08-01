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
        .find(|(candidate, l)| l == &language && candidate.starts_with(line) && candidate != line)
        .map(|(candidate, _)| candidate[line.len()..].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A suggestion never crosses languages, whichever way round they were typed.
    #[test]
    fn a_suggestion_stays_in_its_own_language() {
        seed(vec![
            ("echo one".to_string(), "sh".to_string()),
            ("echo two".to_string(), "lua".to_string()),
        ]);
        assert_eq!(for_language("sh"), vec!["echo one".to_string()]);
        assert_eq!(for_language("lua"), vec!["echo two".to_string()]);
        seed(Vec::new());
    }
}
