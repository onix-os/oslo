//! A column that no stage can be carrying is refused **before any stage runs**.
//!
//! `data::plan` decides which channel every edge carries before anything starts. The declaration it
//! read stopped at the shape — *takes rows, gives rows* — so a mistyped column name was caught only
//! by `tools::unknown_column` scanning rows that had already been produced. `data::columns` carries
//! the other half of the declaration, and this is the behaviour that falls out of it.

mod common;

use std::path::Path;

fn oslo(script: &str) -> common::Run {
    common::run_in(Path::new("."), script)
}

/// **The case the runtime check cannot see.** `unknown_column` returns early on an empty stream —
/// no rows say nothing about which columns exist — so `ls <empty> | cols nmae` printed nothing and
/// answered 0. The declaration knows what `ls` produces whether or not it produced any.
#[test]
fn an_empty_stream_still_refuses_a_column_that_cannot_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let empty = dir.path().join("empty");
    std::fs::create_dir(&empty).expect("mkdir");

    let run = oslo(&format!("ls {} | cols nmae", empty.display()));
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("nmae"),
        "the message names the column: {}",
        run.stderr
    );
}

/// The everyday case, and the reason it is worth moving earlier: nothing runs.
#[test]
fn a_mistyped_column_is_refused_and_the_producer_does_not_run() {
    let run = oslo("ls | cols nmae");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.out().trim().is_empty(),
        "no rows escape a refusal, got {:?}",
        run.out()
    );
}

/// Every verb that names a column the stream must already have.
#[test]
fn each_verb_that_names_a_column_is_checked() {
    for line in [
        "ls | cols nmae",
        "ls | get nmae",
        "ls | reject nmae",
        "ls | rename nmae x",
        "ls | sort-by nmae",
        "ls | sort-by -r nmae",
        "ls | group-by nmae",
        "ls | stats nmae",
        "ls | histogram nmae",
        "ls | distinct nmae",
        "ls | compact nmae",
        "ls | update nmae 1",
    ] {
        let run = oslo(line);
        assert_eq!(run.status, 2, "`{line}` was not refused: {}", run.stderr);
    }
}

/// **Nothing may be refused on an `Unknown`.** The rule the design rests on: a stream whose columns
/// come from the data is not judged here, and the runtime check still catches the mistake.
#[test]
fn a_data_driven_stream_is_not_judged_early() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.json");
    std::fs::write(&file, r#"[{"a":1}]"#).expect("write");

    // Caught, but by `unknown_column` once the rows exist — the point is that it is still caught.
    let run = oslo(&format!("cat {} | from json | cols nmae", file.display()));
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);

    // And a real column of that stream goes through untouched.
    let good = oslo(&format!("cat {} | from json | cols a", file.display()));
    assert_eq!(good.status, 0, "stderr: {}", good.stderr);
    assert_eq!(good.out().trim(), "1");
}

/// **`parse` says what it produces**, so a pipeline over `/etc/passwd` is judged before a byte of it
/// is read.
#[test]
fn parse_declares_its_columns_from_its_pattern() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("pw");
    std::fs::write(&file, "bo:x:1000:staff\n").expect("write");

    let bad = oslo(&format!(
        "cat {} | parse '{{user}}:{{x}}:{{uid}}:{{group}}' | cols nmae",
        file.display()
    ));
    assert_eq!(bad.status, 2, "stderr: {}", bad.stderr);

    let good = oslo(&format!(
        "cat {} | parse '{{user}}:{{x}}:{{uid}}:{{group}}' | cols user group",
        file.display()
    ));
    assert_eq!(good.status, 0, "stderr: {}", good.stderr);
    assert_eq!(good.out().trim(), "bo\tstaff");
}

/// A column a verb *created* is a column the next stage may name.
#[test]
fn the_algebra_follows_the_pipeline() {
    let run = oslo("ls | insert kb 'size / 1024' | sort-by kb | cols name kb | first 1");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(!run.out().trim().is_empty());

    // And a column a verb removed is not.
    let gone = oslo("ls | reject size | sort-by size");
    assert_eq!(gone.status, 2, "stderr: {}", gone.stderr);
}

/// A renamed column answers to its new name and not its old one.
#[test]
fn a_rename_moves_the_name_the_next_stage_may_use() {
    let ok = oslo("ls | rename size bytes | sort-by bytes | first 1 | cols name");
    assert_eq!(ok.status, 0, "stderr: {}", ok.stderr);

    let stale = oslo("ls | rename size bytes | sort-by size");
    assert_eq!(stale.status, 2, "stderr: {}", stale.stderr);
}

/// **A word that is not a literal is not judged.** It is unknown until it runs, which is the same
/// rule the planner already follows for a command name that comes out of an expansion.
#[test]
fn an_expanded_operand_is_left_to_the_runtime_check() {
    // The column is good, and reaching it through a variable must not be refused.
    let run = oslo("c=name; ls | cols $c | first 1");
    assert_eq!(
        run.status, 0,
        "an expanded operand must not be refused: {}",
        run.stderr
    );
    assert!(!run.out().trim().is_empty());
}

/// An opaque verb ends what is known, and everything after it goes unjudged rather than wrongly
/// judged.
#[test]
fn knowledge_ends_at_an_opaque_verb() {
    // `map` answers whatever the Lua did, so `cols` after it is the runtime's problem — and a
    // column that really is there must go through.
    let run = oslo("ls | map '{ n = name }' | cols n | first 1");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(!run.out().trim().is_empty());
}

/// **Quoting is not expansion, and treating it as such made this whole pass nearly inert.**
///
/// The plan-time check read a bare literal and nothing else, so `where 'size > 100'` — the spelling
/// every example uses — counted as unknown: the stage was skipped *and* the column set went
/// `Unknown` for everything after it. So the typo below was caught only once the rows existed, by
/// which time `ls` had run and the status was the last stage's.
#[test]
fn a_quoted_operand_does_not_blind_the_column_contract() {
    let dir = tempfile::tempdir().expect("tempdir");

    for filter in ["where \"true\"", "where 'true'"] {
        let run = common::run_in(dir.path(), &format!("ls | {filter} | cols nmae | length"));
        assert_eq!(
            run.status,
            2,
            "`{filter}` left the typo to be found at runtime: {} {}",
            run.out(),
            run.stderr
        );
        assert!(
            run.out().is_empty(),
            "nothing should have run: {:?}",
            run.out()
        );
    }
}

/// A word that really is unknowable until it runs still defers, which is the line this must not
/// cross: refusing what a variable *might* name would refuse working pipelines.
#[test]
fn an_expanded_operand_is_still_left_to_the_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = common::run_in(dir.path(), "c=nmae\nls | cols $c | length");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.out(), "0", "the refusal came from the rows, as before");
}

/// **`insert` refuses a column that already exists, before anything runs** — the mirror of the
/// check above, and the same words `assign` uses, so the two differ only in when they notice.
#[test]
fn insert_over_an_existing_column_is_refused_at_plan_time() {
    let dir = tempfile::tempdir().expect("tempdir");

    let run = common::run_in(dir.path(), "printf 'a\\n' | lines | insert line 1 | length");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("already a column"),
        "stderr: {}",
        run.stderr
    );
    assert!(run.out().is_empty(), "nothing ran: {:?}", run.out());

    // A column that is genuinely new is not refused.
    let fine = common::run_in(dir.path(), "printf 'a\\n' | lines | insert kb 1 | length");
    assert_eq!(fine.out(), "1", "stderr: {}", fine.stderr);
}
