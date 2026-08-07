//! `chain` — what each link of the last `a && b || c` did.
//!
//! The shell already computes this and used to drop it: `eval_and_or_list` keeps one status, the
//! one the last link that ran left behind. `$PIPESTATUS` answers the same question one level down,
//! for the stages inside a single pipeline, and there is no equivalent for the level above it.
//!
//! ```text
//! ❯ make clean && make build && make test
//! ❯ chain
//!    make clean     ok      5ms
//! && make build     failed  412ms
//! && make test      skipped
//! ❯ chain resume
//! make build && make test
//! ```
//!
//! **`skipped` is the row worth having.** A link the chain short-circuited past did not run, which
//! is neither success nor failure, and no shell records the difference — so "where did it stop"
//! has never been answerable from the outside.
//!
//! Interactive only, in the sense that matters: the recorder is armed by the read loop and by
//! nothing else, so in a script this reports an empty chain rather than a stale one.

use crate::env::Environment;
use crate::error::Result;
use crate::exec::pipeline::segments;

pub fn builtin_chain(_env: &mut Environment, args: &[String]) -> Result<i32> {
    match args.get(1).map(String::as_str) {
        None => Ok(report()),
        Some("resume") => Ok(resume()),
        Some("-h" | "--help") => Ok(usage()),
        Some(other) => {
            eprintln!("oslo: chain: {other}: unknown argument");
            Ok(usage())
        }
    }
}

fn usage() -> i32 {
    eprintln!(
        "usage: chain [resume]\n\n\
         With no argument, what each link of the last chain did.\n\
         `resume` prints the chain from the link that failed, to run again."
    );
    2
}

/// One row per link: how it was joined, what it was, and what it did.
fn report() -> i32 {
    let segments = segments::last_chain();
    if segments.is_empty() {
        eprintln!("oslo: chain: nothing has run yet");
        return 1;
    }
    // The config gets first refusal. **Fired from inside a builtin**, so a handler may draw
    // anything it likes but must not reach for shell state — see `ui::report`.
    if drawn_by_config(&segments) {
        return 0;
    }
    // The widest link decides the column, so the outcomes line up however long the commands are.
    let width = segments.iter().map(|s| s.text.len()).max().unwrap_or(0);
    let mut total = 0;
    for segment in &segments {
        let outcome = match segment.status {
            None => "skipped".to_string(),
            Some(0) => "ok".to_string(),
            Some(status) => format!("failed ({status})"),
        };
        // Blank rather than `0ms` for a link that never ran: zero is a measurement, and this is
        // the absence of one.
        let took = if segment.ran() {
            total += segment.duration_ms;
            format!("{}ms", segment.duration_ms)
        } else {
            String::new()
        };
        println!(
            "{:>2} {:<width$}  {:<12} {:>8}",
            segment.join.written(),
            segment.text,
            outcome,
            took
        );
        // The stages of a pipeline, under the link they belong to. **No time against them**: they
        // ran at the same moment as each other, so a wall clock per stage would be the pipeline's
        // own number printed once per stage and read as though each had taken that long.
        for (i, stage) in segment.stages.iter().enumerate() {
            let joined = if i == 0 { "  " } else { " |" };
            let outcome = if stage.status == 0 {
                "ok".to_string()
            } else {
                format!("failed ({})", stage.status)
            };
            println!("   {joined} {:<width$}  {outcome}", stage.text);
        }
    }
    if segments.len() > 1 {
        println!(
            "{:>2} {:<width$}  {:<12} {:>8}",
            "",
            "",
            "total",
            format!("{total}ms")
        );
    }
    0
}

/// Whether an `on-report` handler drew this instead.
///
/// The same `segments` table `pre-record` hands over, so a config that already walks one for a
/// filter walks the same shape here.
fn drawn_by_config(segments: &[segments::Segment]) -> bool {
    use crate::ui::report::{self, int, rows, text};
    if !report::watched() {
        return false;
    }
    let links = rows(
        segments
            .iter()
            .map(|link| {
                let mut row = vec![
                    ("text", text(&link.text)),
                    ("op", text(link.join.written())),
                    ("ran", crate::lua::eval::value::Value::Bool(link.ran())),
                    ("ms", int(link.duration_ms)),
                ];
                // Absent rather than a number when it never ran: any number here reads as a status.
                if let Some(status) = link.status {
                    row.push(("status", int(i64::from(status))));
                }
                row
            })
            .collect(),
    );
    report::handled("chain", vec![("segments", links)])
}

/// The chain from the link that failed onwards, ready to run again.
fn resume() -> i32 {
    match segments::last_resumable() {
        Some(line) => {
            println!("{line}");
            0
        }
        None => {
            eprintln!("oslo: chain: nothing to resume — the last chain did not stop part-way");
            1
        }
    }
}
