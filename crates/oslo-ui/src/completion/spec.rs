//! Completing from a command's declared arguments, and from the alias table.
//!
//! Split from `completion.rs` when it crossed the 600-line limit, along the seam the subject
//! already had: every other builder answers from a name or the filesystem, and this one answers
//! from a *declaration* — argc comments, a shipped spec, a config's `oslo.completion.spec` — after
//! resolving whatever alias stands in front of it.

use super::{CompletionCandidate, matches_prefix};
use crate::OsloHelper;
use crate::words::{Word, quote_replacement, unquote};

impl OsloHelper {
    /// The command a name really stands for, following aliases.
    ///
    /// Transitive, because aliases chain — `alias g=git`, `alias gc='git commit'` — and bounded,
    /// because they can also loop: `alias a=b` with `alias b=a` is legal to write and must not
    /// hang the shell on Tab.
    ///
    /// **The whole expansion, not just its head.** `alias gco='git checkout'` used to complete as
    /// plain `git`, so `gco -<Tab>` offered git's top-level options — `--version`, `-C` — where
    /// `git checkout -<Tab>` offers `--force` and `-b`. The subcommand the alias names was dropped,
    /// and everybody aliases `gco`.
    fn resolve_alias(&self, name: &str) -> Vec<String> {
        let Ok(env) = self.env.lock() else {
            return vec![name.to_string()];
        };
        let mut words = vec![name.to_string()];
        for _ in 0..16 {
            let head = words[0].clone();
            let Some(expansion) = env.alias(&head) else {
                break;
            };
            let expanded: Vec<String> = expansion
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>();
            let Some(first) = expanded.first() else {
                break;
            };
            if *first == head {
                // `alias ls='ls --color'` — the classic self-reference. It expands to itself, so
                // the answer is already right and following it again would not terminate.
                break;
            }
            // The words this round added come first, then whatever earlier rounds left behind:
            // `alias g=git`, `alias gc='g commit'` has to reach `git commit`, in that order.
            let carried = words[1..].to_vec();
            words = expanded;
            words.extend(carried);
        }
        words
    }

    /// Just the name, for callers that only need to know what is being run.
    pub(super) fn resolve_head(&self, name: &str) -> String {
        self.resolve_alias(name)
            .first()
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    pub(super) fn spec_candidates(&self, word: &Word<'_>, out: &mut Vec<CompletionCandidate>) {
        // `prior_words` holds this command's words only, so `ls | git comm<TAB>` looks up `git`
        // and not `ls`.
        let Some((primary, rest)) = word.prior_words.split_first() else {
            return;
        };
        // Through the alias table first. Everyone aliases `git`, and `g comm<TAB>` offering
        // nothing is a gap the shell has no excuse for: the alias table is already loaded and this
        // function is already holding the environment.
        let expanded = self.resolve_alias(&unquote(primary));
        let Some((head, from_alias)) = expanded.split_first() else {
            return;
        };
        let Some(spec) = self.spec_registry.find_spec(head) else {
            return;
        };

        // Walk down the subcommand tree following what has already been typed, so
        // `git commit --a<TAB>` offers `--amend` and not git's own top-level options.
        //
        // **The alias's own words are walked first.** `gco` *is* `git checkout`, so the two halves
        // of the line are the words the alias supplied and then the words that were typed after it.
        let mut subcommands = &spec.subcommands;
        let mut options = &spec.options;
        let typed = rest.iter().map(|token| unquote(token));
        for token in from_alias.iter().cloned().chain(typed) {
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

        // Through `matches_prefix` like every other builder, so `oslo.completion.case_sensitive`
        // and the fuzzy passes reach spec names too. A raw `starts_with` here left `git COMM`
        // matching nothing while `ls RE` folded case, in the same dropdown.
        let stem = word.stem.as_str();
        let fold = self.case_sensitive();
        if stem.starts_with('-') {
            for opt in options {
                for name in &opt.names {
                    if matches_prefix(name, stem, fold) {
                        out.push(self.spec_candidate(word, name, &opt.description, "flag"));
                    }
                }
            }
        } else {
            for sub in subcommands {
                if matches_prefix(&sub.name, stem, fold) {
                    out.push(self.spec_candidate(word, &sub.name, &sub.description, "subcommand"));
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
}
