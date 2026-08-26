//! Reading a half-typed command line: which command, which position, which flag.
//!
//! ```text
//!   git -C /repo commit --message "wip" -- src/<Tab>
//!       └─ flag  └─ its  └─ sub   └─ flag └─ its    └─ dash position 0
//!          taking    value  command         value
//!          a value
//! ```
//!
//! Everything a declared position needs to know is in that sentence, and none of it can be had by
//! looking at the word under the cursor. The walk below is the only thing between the two.
//!
//! # Why the flags have to be parsed and not skipped
//!
//! The old walk skipped any word starting with `-` and counted nothing else. That is right until a
//! flag takes an argument: in `deploy --env staging <Tab>` the word `staging` belongs to `--env`,
//! and a walk that counts it as the first positional offers the answers meant for the second one —
//! silently, and only for the flags that take values.

use crate::spec::{Arg, CommandSpec, Nargs, OptionSpec, Parsing, flag};
use std::collections::{HashMap, HashSet};

/// Where the cursor is, in the terms a spec declares things in.
#[derive(Debug, Clone, Copy)]
pub enum At<'a> {
    /// The argument of a flag that takes one.
    FlagValue(&'a OptionSpec),
    /// Argument `n` of the resolved command, counting from zero.
    Positional(usize),
    /// Argument `n` after a bare `--`.
    Dash(usize),
}

/// What the line says, once it has been read.
pub struct Walk<'a> {
    /// The command the cursor is inside — the deepest subcommand that was named.
    pub node: &'a CommandSpec,
    /// Persistent flags of every command above `node`.
    pub inherited: Vec<&'a OptionSpec>,
    pub at: At<'a>,
    /// Positional arguments already typed. `${C_ARG0}`.
    pub args: Vec<String>,
    /// Flags that were given a value, by longhand in upper case. `${C_FLAG_MESSAGE}`.
    pub flags: HashMap<String, String>,
    /// Flags already on the line, by the name `flag::key` gives them.
    ///
    /// Kept for every flag, not only the ones that took a value: a switch leaves no other trace,
    /// and "has this been given already" is the question `repeatable` exists to answer.
    pub seen: HashSet<String>,
}

impl<'a> Walk<'a> {
    /// Every flag that could still be typed here.
    ///
    /// **Minus the ones already given.** `ls -l -<Tab>` re-offered `-l`, because `repeatable` was
    /// written by three readers and read by none. A flag that may be repeated stays; the rest have
    /// had their turn.
    pub fn flags_on_offer(&self) -> impl Iterator<Item = &'a OptionSpec> + '_ {
        self.node
            .options
            .iter()
            .chain(self.node.persistent.iter())
            .chain(self.inherited.iter().copied())
            .filter(|opt| !opt.hidden)
            .filter(move |opt| {
                opt.repeatable || !flag::key(&opt.names).is_some_and(|key| self.seen.contains(&key))
            })
    }
}

/// Read `words` — the command's arguments, its own name already removed — against `spec`.
pub fn walk<'a>(spec: &'a CommandSpec, words: &[String]) -> Walk<'a> {
    let mut node = spec;
    let mut inherited: Vec<&'a OptionSpec> = Vec::new();
    let mut args: Vec<String> = Vec::new();
    let mut flags: HashMap<String, String> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut dash: Option<usize> = None;
    // Set when the last word was a flag still waiting for its value, which is the one case where
    // the cursor is inside something rather than after it.
    let mut pending: Option<&OptionSpec> = None;
    // `non-interspersed`: the first positional ends flag parsing for good.
    let mut flags_over = false;

    let mut at = 0;
    while at < words.len() {
        let word = words[at].as_str();
        at += 1;

        if let Some(count) = dash.as_mut() {
            *count += 1;
            continue;
        }
        if word == "--" {
            dash = Some(0);
            pending = None;
            continue;
        }

        let parses_flags = node.parsing != Parsing::Disabled
            && !flags_over
            && word.starts_with('-')
            && word != "-";
        if parses_flags {
            pending = None;
            if let Some(opt) = find(node, &inherited, word) {
                mark(&mut seen, opt);
                let inline = opt.matches(word).flatten();
                if let Some(value) = inline {
                    remember(&mut flags, opt, value);
                    continue;
                }
                if opt.takes != Arg::Required {
                    // A switch, or an optional argument that has to be written `--flag=value`.
                    continue;
                }
                let taken = consume(words, at, opt.nargs);
                if taken > 0 {
                    remember(&mut flags, opt, &words[at..at + taken].join(" "));
                }
                // **A flag that could still take more is still the flag being typed.** `git branch
                // -d one <Tab>` is a second branch, not the command's first argument: the argument
                // is variadic and the line ran out. Answering `Positional(0)` there offers the
                // wrong list and throws off every position after it.
                let wants_more = match opt.nargs {
                    Nargs::One => taken == 0,
                    Nargs::Exactly(n) => taken < n,
                    Nargs::Any => at + taken == words.len(),
                };
                if wants_more {
                    pending = Some(opt);
                }
                at += taken;
                continue;
            }

            // **`-la` is `-l` and `-a`.** Short flags cluster, and 679 of the 720 shipped Fig specs
            // expect it — a walk that read the whole run as one unknown flag lost every one of
            // them. Only when *every* letter is a declared short flag, so an unrelated `-xyz` is
            // still the unknown flag it looks like.
            if let Some(bundle) = cluster(node, &inherited, word) {
                for opt in &bundle {
                    mark(&mut seen, opt);
                }
                // The last letter is the only one that can carry a value: `tar -xzf name`.
                if let Some(last) = bundle.last().filter(|o| o.takes == Arg::Required) {
                    let taken = consume(words, at, last.nargs);
                    if taken > 0 {
                        remember(&mut flags, last, &words[at..at + taken].join(" "));
                    } else {
                        pending = Some(last);
                    }
                    at += taken;
                }
                continue;
            }

            // **A dashed word can be a subcommand.** `nix-store --gc`, `cmake -E`: 505 of the
            // shipped specs name one, and reading every dashed word as a flag left all of them
            // unreachable. A *declared* flag still wins — that is the common case and the ambiguous
            // one — so this is only reached when neither a flag nor a cluster claimed the word.
            if args.is_empty()
                && let Some(found) = node.subcommands.iter().find(|sub| sub.answers_to(word))
            {
                inherited.extend(node.persistent.iter());
                node = found;
                continue;
            }
            // Otherwise a flag nothing declared. It consumes itself and nothing more — guessing
            // that it takes a value would swallow the next word, and the next word may be the
            // subcommand the whole rest of the walk depends on.
            continue;
        }

        pending = None;
        // A subcommand is only a subcommand before the first argument. `git add commit` is two
        // paths, not a command inside a command.
        if args.is_empty()
            && node.parsing != Parsing::Disabled
            && let Some(found) = node.subcommands.iter().find(|sub| sub.answers_to(word))
        {
            inherited.extend(node.persistent.iter());
            node = found;
            continue;
        }

        args.push(word.to_string());
        if node.parsing == Parsing::NonInterspersed {
            flags_over = true;
        }
    }

    let at = match (pending, dash) {
        (Some(opt), _) => At::FlagValue(opt),
        (None, Some(count)) => At::Dash(count),
        (None, None) => At::Positional(args.len()),
    };
    Walk {
        node,
        inherited,
        at,
        args,
        flags,
        seen,
    }
}

/// How many words this flag's argument is, given how many are left.
fn consume(words: &[String], from: usize, nargs: Nargs) -> usize {
    let left = words.len() - from;
    match nargs {
        Nargs::One => left.min(1),
        Nargs::Exactly(n) => left.min(n),
        // Everything up to the next flag. A `nargs: -1` flag at the end of the line is still
        // waiting, which is what makes `--files <Tab>` complete files rather than the positional.
        Nargs::Any => words[from..]
            .iter()
            .take_while(|word| !word.starts_with('-'))
            .count(),
    }
}

/// Write down what a flag was given, under the name `${C_FLAG_…}` uses.
///
/// The same name `completion.flag` keys on, through the same function: a spec declaring values for
/// `file` and a value reading `C_FLAG_FILE` are naming one flag, and two rules for spelling it
/// would be one rule too many.
fn remember(flags: &mut HashMap<String, String>, opt: &OptionSpec, value: &str) {
    if let Some(name) = flag::key(&opt.names) {
        flags.insert(name.to_ascii_uppercase(), value.to_string());
    }
}

/// The flag `word` names, here or anywhere above here.
fn find<'a>(
    node: &'a CommandSpec,
    inherited: &[&'a OptionSpec],
    word: &str,
) -> Option<&'a OptionSpec> {
    node.options
        .iter()
        .chain(node.persistent.iter())
        .chain(inherited.iter().copied())
        .find(|opt| opt.matches(word).is_some())
}

#[cfg(test)]
#[path = "walk/tests.rs"]
mod tests;

/// Comparing two positions is a thing only the tests do, and the flag one carries is compared by
/// identity: two flags with the same name in the same spec are the same flag.
#[cfg(test)]
impl PartialEq for At<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (At::FlagValue(one), At::FlagValue(two)) => std::ptr::eq(*one, *two),
            (At::Positional(one), At::Positional(two)) => one == two,
            (At::Dash(one), At::Dash(two)) => one == two,
            _ => false,
        }
    }
}

/// Write down that a flag has been given, under the name `flags_on_offer` filters by.
fn mark(seen: &mut HashSet<String>, opt: &OptionSpec) {
    if let Some(key) = flag::key(&opt.names) {
        seen.insert(key);
    }
}

/// The flags a clustered short run names, when every letter in it is one.
///
/// `-la` is `-l` and `-a`; `-xzf` is three. **All or nothing**: a run holding one letter nothing
/// declared is not a cluster but an unknown flag, and reading it as a cluster would invent flags
/// the command does not have. A `--long` word is never a cluster, and neither is a bare `-x`, which
/// `find` has already answered for.
pub(super) fn cluster<'a>(
    node: &'a CommandSpec,
    inherited: &[&'a OptionSpec],
    word: &str,
) -> Option<Vec<&'a OptionSpec>> {
    let letters = word
        .strip_prefix('-')
        .filter(|rest| !rest.starts_with('-'))?;
    if letters.chars().count() < 2 {
        return None;
    }
    letters
        .chars()
        .map(|letter| {
            let short = format!("-{letter}");
            find(node, inherited, &short)
                // A long-only flag cannot be part of a cluster even when its name is one letter
                // after the dashes: `--l` is not `-l`.
                .filter(|opt| opt.names.iter().any(|name| name == &short))
        })
        .collect()
}
