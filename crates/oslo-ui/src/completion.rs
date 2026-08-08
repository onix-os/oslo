//! Builds completion candidates for Tab.

use super::OsloHelper;
use super::command_index::CommandIndex;
use super::dropdown::CompletionCandidate;
use super::matching::{Fuzzed, Fuzzy, Match, matchers, matches_ignoring_case};
use super::settings;
use super::words::{Quote, Word, current_word, quote_replacement, unquote};
use std::fs;

/// Whether `candidate` starts with what the user typed.
///
/// `oslo.completion.case_sensitive` decides. It defaults to off, so typing `RE` offers `README.md`
/// — which is what a shell that completes filenames on a case-insensitive muscle memory should do.
/// The setting was read from the config and then ignored, so turning it on did nothing at all.
///
/// Case folding is per character rather than by lowercasing the whole string: allocating a String
/// per candidate would mean thousands of allocations per keystroke on a large `$PATH`.
///
/// The flag is a parameter rather than read from the process-global settings here, so the tests
/// can exercise both answers without racing each other — the settings are shared, and the test
/// binary is multi-threaded.
/// Retarget a word at the item being typed inside an open `{a,b}` list.
///
/// `rm /dir/{alpha,be` completes `be` against `/dir/`, so the stem becomes `/dir/be` and `start`
/// moves to the `be`. Without this the whole word is one path with a literal brace in it, which
/// matches nothing and left completion silent from the `{` onwards.
fn brace_segment(word: Word<'_>) -> Word<'_> {
    let Some(open) = unclosed_brace(word.text) else {
        return word;
    };
    let after = &word.text[open + 1..];
    let item = after.rfind(',').map_or(0, |c| c + 1);
    let head = &word.text[..open];
    let start = word.start + open + 1 + item;
    Word {
        start,
        text: &after[item..],
        stem: format!("{}{}", unquote(head), unquote(&after[item..])),
        ..word
    }
}

/// The offset of a `{` that is still open at the end of `text`, ignoring quoted ones.
fn unclosed_brace(text: &str) -> Option<usize> {
    let mut open: Vec<usize> = Vec::new();
    let mut quote = None;
    for (i, c) in text.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '{') => open.push(i),
            (None, '}') => {
                open.pop();
            }
            _ => {}
        }
    }
    open.pop()
}

fn matches_prefix(candidate: &str, typed: &str, case_sensitive: bool) -> bool {
    // The pass currently being tried, when the caller is walking the chain. Set around one
    // builder's run rather than threaded through every call site: the builders are recursive and
    // the alternative is an extra argument on a dozen functions that only two of them care about.
    if let Some(matcher) = MATCHER.with(|slot| slot.get()) {
        return matcher.matches(candidate, typed);
    }
    if case_sensitive {
        return candidate.starts_with(typed);
    }
    matches_ignoring_case(candidate, typed)
}

thread_local! {
    static MATCHER: std::cell::Cell<Option<Match>> = const { std::cell::Cell::new(None) };
}

/// Put the best fuzzy matches first.
///
/// A prefix pass needs no such thing: everything it returns matched the same way, so the ordinary
/// frecency order is the right one. A fuzzy pass is different — `gco` matching `git checkout` and
/// `gcc --output` are not equally good answers, and the difference is exactly what
/// [`fuzzy_score`] measures. Stable, so candidates that score the same keep the order the source
/// gave them, which is frecency.
fn rank_by_fuzz(out: &mut [CompletionCandidate], typed: &str, fuzzy: Fuzzy) {
    // Folded once, outside the sort. `sort_by_key` calls this O(n log n) times, and the previous
    // version rebuilt the typed pattern into a `Vec<char>` on every one of them.
    let pattern = Fuzzed::new(typed, fuzzy);
    out.sort_by_key(|candidate| {
        std::cmp::Reverse(pattern.score(&candidate.display).unwrap_or(i32::MIN))
    });
}

/// A hook a config installs to complete one command itself.
///
/// Handed the words typed so far and the word being completed; answers the candidates, or `None`
/// to fall through to oslo's own. Registered per command name by `oslo.completion.for_command`.
pub type CommandCompleter =
    std::rc::Rc<dyn Fn(&str, &[&str], &str) -> Option<Vec<(String, Option<String>)>>>;

thread_local! {
    /// Thread-local rather than global: the hook calls Lua, which is not `Send`, and only the
    /// editor's thread completes anything.
    static FOR_COMMAND: std::cell::RefCell<Option<CommandCompleter>> =
        const { std::cell::RefCell::new(None) };
}

/// Install the hook `oslo.completion.for_command` describes. `None` removes it.
pub fn set_command_completer(hook: Option<CommandCompleter>) {
    FOR_COMMAND.with(|slot| *slot.borrow_mut() = hook);
}

/// Ask the config to complete this command, if it wants to.
fn config_candidates(
    command: &str,
    prior: &[&str],
    current: &str,
) -> Option<Vec<CompletionCandidate>> {
    // Cloned out of the cell before calling: the hook runs Lua, and Lua can complete another word,
    // which would come back through here and panic on the outstanding borrow.
    let hook = FOR_COMMAND.with(|slot| slot.borrow().clone())?;
    let answers = hook(command, prior, current)?;
    Some(
        answers
            .into_iter()
            .map(|(display, description)| CompletionCandidate {
                replacement: display.clone(),
                display,
                description,
                // **No kind, rather than a wrong one.** This was hardcoded to `option`, so a
                // config completing branches, files or hosts had all of them labelled options —
                // in the one column the user reads to tell them apart. The hook does not report a
                // kind, so the honest answer is that none is known; widening it to carry one is a
                // change to the Lua signature, not a change here.
                kind: None,
                path: None,
                detail: None,
            })
            .collect(),
    )
}

impl OsloHelper {
    /// Return context-quoted candidates and their replacement byte offset.
    pub fn candidates(&self, line: &str, pos: usize) -> (usize, Vec<CompletionCandidate>) {
        let word = brace_segment(current_word(line, pos));
        let mut out = Vec::new();

        if let Some(prefix) = word.stem.strip_prefix('$') {
            self.variable_candidates(prefix, word.quote, &mut out);
        } else if word.command_position {
            // **Only in shell.** A command name is a shell answer: a builtin, an alias, a function,
            // something on `$PATH`. At a Lua prompt none of them can run, and offering them is the
            // same mistake the ghost suggestion used to make — `l` answered with `ls` in a language
            // that has no `ls`.
            //
            // Paths, variables and config-supplied candidates are left alone: those are still
            // meaningful in Lua, and this is the one source that is not.
            if super::prompt::language().is_none_or(|language| language == "sh") {
                // Each way of matching in turn, stopping at the first that finds anything. An
                // exact match must never arrive diluted with looser ones.
                for matcher in matchers(settings::current().completion.fuzzy) {
                    self.command_candidates_with(&word, matcher, &mut out);
                    if !out.is_empty() {
                        // A fuzzy pass has no useful order of its own — every candidate that
                        // matched at all matched, so without this the list arrives in whatever
                        // order `$PATH` happened to be walked in.
                        if let Match::Fuzzy(fuzzy) = matcher {
                            rank_by_fuzz(&mut out, word.stem.as_str(), fuzzy);
                        }
                        break;
                    }
                    if self.case_sensitive() {
                        // The config asked for exactness; the looser passes are not wanted.
                        break;
                    }
                }
            }
        } else if let Some(from_config) = word
            .prior_words
            .first()
            .map(|w| unquote(w))
            .and_then(|cmd| config_candidates(&cmd, &word.prior_words, word.stem.as_str()))
        {
            // A config that answers for this command replaces oslo's own candidates rather than
            // adding to them: `oslo.completion.for_command("git", …)` means the config knows git
            // better than the built-in spec does, and mixing the two would offer both.
            out = from_config;
        } else {
            self.spec_candidates(&word, &mut out);
            if out.is_empty() {
                self.path_candidates(&word, &mut out);
            }
        }

        // Frecency first, name second. Without the first key this is alphabetical, which is how
        // `exit` came to suggest `exitsnoop-bpfcc`.
        // `oslo.completion.sources`: drop the kinds the config did not ask for. Applied after the
        // builders rather than inside them, so a kind is filtered by the name it already carries
        // and adding a new kind needs no change here.
        if let Some(wanted) = &crate::settings::current().completion.sources {
            out.retain(|c| {
                c.kind
                    .as_deref()
                    .is_some_and(|k| wanted.iter().any(|w| w == k))
            });
        }

        // `oslo.completion.sort`. Frecency first, name second — without the first key this is
        // alphabetical, which is how `exit` came to suggest `exitsnoop-bpfcc`. A config that
        // prefers a predictable order can ask for `alpha` and get name only.
        let by_name = crate::settings::current().completion.sort == crate::settings::Sort::Alpha;
        out.sort_by(|a, b| {
            if by_name {
                return a.display.cmp(&b.display);
            }
            let sa = self.frecency.score(&a.display);
            let sb = self.frecency.score(&b.display);
            sb.partial_cmp(&sa)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.display.cmp(&b.display))
        });
        out.dedup_by(|a, b| a.replacement == b.replacement);

        (word.start, out)
    }

    /// `oslo.completion.case_sensitive`, read once per completion rather than per candidate.
    fn case_sensitive(&self) -> bool {
        crate::settings::current().completion.case_sensitive
    }

    fn variable_candidates(&self, prefix: &str, quote: Quote, out: &mut Vec<CompletionCandidate>) {
        let env = self.env.lock().unwrap();
        for name in env.vars().keys() {
            if matches_prefix(name, prefix, self.case_sensitive()) {
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
                if matches_prefix(&b, stem, self.case_sensitive()) {
                    names.push((b.to_string(), "builtin", None));
                }
            }
            for (name, target) in env.aliases() {
                if matches_prefix(name, stem, self.case_sensitive()) {
                    // What it expands to travels with it: that is the one thing about an alias
                    // its name does not tell you, and it is why aliases keep a second column
                    // where a directory does not.
                    names.push((name.clone(), "alias", Some(target.clone())));
                }
            }
            for f in env.functions().keys() {
                if matches_prefix(f, stem, self.case_sensitive()) {
                    names.push((f.clone(), "function", None));
                }
            }
            (env.var("PATH").unwrap_or_default().to_string(), names)
        };

        for (name, kind, detail) in shell_names {
            let mut candidate = self.command_candidate(word, name, kind);
            candidate.detail = detail;
            out.push(candidate);
        }
        // The index is shared, not rebuilt: this used to `read_dir` all of `$PATH` per keystroke.
        for name in CommandIndex::executables(&path).iter() {
            if matches_prefix(name, stem, self.case_sensitive()) {
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

    /// The command a name really stands for, following aliases.
    ///
    /// Transitive, because aliases chain — `alias g=git`, `alias gc='git commit'` — and bounded,
    /// because they can also loop: `alias a=b` with `alias b=a` is legal to write and must not
    /// hang the shell on Tab.
    ///
    /// Only the *first word* of an expansion matters here: `alias gc='git commit'` completes as
    /// `git`, which is right for offering `git`'s options even if it means the subcommand path is
    /// not followed. Getting the head right is most of the value; the rest can come later.
    fn resolve_head(&self, name: &str) -> String {
        let Ok(env) = self.env.lock() else {
            return name.to_string();
        };
        let mut current = name.to_string();
        for _ in 0..16 {
            let Some(expansion) = env.alias(&current) else {
                break;
            };
            let Some(first) = expansion.split_whitespace().next() else {
                break;
            };
            if first == current {
                // `alias ls='ls --color'` — the classic self-reference. It expands to itself, so
                // the answer is already right and following it again would not terminate.
                break;
            }
            current = first.to_string();
        }
        current
    }

    /// Command candidates found one particular way. See [`Match`].
    fn command_candidates_with(
        &self,
        word: &Word<'_>,
        matcher: Match,
        out: &mut Vec<CompletionCandidate>,
    ) {
        MATCHER.with(|slot| slot.set(Some(matcher)));
        self.command_candidates(word, out);
        MATCHER.with(|slot| slot.set(None));
    }

    fn spec_candidates(&self, word: &Word<'_>, out: &mut Vec<CompletionCandidate>) {
        // `prior_words` holds this command's words only, so `ls | git comm<TAB>` looks up `git`
        // and not `ls`.
        let Some((primary, rest)) = word.prior_words.split_first() else {
            return;
        };
        // Through the alias table first. Everyone aliases `git`, and `g comm<TAB>` offering
        // nothing is a gap the shell has no excuse for: the alias table is already loaded and this
        // function is already holding the environment.
        let head = self.resolve_head(&unquote(primary));
        let Some(spec) = self.spec_registry.find_spec(&head) else {
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

#[cfg(test)]
mod case_tests {
    use super::matches_prefix;
    use super::{Match, matchers};

    /// Each way of matching, and the case each one exists for.
    #[test]
    fn a_matcher_answers_what_it_is_for() {
        assert!(Match::Exact.matches("README.md", "REA"));
        assert!(
            !Match::Exact.matches("README.md", "rea"),
            "exact means exact"
        );

        assert!(Match::Ignoring.matches("README.md", "rea"));
        assert!(!Match::Ignoring.matches("README.md", "xyz"));

        // The case everyone describes as "zsh just knew".
        assert!(Match::Pieces.matches("/usr/share/bin", "/u/s/b"));
        assert!(Match::Pieces.matches("foo-bar-baz", "f-b"));
        assert!(Match::Pieces.matches("some_long_name", "s_l_n"));
        assert!(!Match::Pieces.matches("/usr/share/bin", "/u/z/b"));
    }

    /// Piece matching only applies when a separator was typed. Without that guard it is a plain
    /// prefix test wearing another name, tried a second time for nothing.
    #[test]
    fn piece_matching_needs_a_separator() {
        assert!(
            !Match::Pieces.matches("foo-bar", "foo"),
            "no separator typed"
        );
        assert!(Match::Pieces.matches("foo-bar", "foo-b"));
    }

    /// More pieces than the candidate has cannot match, whatever they say.
    #[test]
    fn more_pieces_than_the_candidate_has_never_matches() {
        assert!(!Match::Pieces.matches("/usr/bin", "/u/s/b/x"));
    }

    /// Exactness first. A list mixing an exact hit with looser ones is worse than either alone.
    #[test]
    fn the_chain_tries_exactness_first() {
        let chain = matchers(super::Fuzzy::Off);
        assert_eq!(chain[0], Match::Exact);
        assert_eq!(chain[1], Match::Ignoring);
        assert_eq!(chain[2], Match::Pieces);
    }

    /// Fuzzy is last, and absent entirely when it is off.
    ///
    /// The order is the whole safety argument for turning it on: it runs only once every stricter
    /// pass has come back empty, so a candidate you actually prefixed can never be pushed down the
    /// list by one you merely scattered the letters of.
    #[test]
    fn fuzzy_is_the_last_resort_and_only_when_asked_for() {
        assert_eq!(matchers(super::Fuzzy::Off).len(), 3, "off adds no pass");
        let chain = matchers(super::Fuzzy::Smart);
        assert_eq!(chain.len(), 4);
        assert_eq!(*chain.last().unwrap(), Match::Fuzzy(super::Fuzzy::Smart));
    }

    /// The setting was read from the config and then ignored, so turning it on changed nothing.
    #[test]
    fn case_sensitivity_actually_decides_the_match() {
        assert!(matches_prefix("README.md", "RE", false));
        assert!(
            matches_prefix("README.md", "re", false),
            "off means insensitive"
        );
        assert!(!matches_prefix("README.md", "xy", false));

        assert!(matches_prefix("README.md", "RE", true));
        assert!(
            !matches_prefix("README.md", "re", true),
            "turning it on must matter"
        );
    }

    /// The typed text running past the candidate is not a prefix of it.
    #[test]
    fn a_longer_typed_word_is_not_a_prefix() {
        assert!(!matches_prefix("ls", "lsof", false));
        assert!(matches_prefix("lsof", "ls", false));
        // An empty prefix matches everything, which is what bare Tab does.
        assert!(matches_prefix("anything", "", false));
    }
}
