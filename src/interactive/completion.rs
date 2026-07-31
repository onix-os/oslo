//! Building the candidate list for Tab.
//!
//! Split out of the rustyline glue so that the interesting part — which candidates, spelled how —
//! can be called from a test with no terminal attached.

use super::OsloHelper;
use super::command_index::CommandIndex;
use super::dropdown::CompletionCandidate;
use super::words::{Quote, Word, current_word, quote_replacement, unquote};
use std::fs;

impl OsloHelper {
    /// The candidates for the word at `pos`, together with the byte offset they replace from.
    ///
    /// Every replacement is already quoted for the context it lands in: the old code handed
    /// rustyline a bare `entry.file_name()`, so `wc -c My<TAB>` produced `wc -c My File.txt` and
    /// three "no such file" errors.
    pub fn candidates(&self, line: &str, pos: usize) -> (usize, Vec<CompletionCandidate>) {
        let word = current_word(line, pos);
        let mut out = Vec::new();

        if let Some(prefix) = word.stem.strip_prefix('$') {
            self.variable_candidates(prefix, word.quote, &mut out);
        } else if word.command_position {
            self.command_candidates(&word, &mut out);
        } else {
            self.spec_candidates(&word, &mut out);
            if out.is_empty() {
                self.path_candidates(&word, &mut out);
            }
        }

        // Frecency first, name second. Without the first key this is alphabetical, which is how
        // `exit` came to suggest `exitsnoop-bpfcc`.
        out.sort_by(|a, b| {
            let sa = self.frecency.score(&a.display);
            let sb = self.frecency.score(&b.display);
            sb.partial_cmp(&sa)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.display.cmp(&b.display))
        });
        out.dedup_by(|a, b| a.replacement == b.replacement);

        (word.start, out)
    }

    fn variable_candidates(&self, prefix: &str, quote: Quote, out: &mut Vec<CompletionCandidate>) {
        let env = self.env.lock().unwrap();
        for name in env.get_all_vars().keys() {
            if name.starts_with(prefix) {
                let value = format!("${}", name);
                out.push(CompletionCandidate {
                    display: value.clone(),
                    // `$` survives quoting only outside double quotes; inside them it would be
                    // escaped and stop expanding, so a variable is always offered unquoted.
                    replacement: if quote == Quote::None {
                        value
                    } else {
                        quote_replacement(&value, quote)
                    },
                    // As above: the ` variable ` badge already says this.
                    description: None,
                    kind: Some("variable".to_string()),
                    path: None,
                    detail: None,
                });
            }
        }
    }

    fn command_candidates(&self, word: &Word<'_>, out: &mut Vec<CompletionCandidate>) {
        let stem = word.stem.as_str();
        // Kind and detail are decided here, where the environment is already locked. An alias is
        // its own kind rather than a `builtin`: they behave differently, and lumping them meant
        // the badge told you `builtin` about something you had defined yourself a minute earlier.
        let (path, shell_names) = {
            let env = self.env.lock().unwrap();
            let mut names: Vec<(String, &str, Option<String>)> = Vec::new();
            for b in env.builtin_names() {
                if b.starts_with(stem) {
                    names.push((b.to_string(), "builtin", None));
                }
            }
            for (name, target) in env.get_aliases() {
                if name.starts_with(stem) {
                    // What it expands to travels with it: that is the one thing about an alias
                    // its name does not tell you, and it is why aliases keep a second column
                    // where a directory does not.
                    names.push((name.clone(), "alias", Some(target.clone())));
                }
            }
            for f in env.get_functions().keys() {
                if f.starts_with(stem) {
                    names.push((f.clone(), "function", None));
                }
            }
            (env.get_var("PATH").unwrap_or_default().to_string(), names)
        };

        for (name, kind, detail) in shell_names {
            let mut candidate = self.command_candidate(word, name, kind);
            candidate.detail = detail;
            out.push(candidate);
        }
        // The index is shared, not rebuilt: this used to `read_dir` all of `$PATH` per keystroke.
        for name in CommandIndex::executables(&path).iter() {
            if name.starts_with(stem) {
                out.push(self.command_candidate(word, name.clone(), "command"));
            }
        }
    }

    fn command_candidate(&self, word: &Word<'_>, name: String, kind: &str) -> CompletionCandidate {
        let description = self
            .spec_registry
            .find_spec(&name)
            .map(|s| s.description.to_string());
        CompletionCandidate {
            display: name.clone(),
            replacement: quote_replacement(&name, word.quote),
            description,
            kind: Some(kind.to_string()),
            path: None,
            detail: None,
        }
    }

    fn spec_candidates(&self, word: &Word<'_>, out: &mut Vec<CompletionCandidate>) {
        // `prior_words` holds this command's words only, so `ls | git comm<TAB>` looks up `git`
        // and not `ls`.
        let Some((primary, rest)) = word.prior_words.split_first() else {
            return;
        };
        let Some(spec) = self.spec_registry.find_spec(&unquote(primary)) else {
            return;
        };

        // Walk down the subcommand tree following what has already been typed, so
        // `git commit --a<TAB>` offers `--amend` and not git's own top-level options.
        let mut subcommands = &spec.subcommands;
        let mut options = &spec.options;
        for token in rest {
            let token = unquote(token);
            // Flags on the way down do not change which subcommand we are inside.
            if token.starts_with('-') {
                continue;
            }
            match subcommands.iter().find(|s| s.name == token) {
                Some(found) => {
                    subcommands = &found.subcommands;
                    options = &found.options;
                }
                // An argument we do not recognise: stop rather than guess at a deeper level.
                None => break,
            }
        }

        if word.stem.starts_with('-') {
            for opt in options {
                for name in &opt.names {
                    if name.starts_with(word.stem.as_str()) {
                        out.push(self.spec_candidate(word, name, opt.description, "flag"));
                    }
                }
            }
        } else {
            for sub in subcommands {
                if sub.name.starts_with(word.stem.as_str()) {
                    out.push(self.spec_candidate(word, sub.name, sub.description, "subcommand"));
                }
            }
        }
    }

    fn spec_candidate(
        &self,
        word: &Word<'_>,
        name: &str,
        description: &str,
        kind: &str,
    ) -> CompletionCandidate {
        CompletionCandidate {
            display: name.to_string(),
            replacement: quote_replacement(name, word.quote),
            description: Some(description.to_string()),
            kind: Some(kind.to_string()),
            path: None,
            detail: None,
        }
    }

    fn path_candidates(&self, word: &Word<'_>, out: &mut Vec<CompletionCandidate>) {
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
            if !name.starts_with(prefix) {
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
