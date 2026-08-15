//! Builds completion candidates for Tab.

mod paths;
pub(crate) use paths::executable;
pub(crate) use paths::glob_matches_anything;
pub(crate) use paths::takes_only_directories;
pub mod provider;
mod segment;
mod spec;
mod sugar;

use segment::{after_break, brace_segment};

use super::OsloHelper;
use super::command_index::CommandIndex;
use super::dropdown::CompletionCandidate;
use super::matching::{Fuzzed, Match, matchers, matches_ignoring_case};
use super::settings;
use super::words::{Quote, Word, current_word, quote_replacement, unquote};

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

/// Whether the variable being completed was written `${NAME` rather than `$NAME`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Braced {
    Yes,
    No,
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
        // oslo's own shorthands first: both look like ordinary words and neither completes like
        // one, so the retarget has to happen before anything reads `stem`.
        let sugared = sugar::at_segment(sugar::equals_segment(sugar::escaped_segment(
            current_word(line, pos),
        )));
        let word = brace_segment(after_break(sugared));
        let mut out = Vec::new();
        // Set when a fuzzy pass answered, and read by the final sort — see there.
        let mut fuzzed: Option<Fuzzed> = None;

        if let Some(prefix) = word.stem.strip_prefix('$') {
            // `${NAME` is the same question with a brace on it, and the answer has to carry the
            // closing one — `${HO` completed to `$HOME` would leave an expansion that never ends.
            match prefix.strip_prefix('{') {
                Some(braced) => self.variable_candidates(braced, Braced::Yes, word.quote, &mut out),
                None => self.variable_candidates(prefix, Braced::No, word.quote, &mut out),
            }
        } else if word.stem.starts_with('@')
            && word.quote == Quote::None
            && !word.stem.contains('/')
            && !word.command_position
        {
            // A name still being typed. `@work/` has already been retargeted at the directory it
            // stands for, so only the bare name reaches here.
            //
            // **Not at the start of a line**, where `@name` does not expand either: that position is
            // reserved, and offering completions for it would promise something the shell will not
            // then do.
            //
            // **And not inside quotes**, for the same reason and a worse symptom. A quoted `@name`
            // is a literal to the expander, so completing it promises an expansion that will not
            // happen — and `named_dir_candidates` writes back as if the word were unquoted, so
            // `ls "@pr<Tab>` came back as `ls @proj/` with the opening quote deleted. Every sibling
            // builder in `sugar.rs` declines a quoted word; this branch was the one that did not.
            self.named_dir_candidates(word.stem.as_str(), &mut out);
            if out.is_empty() {
                self.path_candidates(&word, &mut out);
            }
        } else if word.command_position && word.stem.contains('/') {
            // **A command with a `/` in it is a path, not a name.** Nothing on `$PATH` is reached by
            // one, so the name builders below can never answer — and until this branch existed
            // `./bui<Tab>`, `/usr/bin/gre<Tab>` and `~/bin/my<Tab>` did nothing at all, while the
            // highlighter happily coloured the same word as a real command once it was typed out.
            self.path_candidates(&word, &mut out);
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
                        // order `$PATH` happened to be walked in. The pattern is carried to the
                        // final sort rather than applied here: sorting twice means the second
                        // sort wins, and it did.
                        if let Match::Fuzzy(fuzzy) = matcher {
                            fuzzed = Some(Fuzzed::new(word.stem.as_str(), fuzzy));
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

        // What a provider offers, merged in rather than replacing anything. Collected before the
        // filter and the sort below so a provider's candidates are filtered by kind and ranked
        // against oslo's own — which is the difference between adding to a menu and owning it.
        let boosts = self.provider_candidates(&word, &mut out);

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
        //
        // **How well it matched outranks both**, when a fuzzy pass is what found these. Every
        // candidate in a fuzzy set matched, so frecency cannot separate them — unrun commands all
        // score zero and the list came back in plain alphabetical order, which is how `cnr` offered
        // `airscan-discover` above `cargo-nextest-runner` and why `gco` never reached
        // `git checkout` first. Frecency still orders candidates that matched equally well.
        let by_name = crate::settings::current().completion.sort == crate::settings::Sort::Alpha;
        out.sort_by(|a, b| {
            if by_name {
                return a.display.cmp(&b.display);
            }
            if let Some(pattern) = &fuzzed {
                let fa = pattern.score(&a.display).unwrap_or(i32::MIN);
                let fb = pattern.score(&b.display).unwrap_or(i32::MIN);
                if fa != fb {
                    return fb.cmp(&fa);
                }
            }
            let sa =
                self.frecency.score(&a.display) + boosts.get(&a.display).copied().unwrap_or(0.0);
            let sb =
                self.frecency.score(&b.display) + boosts.get(&b.display).copied().unwrap_or(0.0);
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

    fn variable_candidates(
        &self,
        prefix: &str,
        braced: Braced,
        quote: Quote,
        out: &mut Vec<CompletionCandidate>,
    ) {
        let env = self.env.lock().unwrap();
        for name in env.vars().keys() {
            if matches_prefix(name, prefix, self.case_sensitive()) {
                let value = match braced {
                    Braced::Yes => format!("${{{name}}}"),
                    Braced::No => format!("${name}"),
                };
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
        // The structured verbs and the tools a config registered. Not on `$PATH` and not builtins,
        // so nothing else here can offer them — `whe<Tab>` found nothing at all.
        for (name, kind) in oslo_base::vocab::all() {
            if matches_prefix(&name, stem, self.case_sensitive()) {
                out.push(self.command_candidate(word, name, kind));
            }
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

    /// Merge in what the registered providers offer, answering each one's score offset by display.
    ///
    /// **Before the kind filter and the sort**, so a provider's candidates are filtered by the kind
    /// it declared and ranked against oslo's own rather than being stapled to one end of the list.
    fn provider_candidates(
        &self,
        word: &Word<'_>,
        out: &mut Vec<CompletionCandidate>,
    ) -> std::collections::HashMap<String, f64> {
        let mut boosts = std::collections::HashMap::new();
        if !provider::any() {
            return boosts;
        }
        let Some(primary) = word.prior_words.first() else {
            return boosts;
        };
        let command = self.resolve_head(&unquote(primary));
        let ctx = provider::Ctx {
            command,
            words: word.prior_words.iter().map(|w| unquote(w)).collect(),
            current: word.stem.as_str().to_string(),
            arg: word.prior_words.len().saturating_sub(1).max(1),
            cwd: std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        };
        for (candidate, offset) in provider::offers(&ctx) {
            // Only what continues the word being typed. A menu that offered everything regardless
            // of what you have already typed would be a list, not a completion.
            if !matches_prefix(
                &candidate.display,
                ctx.current.as_str(),
                self.case_sensitive(),
            ) {
                continue;
            }
            if offset != 0.0 {
                boosts.insert(candidate.display.clone(), offset);
            }
            out.push(candidate);
        }
        boosts
    }
}

#[cfg(test)]
mod case_tests {
    use super::matches_prefix;
    use super::{Match, matchers};
    use crate::matching::Fuzzy;

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
        let chain = matchers(Fuzzy::Off);
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
        assert_eq!(matchers(Fuzzy::Off).len(), 3, "off adds no pass");
        let chain = matchers(Fuzzy::Smart);
        assert_eq!(chain.len(), 4);
        assert_eq!(*chain.last().unwrap(), Match::Fuzzy(Fuzzy::Smart));
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
