//! Choosing between the byte pipeline and the structured one.
//!
//! Every pipeline oslo has ever run goes through the byte path: one process per stage, bytes on
//! every descriptor. This module answers one question before that happens — *does any edge of this
//! pipeline carry structured rows instead* — and it answers "no" for every pipeline written before
//! oslo invented a vocabulary for saying otherwise.
//!
//! Kept apart from the pipeline itself so the byte path stays exactly as long and exactly as
//! readable as it was, and so the seam between the two is one function rather than a condition
//! threaded through the old code. See `docs/research/dual-channel-pipe.md`.

use super::Pipeline;
use crate::ast::types::{Command, SimpleCommand, WordPart};
use crate::data::{Sink, Stage};
use crate::env::Environment;
use crate::error::Result;

/// The literal command name of a simple command, when it has one.
///
/// Only a plain literal counts. A name that comes out of an expansion — `$cmd foo` — is not known
/// until the command runs, and a planner that guessed could send structure to something that
/// turned out to be an external. Unknown means bytes, which is always safe.
fn simple_command_name(simple: &SimpleCommand) -> Option<String> {
    let word = simple.words.first()?;
    match word.parts.as_slice() {
        [WordPart::Literal(name)] if !name.is_empty() => Some(name.clone()),
        _ => None,
    }
}

/// How each stage of this pipeline will be asked to write its output.
///
/// Every stage that is not a simple command naming a registered tool is reported as plain bytes,
/// which is what keeps compound commands, functions and externals on the path they always took.
fn plan_pipeline(pipeline: &Pipeline) -> Vec<Sink> {
    let stages: Vec<Stage> = pipeline
        .commands
        .iter()
        .map(|command| {
            let Command::Simple(simple) = command else {
                // A compound command or a function: not in-process in the sense that matters,
                // because its own stages are pipelines of their own.
                return Stage::bytes();
            };
            let Some(name) = simple_command_name(simple) else {
                return Stage::bytes();
            };
            match crate::data::tool::lookup(&name) {
                Some(tool) => Stage {
                    accepts: tool.accepts,
                    produces: tool.produces,
                    in_process: true,
                    // A redirection means the user asked for bytes at a named place, and that
                    // outranks anything the planner would prefer.
                    redirected: !simple.redirections.is_empty(),
                },
                None => Stage::bytes(),
            }
        })
        .collect();
    crate::data::plan(
        &stages,
        std::io::IsTerminal::is_terminal(&std::io::stdout()),
    )
}

/// Whether any edge of this pipeline carries rows.
pub(super) fn has_structured_edge(pipeline: &Pipeline) -> bool {
    plan_pipeline(pipeline).contains(&Sink::Rows)
}

/// Run a pipeline that has at least one structured edge.
///
/// Nothing reaches this yet: no tool declares itself, so [`has_structured_edge`] is false for every
/// pipeline. It is wired now, with the corpus proving it is unreachable, so that the commit adding
/// the first structured tool is small and its blast radius has already been measured.
///
/// `fallback` is the byte path, taken until this has something of its own to do — a fallback rather
/// than a panic, so a config registering a tool before the machinery exists degrades to today's
/// behaviour instead of killing the shell.
pub(super) fn run(
    env: &mut Environment,
    pipeline: &Pipeline,
    fallback: fn(&mut Environment, &Pipeline) -> Result<i32>,
) -> Result<i32> {
    fallback(env, pipeline)
}
