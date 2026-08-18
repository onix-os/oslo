//! The stack of streams a coordinate reads from, and substituting one into a command's words.
//!
//! A *stream* is text something produced: a pipeline stage that has finished, or a whole command at
//! the prompt. Both go on the same stack, so stepping back through a pipeline and stepping back
//! through the session are the same motion — see [`oslo_base::coords`] for the coordinate itself.
//!
//! # Which way the index goes
//!
//! ```text
//! cat hosts.txt | grep web | ssh {0:0}
//!                      │           └─ 0  this stage's input: what `grep web` printed
//!                      └───────────── 1  one stage further back: what `cat` printed
//!
//! ssh {-1:0:0}   ← -1  the previous prompt, whatever it was
//! ```
//!
//! **Zero and up walk back through this pipeline; below zero walks back through the session.** They
//! are different collections and giving them one axis would mean `{3:…}` silently crossing from one
//! into the other when a pipeline happened to be short. The sign says which you meant, and it reads
//! the way negative indices already read on a line: from the other end.
//!
//! # A value is one argument
//!
//! Substitution happens on the **syntax tree**, before any expansion runs, and every value becomes
//! a single-quoted word part. Single quotes are literal throughout in every shell there is, so a
//! line holding a space, a `*` or a `$` arrives at the command whole and is never field-split or
//! re-globbed. Reusing the quoting the shell already has beats inventing a new origin and teaching
//! six expansions about it — and a shell that field-splits its own substitutions is a shell that
//! executes filenames.

use oslo_base::ast::{
    AssignmentValue, CaseItem, Command, CommandList, CompoundCommand, Redirection, Word, WordPart,
};
use oslo_base::coords::{self, Coord};

/// How many prompts back a coordinate may reach.
///
/// Ten is the number of things anybody keeps in their head; past that you look at the screen.
pub const PROMPTS_KEPT: usize = 10;

/// The most of one stream that is kept, in bytes.
///
/// Shared with `keep`/`copy --last` deliberately — two limits on "how much output do we hold"
/// would be two numbers to reason about and one of them would be wrong.
pub use oslo_base::capture::MAX as STREAM_MAX;

thread_local! {
    /// The command lines of previous prompts, newest first.
    ///
    /// **Lines, not output**, and the difference is worth being plain about. A pipeline stage's
    /// output can be captured for nothing, because a stage already writes to a pipe. A *command's*
    /// output goes to the terminal, and standing between the two turns `isatty` false for
    /// everything — see `capture.rs`, where that argument is made at length. What a previous prompt
    /// does have, free and exactly, is the line that was typed:
    ///
    /// ```text
    /// $ cat one.txt two.txt
    /// $ wc -l {-1:0:1}          → wc -l one.txt
    ///         └─ previous prompt, its only line, word 1
    /// ```
    ///
    /// So `{-n:…}` addresses the command *line* — one line, its words being the command and its
    /// arguments. `{-1:0:-1}` is the last argument, which is `!$` written in this grammar and
    /// usable where `!$` is not: inside a script, inside a function, inside quotes.
    static PROMPTS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Remember the line a prompt ran, for `{-n:…}` to address.
pub fn remember_prompt(line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    PROMPTS.with(|slot| {
        let mut lines = slot.borrow_mut();
        lines.insert(0, line.to_string());
        lines.truncate(PROMPTS_KEPT);
    });
}

/// Forget every remembered line. `history -c` clears this too — a line the user asked to be gone
/// must not stay reachable by coordinate.
pub fn forget_prompts() {
    PROMPTS.with(|slot| slot.borrow_mut().clear());
}

/// The streams a coordinate can reach.
#[derive(Debug, Default, Clone)]
pub struct Streams {
    /// This pipeline's finished stages, oldest first. Index 0 of a coordinate is the *last* of
    /// these — the stage feeding the command being built.
    stages: Vec<String>,
    /// Previous prompts, newest first, so `-1` is `prompts[0]`.
    prompts: Vec<String>,
}

impl Streams {
    /// A stack whose negative side is the session's remembered prompt lines.
    ///
    /// Built per pipeline rather than kept: the stages are this pipeline's and the prompts are the
    /// session's, and copying a handful of short lines is cheaper than reasoning about a shared
    /// mutable stack across a fork.
    pub fn for_this_pipeline() -> Streams {
        Streams {
            stages: Vec::new(),
            prompts: PROMPTS.with(|slot| slot.borrow().clone()),
        }
    }
}

impl Streams {
    /// Note what a pipeline stage printed.
    pub fn push_stage(&mut self, text: impl Into<String>) {
        self.stages.push(cap(text.into()));
    }

    /// Note what a whole command printed, and start a fresh pipeline.
    ///
    /// The stages are cleared because they belonged to the pipeline that just ended: a coordinate
    /// in the *next* command counting forward from zero would otherwise reach into a pipeline that
    /// is over, which is a different stream than the one it names.
    pub fn push_prompt(&mut self, text: impl Into<String>) {
        self.stages.clear();
        self.prompts.insert(0, cap(text.into()));
        self.prompts.truncate(PROMPTS_KEPT);
    }

    /// Start a new pipeline without recording anything — a command that produced nothing worth
    /// keeping, or one whose output was never captured.
    pub fn end_pipeline(&mut self) {
        self.stages.clear();
    }

    /// The text a coordinate's stream dimension names, if there is one.
    ///
    /// `None` where nothing was captured, which reads as an empty selection rather than an error.
    pub fn text(&self, coord: &Coord) -> Option<&str> {
        let at = match coord.stream {
            coords::Sel::At(at) => at,
            // A range of *streams* is not meaningful — `{0..2:0:0}` would mean "the same line of
            // three different commands", which is a question nobody asks and a syntax nobody would
            // reach for by accident. The first is taken, so the coordinate still reads.
            coords::Sel::Span { from, .. } => from.unwrap_or(0),
        };
        match at >= 0 {
            // Counting back from the newest stage: 0 is the one that just finished.
            true => {
                let back = at as usize;
                self.stages
                    .len()
                    .checked_sub(back + 1)
                    .map(|i| &self.stages[i][..])
            }
            // Previous prompts, newest first.
            false => self.prompts.get((-at - 1) as usize).map(String::as_str),
        }
    }
}

/// Keep the head, not the tail: a coordinate counts from the start, and `{-1}` on a truncated
/// stream is honestly the last line *of what was kept*.
fn cap(mut text: String) -> String {
    if text.len() > STREAM_MAX {
        text.truncate(STREAM_MAX);
    }
    text
}

/// Whether a word contains anything a coordinate could claim.
///
/// A cheap scan, because it runs on every word of every command. It only has to be right about
/// "there is a `{` with a digit, `-`, `*` or `:` after it" — [`substitute`] is what decides.
pub fn looks_like_a_coordinate(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.iter().enumerate().any(|(i, b)| {
        *b == b'{'
            && bytes
                .get(i + 1)
                .is_some_and(|n| n.is_ascii_digit() || matches!(n, b'-' | b'*' | b':' | b'.'))
    })
}

/// Replace every coordinate in one piece of literal text, answering the words it becomes.
///
/// One word can become several: `{*:0}` on three lines is three arguments, the way `"$@"` is. That
/// only happens for a word that is a coordinate and **nothing else** — `{0:0}-{1:0}` has to stay one
/// word to mean anything, so there the values are joined with a space and the text between them is
/// kept.
///
/// `None` when the text holds no coordinate, so an ordinary brace group falls through to the brace
/// expansion that already handles it.
pub fn substitute(text: &str, streams: &Streams) -> Option<Vec<Word>> {
    // A word that is one coordinate and nothing else: one argument per value.
    if let Some((before, coord, after)) = split(text)
        && before.is_empty()
        && after.is_empty()
    {
        return Some(values(&coord, streams).into_iter().map(quoted).collect());
    }

    // Otherwise every coordinate is replaced where it stands, and the word stays one word.
    let mut parts = Vec::new();
    let mut rest = text;
    let mut found = false;
    while let Some((before, coord, after)) = split(rest) {
        found = true;
        if !before.is_empty() {
            parts.push(WordPart::Literal(before.to_string()));
        }
        parts.push(WordPart::SingleQuoted(values(&coord, streams).join(" ")));
        rest = after;
    }
    if !found {
        return None;
    }
    if !rest.is_empty() {
        parts.push(WordPart::Literal(rest.to_string()));
    }
    Some(vec![Word { parts }])
}

/// What a coordinate reads, or nothing when its stream was never captured.
fn values(coord: &Coord, streams: &Streams) -> Vec<String> {
    match streams.text(coord) {
        Some(stream) => coords::select(coord, stream),
        // Nothing captured: the coordinate reads empty rather than refusing to run, which is the
        // rule everywhere else in this feature.
        None => Vec::new(),
    }
}

/// One value as one word that cannot be split or globbed.
fn quoted(value: String) -> Word {
    Word {
        parts: vec![WordPart::SingleQuoted(value)],
    }
}

/// Split a word around its first coordinate: `(before, coord, after)`.
///
/// Scans forward for a `{` that opens one, so `a{b}c{0:0}` finds the coordinate rather than giving
/// up at the brace group in front of it.
fn split(text: &str) -> Option<(&str, Coord, &str)> {
    let mut from = 0;
    while let Some(open) = text[from..].find('{') {
        let open = from + open;
        let close = open + text[open..].find('}')?;
        if let Some(coord) = coords::parse(&text[open + 1..close]) {
            return Some((&text[..open], coord, &text[close + 1..]));
        }
        from = open + 1;
    }
    None
}

/// Whether this text really holds a coordinate, parsed rather than guessed.
///
/// [`looks_like_a_coordinate`] is the cheap scan used to decide whether to bother; this is the
/// answer. `{0..2}` passes the scan and fails here, because it is brace expansion.
pub fn holds_a_coordinate(text: &str) -> bool {
    split(text).is_some()
}

/// Rewrite every coordinate a command can hold.
///
/// **Everywhere a word can appear, not only the argument list.** A coordinate in a redirection
/// target, an assignment's value or the body of a loop used to be left as text and then read as
/// nothing — `cat f | cat > {0:0}` wrote to a file called `{0:0}`, and `cat f | (echo {0:0})`
/// printed a blank line. Both failed *silently*, which is the worst way for a substitution to fail:
/// the command runs, and does the wrong thing.
///
/// **A function definition is not rewritten.** `f(){ echo {0:0}; }` defines a function whose body
/// is run later, when this stream is gone — baking today's text into it would make the definition
/// mean something different from what was written.
pub fn rewrite_command(command: &mut Command, streams: &Streams) {
    match command {
        Command::Simple(simple) => {
            rewrite(&mut simple.words, streams);
            for assignment in &mut simple.assignments {
                match &mut assignment.value {
                    AssignmentValue::Scalar(word) => rewrite_word(word, streams),
                    AssignmentValue::Array(elements) => {
                        for element in elements {
                            if let Some(index) = &mut element.index {
                                rewrite_word(index, streams);
                            }
                            rewrite_word(&mut element.value, streams);
                        }
                    }
                }
            }
            rewrite_redirections(&mut simple.redirections, streams);
        }
        Command::Compound { kind, redirections } => {
            rewrite_compound(kind, streams);
            rewrite_redirections(redirections, streams);
        }
        // Deliberately untouched — see the note above.
        Command::FunctionDef { .. } => {}
    }
}

fn rewrite_redirections(redirections: &mut [Redirection], streams: &Streams) {
    for redirection in redirections {
        rewrite_word(&mut redirection.target, streams);
        if let Some(body) = &mut redirection.heredoc_content {
            rewrite_word(body, streams);
        }
    }
}

fn rewrite_compound(kind: &mut CompoundCommand, streams: &Streams) {
    match kind {
        CompoundCommand::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => {
            rewrite_list(condition, streams);
            rewrite_list(then_branch, streams);
            for (condition, body) in elif_branches {
                rewrite_list(condition, streams);
                rewrite_list(body, streams);
            }
            if let Some(body) = else_branch {
                rewrite_list(body, streams);
            }
        }
        CompoundCommand::While { condition, body } | CompoundCommand::Until { condition, body } => {
            rewrite_list(condition, streams);
            rewrite_list(body, streams);
        }
        CompoundCommand::For { items, body, .. } => {
            if let Some(items) = items {
                let mut words = std::mem::take(items);
                rewrite(&mut words, streams);
                *items = words;
            }
            rewrite_list(body, streams);
        }
        CompoundCommand::Case { word, items } => {
            rewrite_word(word, streams);
            for item in items.iter_mut() {
                rewrite_case_item(item, streams);
            }
        }
        CompoundCommand::ArithmeticFor { body, .. } => rewrite_list(body, streams),
        CompoundCommand::Subshell(list) | CompoundCommand::Group(list) => {
            rewrite_list(list, streams)
        }
        // Arithmetic is a string the arithmetic parser owns, not a word list.
        CompoundCommand::Arithmetic(_) => {}
    }
}

fn rewrite_case_item(item: &mut CaseItem, streams: &Streams) {
    let mut patterns = std::mem::take(&mut item.patterns);
    rewrite(&mut patterns, streams);
    item.patterns = patterns;
    rewrite_list(&mut item.body, streams);
}

fn rewrite_list(list: &mut CommandList, streams: &Streams) {
    for item in &mut list.items {
        rewrite_pipeline(&mut item.and_or.first, streams);
        for (_, pipeline) in &mut item.and_or.rest {
            rewrite_pipeline(pipeline, streams);
        }
    }
}

/// A nested pipeline is rewritten only when it has no stream of its own.
///
/// **Where the recursion has to stop.** `cat f | (echo {0:0})` names the outer pipe, because the
/// subshell has nothing feeding it — so it is rewritten from here. But
/// `for i in 1 2; do printf "$i\n" | echo {0:0}; done` does not: that inner pipeline has its own
/// upstream, and its coordinate means *that* one. Rewriting it from out here answered with the
/// enclosing stream, which for a loop at the top of a pipeline is nothing at all — the loop printed
/// three blanks.
///
/// So a nested pipeline of two or more stages is left alone. It reaches `run_stages` in its own
/// right and asks the same question there.
fn rewrite_pipeline(pipeline: &mut oslo_base::ast::Pipeline, streams: &Streams) {
    if pipeline.commands.len() > 1 {
        return;
    }
    for command in &mut pipeline.commands {
        rewrite_command(command, streams);
    }
}

/// Rewrite one word in place, when it is a lone literal holding a coordinate.
///
/// A word that becomes *several* cannot be put back in a slot that holds one, so a redirection
/// target or an assignment value takes the values joined — which is what a single slot can mean.
fn rewrite_word(word: &mut Word, streams: &Streams) {
    let Some(text) = only_literal(word) else {
        return;
    };
    let Some(mut replacements) = substitute(text, streams) else {
        return;
    };
    *word = match replacements.len() {
        1 => replacements.remove(0),
        _ => Word {
            parts: vec![WordPart::SingleQuoted(
                replacements
                    .iter()
                    .map(|word| {
                        word.parts
                            .iter()
                            .map(|part| match part {
                                WordPart::Literal(text) | WordPart::SingleQuoted(text) => {
                                    text.as_str()
                                }
                                _ => "",
                            })
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            )],
        },
    };
}

/// Rewrite every coordinate in a simple command's words, in place.
///
/// **Only `Literal` parts are looked at.** A coordinate inside single or double quotes is text the
/// user quoted on purpose, and `echo "{0:1}"` printing a literal `{0:1}` is the same promise every
/// other expansion keeps — there is always a way to write the characters themselves.
///
/// Answers whether anything changed, so a caller can tell a command that needed the stack from one
/// that merely looked like it might.
pub fn rewrite(words: &mut Vec<Word>, streams: &Streams) -> bool {
    let mut out = Vec::with_capacity(words.len());
    let mut changed = false;
    for word in words.drain(..) {
        let Some(text) = only_literal(&word) else {
            out.push(word);
            continue;
        };
        match substitute(text, streams) {
            Some(replacements) => {
                changed = true;
                out.extend(replacements);
            }
            None => out.push(word),
        }
    }
    *words = out;
    changed
}

/// The text of a word that is one unquoted literal, which is the only shape a coordinate can be
/// written in.
fn only_literal(word: &Word) -> Option<&str> {
    match word.parts.as_slice() {
        [WordPart::Literal(text)] => Some(text.as_str()),
        _ => None,
    }
}

/// Whether a command could carry a coordinate anywhere.
///
/// The gate for the whole feature: a pipeline that answers `false` runs down the path it always
/// did, forked concurrently, with nothing captured and nothing to pay for.
///
/// **It looks everywhere [`rewrite_command`] writes.** A gate that read only the argument list
/// while the rewriter also handled redirections would leave `cat f | cat > {0:0}` on the concurrent
/// path, where the rewriter never runs — the substitution would not happen, and would not say so.
/// The two walks mirror each other and a test holds them together.
pub fn command_uses_coordinates(command: &Command) -> bool {
    match command {
        Command::Simple(simple) => {
            any_word(&simple.words)
                || simple
                    .assignments
                    .iter()
                    .any(|assignment| match &assignment.value {
                        AssignmentValue::Scalar(word) => is_one(word),
                        AssignmentValue::Array(elements) => elements.iter().any(|element| {
                            element.index.as_ref().is_some_and(is_one) || is_one(&element.value)
                        }),
                    })
                || any_redirection(&simple.redirections)
        }
        Command::Compound { kind, redirections } => {
            any_compound(kind) || any_redirection(redirections)
        }
        // Not rewritten, so not a reason to leave the concurrent path.
        Command::FunctionDef { .. } => false,
    }
}

fn is_one(word: &Word) -> bool {
    only_literal(word).is_some_and(looks_like_a_coordinate)
}

fn any_word(words: &[Word]) -> bool {
    words.iter().any(is_one)
}

fn any_redirection(redirections: &[Redirection]) -> bool {
    redirections.iter().any(|redirection| {
        is_one(&redirection.target) || redirection.heredoc_content.as_ref().is_some_and(is_one)
    })
}

fn any_compound(kind: &CompoundCommand) -> bool {
    match kind {
        CompoundCommand::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => {
            any_list(condition)
                || any_list(then_branch)
                || elif_branches
                    .iter()
                    .any(|(condition, body)| any_list(condition) || any_list(body))
                || else_branch.as_ref().is_some_and(any_list)
        }
        CompoundCommand::While { condition, body } | CompoundCommand::Until { condition, body } => {
            any_list(condition) || any_list(body)
        }
        CompoundCommand::For { items, body, .. } => {
            items.as_ref().is_some_and(|items| any_word(items)) || any_list(body)
        }
        CompoundCommand::Case { word, items } => {
            is_one(word)
                || items
                    .iter()
                    .any(|item| any_word(&item.patterns) || any_list(&item.body))
        }
        CompoundCommand::ArithmeticFor { body, .. } => any_list(body),
        CompoundCommand::Subshell(list) | CompoundCommand::Group(list) => any_list(list),
        CompoundCommand::Arithmetic(_) => false,
    }
}

fn any_list(list: &CommandList) -> bool {
    list.items.iter().any(|item| {
        any_pipeline(&item.and_or.first)
            || item
                .and_or
                .rest
                .iter()
                .any(|(_, pipeline)| any_pipeline(pipeline))
    })
}

/// Mirrors [`rewrite_pipeline`], including where it stops: a nested pipeline with stages of its own
/// is not this stage's business, so finding a coordinate in one must not open the gate here.
fn any_pipeline(pipeline: &oslo_base::ast::Pipeline) -> bool {
    pipeline.commands.len() == 1 && pipeline.commands.iter().any(command_uses_coordinates)
}

#[cfg(test)]
#[path = "streams/tests.rs"]
mod tests;
