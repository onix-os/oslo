//! What each link of `a && b || c` did, for the line that just ran.
//!
//! # Why the shell throws this away today
//!
//! [`super::eval_and_or_list`] walks the chain and keeps one number: the status of the last
//! pipeline that ran. Everything else — which link failed, which never ran at all, how long each
//! took — is computed and then dropped on the floor. `$PIPESTATUS` does the same job one level
//! down, for the stages *inside* one pipeline, and stops there.
//!
//! # `ran` is the field that matters
//!
//! In `a && b`, a failing `a` means `b` **did not run**, which is not the same as `b` exiting 0 and
//! not the same as it failing. Nothing in a shell records that distinction, and it is the whole
//! value of this buffer: "the chain stopped here" is only answerable if you can tell a link that
//! was skipped from one that succeeded.
//!
//! # Only the line you typed
//!
//! `if a && b; then c && d; fi` has and-or lists inside it, and they are not links of the typed
//! line — recording them would interleave two different chains into one list. `enter` answers
//! "is this the outermost chain since the REPL armed us", so the nested ones record nothing.
//!
//! Sequential items *are* recorded, all of them: `a; b && c` is one line and three links, and
//! `enter` says yes to both of its chains because the first has finished by the time the second
//! starts.
//!
//! # Thread-local, not on `Environment`
//!
//! `$PIPESTATUS` lives on `Environment` because it is a shell *variable* a script can read. This is
//! not one. A thread-local keeps it out of the environment that gets cloned into subshells and
//! saved by scope frames, and the REPL, the recorder and the Lua hook that reads it are all on one
//! thread by construction.

use crate::ast::AndOrOp;
use std::cell::{Cell, RefCell};

/// How a link was joined to the one before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Join {
    /// The first link of a chain — nothing joined it to anything.
    First,
    /// `&&`
    And,
    /// `||`
    Or,
    /// `;` or a newline: a new chain in the same typed line.
    Then,
}

impl Join {
    /// The operator as it is written, for a report or a resumable line.
    pub fn written(self) -> &'static str {
        match self {
            Join::First => "",
            Join::And => "&&",
            Join::Or => "||",
            Join::Then => ";",
        }
    }

    pub fn of(op: AndOrOp) -> Join {
        match op {
            AndOrOp::And => Join::And,
            AndOrOp::Or => Join::Or,
        }
    }
}

/// One stage of a pipeline: `a` and `b` in `a | b`.
///
/// **No duration, deliberately.** The stages of a pipeline are forked and run *at the same time* —
/// that is what a pipe is — so a wall clock per stage would print one number three times and read
/// as though each had taken the whole pipeline's time. What is genuinely per-stage is the status,
/// which is `$PIPESTATUS`, and the text.
#[derive(Debug, Clone)]
pub struct Stage {
    pub text: String,
    pub status: i32,
}

/// One link of the typed line.
#[derive(Debug, Clone)]
pub struct Segment {
    /// Position in the whole line, from zero.
    pub index: usize,
    pub join: Join,
    /// The link as text, rendered from the AST by `describe`.
    pub text: String,
    /// `None` when it never ran — see the module note.
    pub status: Option<i32>,
    pub duration_ms: i64,
    /// The stages of this link, when it was a pipeline. Empty for a link of one command, which is
    /// the overwhelmingly common case and where a one-entry list would be noise.
    pub stages: Vec<Stage>,
}

impl Segment {
    /// Whether this link actually ran. The distinction the buffer exists for.
    pub fn ran(&self) -> bool {
        self.status.is_some()
    }
}

thread_local! {
    /// The links of the line being run, in order.
    static SEGMENTS: RefCell<Vec<Segment>> = const { RefCell::new(Vec::new()) };
    /// Whether the REPL asked for this line to be recorded. Off for scripts and `-c`, where there
    /// is no prompt to offer a resume at and nobody to read a timing report.
    static ARMED: Cell<bool> = const { Cell::new(false) };
    /// Whether an outer chain is already being recorded, so a nested one is not.
    static INSIDE: Cell<bool> = const { Cell::new(false) };
    /// The stages of the pipeline that just finished, waiting for its link to be pushed.
    static PENDING_STAGES: RefCell<Vec<Stage>> = const { RefCell::new(Vec::new()) };
    /// The last line that was a *chain*, kept for `chain` to report on. See [`disarm`].
    static LAST: RefCell<Vec<Segment>> = const { RefCell::new(Vec::new()) };
}

/// Start recording the line about to run, discarding the last one.
pub fn arm() {
    ARMED.set(true);
    INSIDE.set(false);
    SEGMENTS.with(|s| s.borrow_mut().clear());
    PENDING_STAGES.with(|s| s.borrow_mut().clear());
}

/// Stop recording, and keep the line if it was a chain.
///
/// **The kept copy is what `chain` reads, and it has to exist.** Asking about the last chain means
/// typing a command, and the loop arms — and so clears — before every line, so by the time `chain`
/// runs the live buffer holds `chain` and nothing else. It reported "nothing has run yet" for
/// every chain there had ever been.
///
/// Only a *chain* is kept, so `chain`, `chain resume` and any other single command you type in
/// between do not displace the thing you are asking about. "The last chain" is the useful reading
/// and the only one that survives having to type at all.
pub fn disarm() {
    ARMED.set(false);
    INSIDE.set(false);
    SEGMENTS.with(|s| {
        let line = s.borrow();
        if line.len() > 1 {
            LAST.with(|last| *last.borrow_mut() = line.clone());
        }
    });
}

/// Whether this chain is the outermost one since [`arm`], and should therefore record.
///
/// Marks the shell as inside a chain when it answers `true`; the caller must pass that answer back
/// to [`leave`] whatever happens, including on the error path.
pub fn enter() -> bool {
    if !ARMED.get() || INSIDE.get() {
        return false;
    }
    INSIDE.set(true);
    true
}

/// Undo `enter`. A no-op for a nested chain, which never claimed anything.
pub fn leave(was_outermost: bool) {
    if was_outermost {
        INSIDE.set(false);
    }
}

/// Add a link that ran, with what it did.
pub fn record(join: Join, text: String, status: i32, duration_ms: i64) {
    push(join, text, Some(status), duration_ms);
}

/// Add a link the chain short-circuited past.
pub fn record_skipped(join: Join, text: String) {
    push(join, text, None, 0);
}

/// Note the stages of the pipeline currently running, for the link about to be pushed.
///
/// A pipeline finishes before its link is recorded — the status is not known until every stage has
/// been reaped — so the stages are left here and collected when the link itself is. Nothing else can
/// arrive in between: a link is one pipeline, and the recorder only ever follows the outermost
/// chain.
pub fn note_stages(stages: Vec<Stage>) {
    if !ARMED.get() {
        return;
    }
    PENDING_STAGES.with(|s| *s.borrow_mut() = stages);
}

/// [`note_stages`] from what the pipeline evaluator has in hand.
///
/// `$PIPESTATUS` keeps the same numbers for a script to read; this keeps them beside the text that
/// produced each one, which is what makes "which stage failed" answerable without counting on your
/// fingers. Nothing is rendered when the recorder is off — `describe_command` walks an AST, and a
/// script should not pay for a report nobody will read.
pub fn note_pipeline(commands: &[crate::ast::Command], statuses: &[i32]) {
    if !ARMED.get() || commands.len() < 2 {
        return;
    }
    note_stages(
        commands
            .iter()
            .zip(statuses)
            .map(|(command, status)| Stage {
                text: super::describe::describe_command(command),
                status: *status,
            })
            .collect(),
    );
}

fn push(join: Join, text: String, status: Option<i32>, duration_ms: i64) {
    let stages = PENDING_STAGES.with(|s| std::mem::take(&mut *s.borrow_mut()));
    SEGMENTS.with(|s| {
        let mut list = s.borrow_mut();
        let index = list.len();
        list.push(Segment {
            index,
            join,
            text,
            status,
            duration_ms,
            stages,
        });
    });
}

/// The links of the last line run, in order.
pub fn taken() -> Vec<Segment> {
    SEGMENTS.with(|s| s.borrow().clone())
}

/// Whether the last line was a chain at all. One link is just a command.
pub fn was_a_chain() -> bool {
    SEGMENTS.with(|s| s.borrow().len() > 1)
}

/// The last chain that ran, which is what `chain` reports on.
///
/// Not the live buffer: see [`disarm`]. Asking about a chain means typing a command, and that
/// command has already cleared the live one by the time it can look.
pub fn last_chain() -> Vec<Segment> {
    LAST.with(|last| last.borrow().clone())
}

/// The first link that ran and failed, and everything from there on.
///
/// `None` when the line succeeded, when nothing ran, or when it was not a chain — in each case
/// there is nothing to resume *from*, which is different from having nothing to resume.
pub fn resumable() -> Option<String> {
    rebuild_from(&taken())
}

/// [`resumable`] for the last *chain* rather than the last line.
///
/// What `chain resume` needs: by the time it runs, the live buffer holds `chain resume` itself.
/// See [`disarm`].
pub fn last_resumable() -> Option<String> {
    rebuild_from(&last_chain())
}

fn rebuild_from(segments: &[Segment]) -> Option<String> {
    if segments.len() < 2 {
        return None;
    }
    let failed = segments
        .iter()
        .position(|s| s.status.is_some_and(|status| status != 0))?;
    let mut line = String::new();
    for segment in &segments[failed..] {
        if !line.is_empty() {
            line.push(' ');
            line.push_str(segment.join.written());
            line.push(' ');
        }
        line.push_str(&segment.text);
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each test owns the thread-local, so they must not run against a dirty one.
    fn fresh() {
        arm();
    }

    #[test]
    fn a_chain_that_worked_has_nothing_to_resume() {
        fresh();
        record(Join::First, "a".into(), 0, 1);
        record(Join::And, "b".into(), 0, 1);
        assert_eq!(resumable(), None);
    }

    /// The point of the whole file: the failed link and everything after it, including the links
    /// that never ran.
    #[test]
    fn a_broken_chain_resumes_from_the_link_that_failed() {
        fresh();
        record(Join::First, "make clean".into(), 0, 5);
        record(Join::And, "make build".into(), 1, 400);
        record_skipped(Join::And, "make test".into());
        assert_eq!(resumable().as_deref(), Some("make build && make test"));
    }

    /// A skipped link is not a successful one. Without this the chain above would look like it
    /// finished.
    #[test]
    fn a_skipped_link_is_distinguishable_from_one_that_worked() {
        fresh();
        record(Join::First, "a".into(), 1, 1);
        record_skipped(Join::And, "b".into());
        let segments = taken();
        assert!(segments[0].ran());
        assert!(
            !segments[1].ran(),
            "b never ran and must not read as success"
        );
    }

    /// One command is not a chain, so a failure is just a failure.
    #[test]
    fn a_single_command_is_not_resumable() {
        fresh();
        record(Join::First, "false".into(), 1, 1);
        assert_eq!(resumable(), None);
        assert!(!was_a_chain());
    }

    /// `||` is rendered as written, or the resumed line would mean something else entirely.
    #[test]
    fn the_operator_is_kept_when_the_line_is_rebuilt() {
        fresh();
        record(Join::First, "a".into(), 0, 1);
        record(Join::And, "b".into(), 1, 1);
        record_skipped(Join::Or, "c".into());
        assert_eq!(resumable().as_deref(), Some("b || c"));
    }

    /// Nested chains do not record: `enter` says no while an outer one holds the buffer.
    #[test]
    fn only_the_outermost_chain_records() {
        fresh();
        let outer = enter();
        assert!(outer);
        assert!(!enter(), "a nested chain must not claim the buffer");
        leave(outer);
        assert!(enter(), "and the next top-level chain may");
    }

    /// A shell that was never armed records nothing at all — a script has no prompt to offer a
    /// resume at.
    #[test]
    fn nothing_is_recorded_when_the_repl_did_not_ask() {
        disarm();
        assert!(!enter());
    }
}
