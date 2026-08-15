//! Choosing between the byte pipeline and the structured one.
//!
//! Every pipeline oslo has ever run goes through the byte path: one process per stage, bytes on
//! every descriptor. This module answers one question before that happens — *does any edge of this
//! pipeline carry structured rows instead* — and it answers "no" for every pipeline written before
//! oslo invented a vocabulary for saying otherwise.
//!
//! Kept apart from the pipeline itself so the byte path stays exactly as long and exactly as
//! readable as it was, and so the seam between the two is one function rather than a condition
//! threaded through the old code. See `docs/features/structured-pipelines.md`.

use super::Pipeline;
use crate::data::{Sink, Stage};
use crate::env::Environment;
use oslo_base::ast::types::{Command, SimpleCommand, WordPart};
use oslo_base::error::Result;

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
/// The shell's standard input, to the end.
///
/// Lossy rather than refusing: a tool that turns bytes into rows is being handed something the
/// user piped in, and answering "not UTF-8" for one stray byte in a log file would be worse than
/// carrying on. `Val::Bytes` exists for the cell that genuinely holds binary; this is the channel.
fn read_standard_input() -> String {
    use std::io::Read;
    let mut buffer = Vec::new();
    let _ = std::io::stdin().lock().read_to_end(&mut buffer);
    String::from_utf8_lossy(&buffer).into_owned()
}

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
    // **Nothing structured here, so do not ask the terminal.** `is_terminal` is an `ioctl`, and it
    // was being issued for every pipeline the shell ran — one failing syscall per simple command,
    // including a bare `x=1` — to decide the last sink of a plan in which nothing carries rows.
    // A pipeline of plain commands has the same answer either way: bytes all the way down.
    if stages.iter().all(|stage| !stage.in_process) {
        return vec![Sink::Text; stages.len()];
    }
    crate::data::plan(
        &stages,
        std::io::IsTerminal::is_terminal(&std::io::stdout()),
    )
}

/// The sinks for this pipeline, when any edge of it carries rows.
///
/// One call, not two: [`plan`](crate::data::plan) counts the structured edges it hands out, and
/// planning the same pipeline twice would report twice as many as were actually taken — which
/// matters because that count is what the POSIX assertion reads.
pub(super) fn structured_sinks(pipeline: &Pipeline) -> Option<Vec<Sink>> {
    // **Asked before anything is built.** Planning allocates a `Vec` of stages, clones every
    // command word, locks the registry once per stage, and issues an `ioctl` to ask whether stdout
    // is a terminal. With nothing registered the answer is `None` for every pipeline, so all of
    // that was spent to learn nothing — measured at one failing `ioctl` per simple command, on the
    // hottest path the shell has.
    if !crate::data::tool::any_registered() {
        return None;
    }
    let sinks = plan_pipeline(pipeline);
    if sinks.contains(&Sink::Rows) {
        return Some(sinks);
    }
    // **A tool on its own has no edge, and `Sink::Rows` only ever describes an edge.** So a
    // pipeline of one plans to no rows at all and falls to the byte path, where the name is looked
    // up on `$PATH`.
    //
    // For `ls`, `ps` and `df` that is the right answer and must stay: a bare `ls` is coreutils, and
    // oslo's row-shaped one is what you get when you ask for structure by piping it somewhere. A
    // name a config invented has no such counterpart — `oslo.register_tool{name = "stale", …}` then
    // answered `stale: command not found`, and only `stale | length` ran it, which is not a
    // discoverable interface for a feature whose whole point is adding a command.
    lone_custom_tool(pipeline).then_some(sinks)
}

/// One simple command, naming a tool the *config* registered, with nothing redirected.
///
/// Redirections are excluded because [`run`] writes with `println!` — the structured stages do not
/// fork, so nothing has applied `> file` to this process's stdout, and routing `stale > out` here
/// would print to the terminal and leave the file empty. The byte path keeps those.
fn lone_custom_tool(pipeline: &Pipeline) -> bool {
    let [Command::Simple(simple)] = pipeline.commands.as_slice() else {
        return false;
    };
    simple.redirections.is_empty()
        && simple_command_name(simple).is_some_and(|name| crate::data::custom::registered(&name))
}

/// Run a pipeline whose edges carry rows.
///
/// **No forking.** Every stage here runs in this process, one after another, and the rows move by
/// being handed over — there is no descriptor between them, so there is nothing to serialise and
/// no format to get wrong. That is the whole argument for keeping structure in-process.
///
/// `fallback` is the byte path, used for anything this cannot run.
pub(super) fn run(
    env: &mut Environment,
    pipeline: &Pipeline,
    sinks: &[Sink],
    fallback: fn(&mut Environment, &Pipeline) -> Result<i32>,
) -> Result<i32> {
    let mut rows: Option<Vec<crate::data::Record>> = None;
    let mut statuses = Vec::with_capacity(pipeline.commands.len());

    // **The byte prefix.** `kubectl get pods -o json | from json | where ...` is an external
    // followed by structured stages: the external cannot hand over rows, so its *output* is what
    // the first structured stage is given. Everything up to the first tool runs exactly as it
    // always did, forked and on descriptors, and only its bytes cross into this half.
    let first_tool = pipeline.commands.iter().position(|c| {
        matches!(c, Command::Simple(s)
            if simple_command_name(s).is_some_and(|n| crate::data::tool::lookup(&n).is_some()))
    });
    let mut bytes: Option<String> = None;
    let start = match first_tool {
        Some(0) | None => {
            // **A bytes-taking tool at the head of the pipeline reads the shell's own input.**
            //
            // There is no stage before it, so `bytes` stayed `None` and `lines` was handed the
            // empty string:
            //
            // ```text
            // printf 'a\nb\nc\n' | oslo -c 'lines | length'      answered 0
            // printf 'a\nb\nc\n' | oslo -c 'cat | lines | length'  answers 3
            // ```
            //
            // Zero rows for three lines of input, silently — a wrong answer where a refusal would
            // have been survivable, which is the one failure `docs/known-gaps.md` opens by saying
            // oslo does not have. It does now what `cat` would do in its place: read standard
            // input to the end. At a terminal that waits for Ctrl-D, exactly as `wc -l` does.
            if let Some(Command::Simple(first)) = pipeline.commands.first()
                && let Some(name) = simple_command_name(first)
                && crate::data::tool::lookup(&name).is_some_and(|tool| {
                    matches!(
                        tool.accepts,
                        crate::data::Shape::Bytes | crate::data::Shape::Any
                    )
                })
            {
                bytes = Some(read_standard_input());
            }
            0
        }
        Some(at) => {
            let prefix = Pipeline {
                commands: pipeline.commands[..at].to_vec(),
                negated: false,
                timed: false,
            };
            match capture(env, &prefix, fallback) {
                Ok((status, output)) => {
                    // One status per prefix stage; the byte path filled in its own vector, and
                    // this half appends to it rather than replacing it.
                    statuses.extend(env.pipeline_status().iter().copied());
                    if statuses.is_empty() {
                        statuses.push(status);
                    }
                    bytes = Some(output);
                    at
                }
                Err(e) => return Err(e),
            }
        }
    };

    for (i, command) in pipeline.commands.iter().enumerate().skip(start) {
        let Command::Simple(simple) = command else {
            return fallback(env, pipeline);
        };
        let Some(name) = simple_command_name(simple) else {
            return fallback(env, pipeline);
        };
        let words = expand_words(env, simple)?;

        let (status, produced) =
            match crate::data::tools::run_tool(&name, &words, rows.take(), bytes.as_deref()) {
                Some(outcome) => outcome,
                // A stage the planner thought was a tool but which cannot run here. Falling back for
                // the whole pipeline is the only honest answer: half of it has not run yet, so there
                // is nothing to undo.
                None => return fallback(env, pipeline),
            };
        statuses.push(status);
        rows = produced;
        // The bytes belong to the first structured stage only; a second `lines` further down the
        // pipeline is reading rows, not the original output all over again.
        bytes = None;

        // Everything but the last stage hands its rows on. The last one writes them out, in
        // whichever of the two renderings its sink asked for.
        if i + 1 == pipeline.commands.len()
            && let Some(final_rows) = rows.take()
        {
            {
                let value = crate::data::Val::table(final_rows);
                let text = match sinks.last() {
                    Some(Sink::Print) => crate::data::render_display(&value),
                    // Into a pipe or a file: the plain, complete form. **Never the drawn table** —
                    // a box-drawing character on another program's stdin is the failure this whole
                    // design exists to prevent.
                    _ => crate::data::render_transport(&value),
                };
                if !text.is_empty() {
                    println!("{text}");
                }
            }
        }
    }

    let status = statuses.last().copied().unwrap_or(0);
    // The stages did not fork, so there are no child statuses to collect — but `PIPESTATUS` must
    // still describe the pipeline the user wrote, or `${PIPESTATUS[0]}` starts lying the moment a
    // pipeline happens to be structured.
    env.set_pipeline_status(statuses);
    Ok(status)
}

/// Run the byte half of a mixed pipeline and collect what it printed.
///
/// The prefix runs through the ordinary path — same forks, same descriptors, same everything — with
/// stdout pointed at a pipe instead of the terminal. Nothing about how those commands execute
/// changes; only where their output goes.
fn capture(
    env: &mut Environment,
    prefix: &Pipeline,
    fallback: fn(&mut Environment, &Pipeline) -> Result<i32>,
) -> Result<(i32, String)> {
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};

    let (reader, writer) = nix::unistd::pipe()
        .map_err(|e| oslo_base::error::ShellError::ExecutionError(format!("pipe: {e}")))?;

    // stdout is put back whatever happens below, including on the error path: leaving the shell
    // writing into a closed pipe would be a far worse failure than the one being reported.
    let saved = nix::unistd::dup(std::io::stdout().as_raw_fd())
        .map_err(|e| oslo_base::error::ShellError::ExecutionError(format!("dup: {e}")))?;
    let _ = nix::unistd::dup2(writer.as_raw_fd(), std::io::stdout().as_raw_fd());
    drop(writer);

    let status = fallback(env, prefix);

    // SAFETY: `saved` is a descriptor this function created with `dup` and has not closed.
    let saved = unsafe { std::os::fd::OwnedFd::from_raw_fd(saved) };
    let _ = nix::unistd::dup2(saved.as_raw_fd(), std::io::stdout().as_raw_fd());

    let mut output = String::new();
    let mut reader = std::fs::File::from(reader);
    let _ = reader.read_to_string(&mut output);
    Ok((status?, output))
}

/// The words of a simple command, expanded as the byte path would expand them.
fn expand_words(env: &mut Environment, simple: &SimpleCommand) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for word in &simple.words {
        out.extend(crate::expand::expand_word(env, word)?);
    }
    Ok(out)
}
