//! Deciding, before anything runs, which channel each edge of a pipeline carries.
//!
//! **The pipe does not sniff.** By the time output exists the producer has already chosen a form
//! and paid to render it, so asking afterwards is too late — and guessing from bytes is how a
//! shell starts treating a log file as a table. Instead the pipeline is walked right to left before
//! any stage starts, and each one is *told* what its output is for.
//!
//! An edge carries rows only when four things hold at once:
//!
//! 1. the producer declares `produces: Rows`,
//! 2. the consumer declares `accepts: Rows`,
//! 3. no redirection touches either side of that edge, and
//! 4. both stages run in this process — not an external, not a function, not a compound.
//!
//! Anything else is bytes, which is the path oslo has always taken.
//!
//! # Why POSIX is safe
//!
//! Not "we were careful": **vocabulary disjointness**. Structure only flows between two stages that
//! both carry a declaration, and every name that can carry one is either invented by oslo (`where`,
//! `get`, `from`, `each`) or a builtin deliberately declared as bytes. A script written before oslo
//! existed cannot name a structured consumer, so [`plan`] answers [`Sink::Text`] for every edge in
//! it and it takes the byte path unchanged.
//!
//! That claim is checked rather than asserted — see [`entered_structured_path`], which the
//! differential suite reads to prove the corpus never once left the byte path.
//!
//! This is also why there is **no new pipe operator**. Two independent reasons: a second operator
//! means the user has to know which to type, which is the new language this shell exists not to
//! require; and `a |> b` is already valid POSIX — a redirection of an empty command — so the
//! operator would itself be the compatibility hazard.

use std::sync::atomic::{AtomicU64, Ordering};

/// What a stage's output is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sink {
    /// The last stage, with a terminal on the other side: render for a person.
    Print,
    /// Bytes, to a pipe or a file. What every stage did before this existed.
    Text,
    /// Structured rows, handed to the next stage in this process.
    Rows,
}

/// What a stage can take and what it can give.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Bytes on a descriptor.
    Bytes,
    /// Structured rows.
    Rows,
    /// Either — `each` and `to json` take whatever they are given.
    Any,
    /// Reads nothing at all.
    Nothing,
}

impl Shape {
    fn takes_rows(self) -> bool {
        matches!(self, Shape::Rows | Shape::Any)
    }

    fn gives_rows(self) -> bool {
        matches!(self, Shape::Rows | Shape::Any)
    }

    /// Whether a stage can read plain bytes — the property that makes something a *bridge*.
    pub fn takes_bytes(self) -> bool {
        matches!(self, Shape::Bytes | Shape::Any)
    }
}

/// One stage of a pipeline, as the planner needs to see it.
#[derive(Debug, Clone, Copy)]
pub struct Stage {
    /// What it will take from the stage before.
    pub accepts: Shape,
    /// What it can give the stage after.
    pub produces: Shape,
    /// Whether it runs in this process. An external, a shell function and a compound command are
    /// all `false`: structure cannot cross a process, and this is where that is enforced.
    pub in_process: bool,
    /// Whether a redirection touches this stage's own descriptors. A redirection means the user
    /// asked for bytes somewhere specific, and that outranks anything the planner would prefer.
    pub redirected: bool,
}

impl Stage {
    /// A stage that knows nothing about structure — every external command, and every builtin that
    /// has not been declared otherwise.
    pub fn bytes() -> Stage {
        Stage {
            accepts: Shape::Bytes,
            produces: Shape::Bytes,
            in_process: false,
            redirected: false,
        }
    }
}

/// How many times a pipeline has taken the structured path.
///
/// The differential suite runs the whole POSIX corpus and then reads this: it must still be zero.
/// That turns "structure cannot affect a POSIX script" from a claim in a document into a test that
/// fails when it stops being true.
static STRUCTURED_EDGES: AtomicU64 = AtomicU64::new(0);

/// How many structured edges have been planned since the process started.
pub fn entered_structured_path() -> u64 {
    STRUCTURED_EDGES.load(Ordering::Relaxed)
}

/// Reset the counter, for a test that wants to measure one run.
pub fn reset_structured_path() {
    STRUCTURED_EDGES.store(0, Ordering::Relaxed);
}

/// Decide the sink for every stage, right to left.
///
/// `last_is_terminal` says whether the final stage's stdout is a terminal — the difference between
/// drawing a table for a person and writing plain text into a file.
pub fn plan(stages: &[Stage], last_is_terminal: bool) -> Vec<Sink> {
    if stages.is_empty() {
        return Vec::new();
    }
    let mut sinks = vec![Sink::Text; stages.len()];

    // The end of the pipeline first: everything else is decided by what follows it.
    let last = stages.len() - 1;
    sinks[last] = if last_is_terminal && !stages[last].redirected {
        Sink::Print
    } else {
        Sink::Text
    };

    for i in (0..last).rev() {
        let producer = &stages[i];
        let consumer = &stages[i + 1];
        // **A redirection governs what a stage writes, not what it reads.**
        //
        // Blocking the edge *into* a redirected consumer dropped the whole pipeline onto the byte
        // path, where the structured verbs are not commands at all: `… | lines | length >/dev/null`
        // answered `lines: command not found`, so every natural way to write "run this and check
        // `$?`" was unusable. Rows may reach it — but only when it is the last stage, because that
        // is the one whose redirection `structured::run` can apply around its own output. A
        // redirected stage in the middle still forces text, since nothing would apply its
        // redirection at all. The producer's own redirection always forces text: its bytes went to
        // the file, so the consumer has nothing to read.
        let consumer_takes_rows = consumer.accepts.takes_rows()
            && consumer.in_process
            && (!consumer.redirected || i + 1 == last);
        let rows = producer.produces.gives_rows()
            && producer.in_process
            && !producer.redirected
            && consumer_takes_rows;
        sinks[i] = if rows {
            STRUCTURED_EDGES.fetch_add(1, Ordering::Relaxed);
            Sink::Rows
        } else {
            Sink::Text
        };
    }
    sinks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The edge counter is process-wide, so a test that measures it cannot run beside one that
    /// increments it — and every test here plans a pipeline.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn tool(accepts: Shape, produces: Shape) -> Stage {
        Stage {
            accepts,
            produces,
            in_process: true,
            redirected: false,
        }
    }

    /// The shape the whole feature exists for: `df | where ...` on a terminal.
    #[test]
    fn two_structured_stages_pass_rows() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let stages = [
            tool(Shape::Nothing, Shape::Rows),
            tool(Shape::Rows, Shape::Rows),
        ];
        assert_eq!(plan(&stages, true), vec![Sink::Rows, Sink::Print]);
    }

    /// **The POSIX case.** Nothing in a script written before oslo existed declares anything, so
    /// every edge is bytes and the counter never moves.
    #[test]
    fn a_posix_pipeline_never_leaves_the_byte_path() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_structured_path();
        let stages = [Stage::bytes(), Stage::bytes(), Stage::bytes()];
        assert_eq!(
            plan(&stages, false),
            vec![Sink::Text, Sink::Text, Sink::Text]
        );
        assert_eq!(
            entered_structured_path(),
            0,
            "a POSIX pipeline must not reach the structured path"
        );
    }

    /// An external in the middle breaks the chain on both sides, because structure cannot cross a
    /// process. `ls | grep foo | where ...` is bytes throughout.
    #[test]
    fn an_external_breaks_the_chain() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let stages = [
            tool(Shape::Nothing, Shape::Rows),
            Stage::bytes(),
            tool(Shape::Rows, Shape::Rows),
        ];
        assert_eq!(
            plan(&stages, true),
            vec![Sink::Text, Sink::Text, Sink::Print]
        );
    }

    /// Edges are decided one at a time, not as a mode. Structure on the first, bytes on the second.
    #[test]
    fn edges_are_independent() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let stages = [
            tool(Shape::Nothing, Shape::Rows),
            tool(Shape::Rows, Shape::Rows),
            Stage::bytes(),
        ];
        assert_eq!(
            plan(&stages, true),
            vec![Sink::Rows, Sink::Text, Sink::Print]
        );
    }

    /// A redirection means the user asked for bytes *out of* that stage — its own output — and that
    /// outranks the planner. What reaches it is a separate question.
    ///
    /// Blocking the edge into it dropped the whole pipeline to the byte path, where the structured
    /// verbs are not commands: `… | lines | length >/dev/null` answered `lines: command not found`.
    /// So rows still arrive, the stage still writes text, and `structured::run` applies the
    /// redirection around that text.
    #[test]
    fn a_redirection_forces_bytes_out_but_not_in() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut consumer = tool(Shape::Rows, Shape::Rows);
        consumer.redirected = true;
        let stages = [tool(Shape::Nothing, Shape::Rows), consumer];
        assert_eq!(plan(&stages, true), vec![Sink::Rows, Sink::Text]);
    }

    /// But only for the *last* stage, which is the one whose redirection anything applies.
    ///
    /// A redirected stage in the middle sends its bytes to a file, so the stage after it has
    /// nothing to read — and nothing would apply its redirection anyway. Both of its edges are text.
    #[test]
    fn a_redirection_in_the_middle_still_forces_bytes() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut middle = tool(Shape::Rows, Shape::Rows);
        middle.redirected = true;
        let stages = [
            tool(Shape::Nothing, Shape::Rows),
            middle,
            tool(Shape::Rows, Shape::Rows),
        ];
        assert_eq!(
            plan(&stages, true),
            vec![Sink::Text, Sink::Text, Sink::Print]
        );
    }

    /// Redirected output at the end is text, not a drawn table: a file gets the transport form.
    #[test]
    fn the_last_stage_prints_only_to_a_terminal() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let stages = [tool(Shape::Nothing, Shape::Rows)];
        assert_eq!(plan(&stages, true), vec![Sink::Print]);
        assert_eq!(
            plan(&stages, false),
            vec![Sink::Text],
            "into a pipe or file"
        );

        let mut redirected = tool(Shape::Nothing, Shape::Rows);
        redirected.redirected = true;
        assert_eq!(plan(&[redirected], true), vec![Sink::Text]);
    }
}
