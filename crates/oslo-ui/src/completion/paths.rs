//! Completing a word that names a file or a directory.
//!
//! Split from `completion.rs` when it crossed the 600-line limit. It is a subject of its own: every
//! other builder answers from something the shell already knows — its builtins, its specs, its
//! history — and this one is the only one that goes to the filesystem.

use super::{CompletionCandidate, matches_prefix};
use crate::OsloHelper;
use crate::words::{Word, quote_replacement, unquote};
use std::fs;

impl OsloHelper {
    pub(super) fn path_candidates(&self, word: &Word<'_>, out: &mut Vec<CompletionCandidate>) {
        let stem = word.stem.as_str();
        // Split on the *unquoted* value: `"My Dir/fi` has to look inside `My Dir`.
        let (dir_part, prefix) = match stem.rfind('/') {
            Some(i) => (&stem[..=i], &stem[i + 1..]),
            None => ("", stem),
        };

        let read_from = if let Some(rest) = dir_part.strip_prefix('~') {
            match std::env::var("HOME") {
                Ok(home) => format!("{}{}", home, rest),
                Err(_) => dir_part.to_string(),
            }
        } else if dir_part.is_empty() {
            ".".to_string()
        } else {
            dir_part.to_string()
        };

        let only_dirs = word
            .prior_words
            .first()
            .map(|w| unquote(w))
            .is_some_and(|c| matches!(c.as_str(), "cd" | "pushd" | "rmdir"));

        let Ok(entries) = fs::read_dir(&read_from) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !matches_prefix(&name, prefix, self.case_sensitive()) {
                continue;
            }
            // A dotfile only shows up when it was asked for, as in bash.
            if name.starts_with('.') && !prefix.starts_with('.') {
                continue;
            }
            // `metadata` follows symlinks: a link to a directory is a directory as far as `cd`
            // and the trailing slash are concerned.
            let is_dir = entry.metadata().map(|m| m.is_dir()).unwrap_or(false);
            if only_dirs && !is_dir {
                continue;
            }

            let display = if is_dir {
                format!("{}/", name)
            } else {
                name.clone()
            };
            // The replacement is the whole word, not just the tail: quoting a fragment would
            // leave the directory part unquoted and the two halves would not agree.
            let value = format!("{}{}", dir_part, display);
            out.push(CompletionCandidate {
                display,
                replacement: quote_replacement(&value, word.quote),
                // No description. The badge already says `dir` or `file`, and "Directory"
                // beside a ` dir ` badge is the same fact written twice — it also forces the
                // description column to exist for a listing that has nothing to put in it,
                // taking width from the names. This is IRIS's rule: where the *kind* is the
                // whole story the tag carries it alone, and only a kind that leaves something
                // unsaid (an alias, and what it expands to) gets both.
                description: None,
                kind: Some(if is_dir { "dir" } else { "file" }.to_string()),
                // The path the entry was read from, not one rebuilt from the typed text: a
                // `~/` stem reads from `$HOME` and would `stat` a directory literally named
                // `~` if the display were re-joined instead.
                path: Some(entry.path().to_string_lossy().into_owned()),
                detail: None,
            });
        }
    }
}
