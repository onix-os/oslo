//! A structured pipeline whose upstream has no end.
//!
//! ```text
//! tail -f app.log | lines | where 'line:match("ERROR")'
//! yes             | lines | first 2
//! ```
//!
//! # What was wrong
//!
//! Everything in the structured half materialises. [`super::capture`] reads the byte prefix **to
//! end of file** into a `String` before the first verb runs, so a pipeline is three phases in
//! sequence — read it all, transform it all, print it all — rather than a flow. An upstream that
//! does not end has no end to read to, so the first phase never finishes and the second never
//! starts: `tail -f app.log | lines | where …` printed nothing at all, for ever, where
//! `tail -f app.log | grep ERROR` prints as it goes.
//!
//! # What it does instead
//!
//! Reads the upstream in slices, turns each slice into rows, pushes those through the verbs, and
//! prints what comes out — then goes back for more. Output appears as the data does, memory is
//! bounded by one slice, and when a verb has seen enough the reader is **closed**, which is what
//! gives the upstream its `SIGPIPE` and ends it. That is the mechanism `yes | head -2` has always
//! used, arriving in the structured half at last.
//!
//! # The prefix is forked, not re-implemented
//!
//! The upstream runs through `run_byte_stages` inside a child of this process, with its stdout on a
//! pipe. **Not a second way to run an external**: spawning it here with `std::process::Command`
//! would mean re-deriving argv, the environment, `$PATH` resolution and the exit status, and any of
//! those drifting from the byte path is a bug that only shows up in a structured pipeline. A fork
//! that calls the same function is exact by construction, and one extra process is what a subshell
//! costs anyway.
//!
//! # Where it does *not* apply, and why the list is short
//!
//! A pipeline is streamed only when every part of it can be, and the fallback is today's path
//! unchanged. See [`plan`]. The two rules worth stating:
//!
//! * **The bridge has to be line-oriented.** `lines` and `parse` answer a row per line, so a slice
//!   of input is a batch of rows. `from json` needs the closing brace before it can answer at all,
//!   and `detect-columns` needs every row before it can say where the columns are — neither is a
//!   thing a stream can give.
//! * **Every verb has to be row-local, one of the four that count, or one of the two that fold.**
//!   Applying `where` to a batch is exactly applying it to each row, so batching is invisible.
//!   Applying `first 2` to each batch would take two rows *per batch*, which is a wrong answer — so
//!   `first`, `skip`, `every` and `enumerate` carry their count across batches instead. `length`
//!   and `final n` answer only at the end but hold a **bound** while they wait — a counter, and n
//!   rows. Anything that has to hold the whole stream to answer — `sort-by`, `group-by`, `stats`,
//!   `reverse` — is not streamable at all, and says so by not being in a list.
//!
//! # A stream cannot be a table
//!
//! The drawn face aligns columns to their widest value, which is a question only the last row can
//! answer. A stream has no last row, so streamed output is a header and then one line per row, each
//! cell rendered for a person but not padded. The alternative is holding every row to align it,
//! which is the thing this module exists not to do.

use super::super::Pipeline;
use crate::data::{Record, Sink, Val};
use crate::env::Environment;
use oslo_base::ast::types::Command;
use oslo_base::error::{Result, ShellError};

/// Verbs whose answer for a batch is the same as their answer for each row in it.
///
/// So batching them is invisible: no state crosses a batch, and the rows that come out of one are
/// exactly the rows that would have come out had the whole stream been present.
/// `upsert` belongs here and `insert` and `update` do not, which looks arbitrary until you read what
/// separates them: those two refuse based on whether the column exists **anywhere in the stream**,
/// which a stream cannot answer until it ends, and `upsert` — `When::Either` — has no refusal at all.
///
/// `each` produces no rows by design; it runs its expression for the side effect. Streaming it is
/// what makes `tail -f app.log | lines | each 'print(line)'` print as the log grows.
const ROW_LOCAL: &[&str] = &[
    "where", "map", "cols", "get", "reject", "rename", "flatten", "compact", "default", "upsert",
    "each",
];

/// Verbs that count, and therefore have to carry their count across batches.
const POSITIONAL: &[&str] = &["first", "skip", "every", "enumerate"];

/// Verbs that answer only once the stream ends, but need to remember **no more than a bound** to do
/// it.
///
/// The distinction that matters is not "needs the last row" — `sort-by` needs it too and can never
/// stream. It is *how much* has to be held to answer: `length` holds a counter, and `final n` holds
/// n rows. `sort-by` holds all of them, which is the thing this module exists not to do.
///
/// So these turn a pipeline that **failed** into one that works: `cat 300MB.log | lines | length`
/// used to hit the 256 MiB cap and report that the upstream was too large, for a question whose
/// answer is one integer.
const FOLDING: &[&str] = &["length", "final"];

/// The shape of a pipeline that can be streamed.
pub(super) struct Streamed {
    /// The one command before the bridge — the upstream.
    prefix: Pipeline,
    /// Index of the `lines` or `parse` that turns bytes into rows.
    bridge: usize,
}

/// Whether this pipeline can be streamed, and where its parts are.
///
/// Every condition is a reason the general path would be wrong rather than merely slower, so a
/// `None` here is not a missed opportunity — it is the honest answer.
pub(super) fn plan(pipeline: &Pipeline, sinks: &[Sink]) -> Option<Streamed> {
    // One upstream, then a bridge, then at least nothing.
    if pipeline.commands.len() < 2 {
        return None;
    }
    let bridge = 1;

    // The upstream is a single simple command that is not itself a tool, and nothing redirects it —
    // a redirection means its bytes were asked for somewhere else.
    let Command::Simple(upstream) = &pipeline.commands[0] else {
        return None;
    };
    if !upstream.redirections.is_empty() {
        return None;
    }
    if super::simple_command_name(upstream)
        .is_some_and(|name| crate::data::tool::lookup(&name).is_some())
    {
        return None;
    }

    // The bridge answers a row per line, so a slice of bytes is a batch of rows.
    let name = tool_name(&pipeline.commands[bridge])?;
    if name != "lines" && name != "parse" {
        return None;
    }

    // Everything after it either does not care about batching or is one of the four that count.
    for command in &pipeline.commands[bridge + 1..] {
        let name = tool_name(command)?;
        if !ROW_LOCAL.contains(&name.as_str())
            && !POSITIONAL.contains(&name.as_str())
            && !FOLDING.contains(&name.as_str())
        {
            return None;
        }
    }

    // The last stage writes rows out; a byte suffix is `hand_over`'s job and wants the whole table.
    if !matches!(sinks.last(), Some(Sink::Print | Sink::Text)) {
        return None;
    }

    Some(Streamed {
        prefix: Pipeline {
            commands: pipeline.commands[..bridge].to_vec(),
            negated: false,
            timed: false,
        },
        bridge,
    })
}

/// A simple command naming a registered tool with nothing redirected.
fn tool_name(command: &Command) -> Option<String> {
    let Command::Simple(simple) = command else {
        return None;
    };
    if !simple.redirections.is_empty() {
        return None;
    }
    let name = super::simple_command_name(simple)?;
    crate::data::tool::lookup(&name).map(|_| name)
}

/// How many rows a counting verb has already let past.
#[derive(Default)]
struct Counted {
    seen: usize,
    /// Set once a `first n` has had its fill: nothing more will ever come out, so the upstream can
    /// be let go.
    finished: bool,
    /// What a [`FOLDING`] verb is holding — the last n rows for `final`, and nothing at all for
    /// `length`, which needs only `seen`.
    kept: Vec<Record>,
}

/// Run the pipeline as a stream. Answers its exit status.
pub(super) fn run(
    env: &mut Environment,
    pipeline: &Pipeline,
    sinks: &[Sink],
    plan: &Streamed,
    fallback: fn(&mut Environment, &Pipeline) -> Result<i32>,
) -> Result<i32> {
    use std::io::Read;
    use std::os::fd::AsRawFd;

    let (reader, writer) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)
        .map_err(|e| ShellError::ExecutionError(format!("pipe: {e}")))?;

    // **The upstream runs in a child, through the ordinary byte path.** Same expansion, same
    // `$PATH` walk, same redirections, same everything — because it is the same function.
    let child = match unsafe { nix::unistd::fork() } {
        Ok(nix::unistd::ForkResult::Child) => {
            let _ = nix::unistd::dup2(writer.as_raw_fd(), std::io::stdout().as_raw_fd());
            drop(reader);
            // A fresh signal state, as every forked stage gets: the inherited `SIG_IGN` for
            // SIGPIPE is what would otherwise keep `yes` alive after this process stops reading.
            crate::exec::job::reset_signals_for_child();
            env.enter_subshell();
            let status = fallback(env, &plan.prefix).unwrap_or(1);
            crate::exec::compound::flush_stdout();
            std::process::exit(status);
        }
        Ok(nix::unistd::ForkResult::Parent { child }) => child,
        Err(e) => return Err(ShellError::ExecutionError(format!("fork: {e}"))),
    };
    // The parent holds only the read end, so the pipe closes when the child exits.
    drop(writer);

    let mut source = std::fs::File::from(reader);
    let mut pending = String::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let mut counters: Vec<Counted> = (0..pipeline.commands.len())
        .map(|_| Counted::default())
        .collect();
    let mut header_written = false;
    let mut interrupted = false;
    // **One status per stage, and the verb's own rather than a flag.** Two things went wrong when
    // this was a single number. Flattening every failure to 1 lost the difference between "this
    // filter broke" and "that column does not exist", which the rest of the shell reports as 2. And
    // reporting the *upstream's* failure as the pipeline's applied `pipefail` when it was off:
    // `false | lines | length` answered 1 where a pipeline reports its last stage. A vector lets
    // `pipeline_status` decide, which is the same helper the byte path and the materialised path
    // both ask.
    let mut statuses = vec![0; pipeline.commands.len()];

    loop {
        if crate::exec::job::interrupt_pending() {
            let _ = nix::sys::signal::kill(child, nix::sys::signal::Signal::SIGINT);
            interrupted = true;
            break;
        }
        let read = match source.read(&mut chunk) {
            Ok(0) => 0,
            Ok(read) => read,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => 0,
        };
        if read == 0 {
            // End of the upstream. Whatever is left without a newline is still a line — and this
            // runs even when nothing is left, because it is the call that lets a fold answer.
            let done = emit(
                env,
                pipeline,
                sinks,
                plan,
                &pending,
                &mut counters,
                &mut header_written,
                &mut statuses,
                true,
            )?;
            let _ = done;
            break;
        }
        pending.push_str(&String::from_utf8_lossy(&chunk[..read]));
        // Only whole lines are turned into rows; a half-arrived line waits for the rest of itself.
        let Some(last) = pending.rfind('\n') else {
            continue;
        };
        let batch: String = pending.drain(..=last).collect();
        if emit(
            env,
            pipeline,
            sinks,
            plan,
            &batch,
            &mut counters,
            &mut header_written,
            &mut statuses,
            false,
        )? {
            // A verb has had its fill. Dropping the read end is what ends the upstream.
            break;
        }
    }

    drop(source);
    // Reaped however this ended, so a finished stream leaves no zombie and a `first n` that closed
    // the pipe early collects the `SIGPIPE` death rather than leaving it.
    let reaped = super::super::wait_for_child(child).0;
    statuses[0] = match reaped {
        // A `SIGPIPE` death is this pipeline working as intended, not a failure — `head` ends `yes`
        // the same way and nobody calls that an error.
        141 => 0,
        other => other,
    };
    if interrupted {
        return Ok(130);
    }
    // **`pipeline_status`, not `last()` and not the upstream's.** `pipefail` is a property of the
    // pipeline rather than of the path it happens to run on, so a streamed pipeline and a
    // materialised one must answer the same number for the same failure.
    let status = super::super::pipeline_status(env, &statuses);
    env.set_pipeline_status(statuses);
    Ok(status)
}

/// Turn one slice of input into rows, push them through the verbs, and write what survives.
///
/// Answers whether the stream is finished — a `first n` that has had its rows.
#[allow(clippy::too_many_arguments)]
fn emit(
    env: &mut Environment,
    pipeline: &Pipeline,
    sinks: &[Sink],
    plan: &Streamed,
    text: &str,
    counters: &mut [Counted],
    header_written: &mut bool,
    statuses: &mut [i32],
    at_end: bool,
) -> Result<bool> {
    let bridge = &pipeline.commands[plan.bridge];
    let Command::Simple(simple) = bridge else {
        return Ok(true);
    };
    let words = super::expand_words(env, simple)?;
    let name = super::simple_command_name(simple).unwrap_or_default();
    let Some((code, produced)) = crate::data::tools::run_tool(&name, &words, None, Some(text))
    else {
        return Ok(true);
    };
    if code != 0 {
        statuses[plan.bridge] = code;
    }
    let mut rows = produced.unwrap_or_default();

    // **Whether the stream is over, which a verb can decide part-way along the chain.** A satisfied
    // `first n` ends it for everything downstream of itself, so a fold after one has to answer in
    // this same pass — there will be no other.
    let mut ended = at_end;

    for (at, command) in pipeline.commands.iter().enumerate().skip(plan.bridge + 1) {
        // **Not when the stream has ended**, because that is exactly when a fold has something to
        // say and an empty last batch is the ordinary way to arrive there.
        if rows.is_empty() && !ended {
            break;
        }
        let Command::Simple(simple) = command else {
            return Ok(true);
        };
        let words = super::expand_words(env, simple)?;
        let name = super::simple_command_name(simple).unwrap_or_default();
        if FOLDING.contains(&name.as_str()) {
            rows = folded(&name, &words, rows, &mut counters[at], ended);
            // Nothing flows past a fold until the last row has gone into it.
            if !ended {
                return Ok(false);
            }
            continue;
        }
        if POSITIONAL.contains(&name.as_str()) {
            let (kept, done) = counted(&name, &words, rows, &mut counters[at]);
            rows = kept;
            // **Not a return.** Stopping here wrote the rows out and never reached the rest of the
            // chain, so `first 2 | length` printed the two rows where the materialised path
            // answered `2`. What a satisfied count ends is the *reading*, not the pipeline.
            ended |= done;
            continue;
        }
        let Some((code, produced)) = crate::data::tools::run_tool(&name, &words, Some(rows), None)
        else {
            return Ok(true);
        };
        if code != 0 {
            statuses[at] = code;
        }
        rows = produced.unwrap_or_default();
    }

    write(&rows, sinks, header_written);
    Ok(ended)
}

/// A folding verb: swallow the batch, and answer only once the last one has gone in.
///
/// **What is held is bounded, which is the whole licence to do this.** `length` keeps a count and
/// `final n` keeps n rows, so the memory a fold costs does not grow with the upstream. The answer
/// at the end is the real verb's, computed from what was kept, so a streamed `final 3` and a
/// materialised one cannot drift apart.
fn folded(
    name: &str,
    words: &[String],
    rows: Vec<Record>,
    state: &mut Counted,
    at_end: bool,
) -> Vec<Record> {
    let n: usize = words.get(1).and_then(|w| w.parse().ok()).unwrap_or(1);
    match name {
        "final" => {
            state.kept.extend(rows);
            // Trimmed every batch rather than at the end, so the window is the bound rather than a
            // thing that merely gets truncated once the whole stream has been held.
            let over = state.kept.len().saturating_sub(n);
            state.kept.drain(..over);
            match at_end {
                true => std::mem::take(&mut state.kept),
                false => Vec::new(),
            }
        }
        // `length`, whose answer needs the count and nothing else.
        _ => {
            state.seen += rows.len();
            match at_end {
                true => vec![Record::from_pairs([(
                    "length",
                    Val::Int(state.seen as i64),
                )])],
                false => Vec::new(),
            }
        }
    }
}

/// A counting verb, with its count carried from the batch before.
///
/// Answers the rows that survive and whether nothing more will ever come out.
fn counted(
    name: &str,
    words: &[String],
    rows: Vec<Record>,
    state: &mut Counted,
) -> (Vec<Record>, bool) {
    let n: usize = words.get(1).and_then(|w| w.parse().ok()).unwrap_or(1);
    match name {
        "first" => {
            let room = n.saturating_sub(state.seen);
            let kept: Vec<Record> = rows.into_iter().take(room).collect();
            state.seen += kept.len();
            state.finished = state.seen >= n;
            (kept, state.finished)
        }
        "skip" => {
            let dropping = n.saturating_sub(state.seen).min(rows.len());
            state.seen += dropping;
            (rows.into_iter().skip(dropping).collect(), false)
        }
        "every" => {
            let mut kept = Vec::new();
            for row in rows {
                if n > 0 && state.seen.is_multiple_of(n) {
                    kept.push(row);
                }
                state.seen += 1;
            }
            (kept, false)
        }
        // `enumerate`, whose index has to keep counting across batches or every batch would start
        // again at zero — the wrong answer, and a quiet one.
        _ => {
            let mut kept = Vec::new();
            for row in rows {
                let mut out = Record::from_pairs([("index", Val::Int(state.seen as i64))]);
                for (column, value) in row.columns().iter().zip(row.values()) {
                    out.set(column, value.clone());
                }
                state.seen += 1;
                kept.push(out);
            }
            (kept, false)
        }
    }
}

/// Write a batch out, with the header once.
///
/// **Not the drawn table.** Aligning columns needs the widest value, which needs the last row, which
/// a stream does not have. So each cell is rendered for a person — `4.2G` rather than the byte count
/// — and the row is written plainly. Into a pipe it is the transport form exactly as always, because
/// the program on the other end is reading records rather than looking at them.
fn write(rows: &[Record], sinks: &[Sink], header_written: &mut bool) {
    if rows.is_empty() {
        return;
    }
    let drawn = matches!(sinks.last(), Some(Sink::Print));
    if drawn && !*header_written {
        *header_written = true;
        let table = Val::table(rows.to_vec());
        println!("{}", table.columns().join("\t"));
    }
    for row in rows {
        let line = match drawn {
            true => row
                .values()
                .iter()
                .map(crate::data::render_display)
                .collect::<Vec<_>>()
                .join("\t"),
            false => crate::data::render_transport(&Val::Record(row.clone())),
        };
        println!("{line}");
    }
    crate::exec::compound::flush_stdout();
}

#[cfg(test)]
#[path = "stream/tests.rs"]
mod tests;
