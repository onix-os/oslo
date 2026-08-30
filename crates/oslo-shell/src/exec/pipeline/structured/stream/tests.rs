use super::*;
use crate::data::Val;

fn rows(values: &[i64]) -> Vec<Record> {
    values
        .iter()
        .map(|n| Record::from_pairs([("n", Val::Int(*n))]))
        .collect()
}

fn numbers(rows: &[Record]) -> Vec<i64> {
    rows.iter()
        .filter_map(|row| match row.get("n") {
            Some(Val::Int(n)) => Some(*n),
            _ => None,
        })
        .collect()
}

fn words(text: &[&str]) -> Vec<String> {
    text.iter().map(|w| w.to_string()).collect()
}

/// **`first n` across batches, which is the whole reason these four are not treated as row-local.**
/// Applied per batch it would take `n` rows out of *each* one — a wrong answer, and a quiet one.
#[test]
fn first_counts_across_batches_and_then_stops() {
    let mut state = Counted::default();
    let (batch, done) = counted("first", &words(&["first", "3"]), rows(&[1, 2]), &mut state);
    assert_eq!(numbers(&batch), [1, 2]);
    assert!(!done, "two of three: still hungry");

    let (batch, done) = counted(
        "first",
        &words(&["first", "3"]),
        rows(&[3, 4, 5]),
        &mut state,
    );
    assert_eq!(numbers(&batch), [3], "only the third row, not three more");
    assert!(done, "and now the upstream can be let go");
}

/// A `first` satisfied inside one batch says so at once, which is what closes the pipe on `yes`.
#[test]
fn first_finishes_within_a_batch() {
    let mut state = Counted::default();
    let (batch, done) = counted(
        "first",
        &words(&["first", "2"]),
        rows(&[1, 2, 3, 4]),
        &mut state,
    );
    assert_eq!(numbers(&batch), [1, 2]);
    assert!(done);
}

/// `skip` counts what it has dropped, so the rows it owes are not dropped again next batch.
#[test]
fn skip_carries_its_debt() {
    let mut state = Counted::default();
    let (batch, done) = counted("skip", &words(&["skip", "3"]), rows(&[1, 2]), &mut state);
    assert!(batch.is_empty(), "both owed to the skip");
    assert!(!done);

    let (batch, _) = counted("skip", &words(&["skip", "3"]), rows(&[3, 4, 5]), &mut state);
    assert_eq!(numbers(&batch), [4, 5], "one more owed, then the rest");
}

/// `every n` keeps its place in the cycle, so a batch boundary is not a phase reset.
#[test]
fn every_keeps_its_phase_across_batches() {
    let mut state = Counted::default();
    let (batch, _) = counted(
        "every",
        &words(&["every", "2"]),
        rows(&[0, 1, 2]),
        &mut state,
    );
    assert_eq!(numbers(&batch), [0, 2]);

    // The next batch begins at position 3, which is odd, so its first row is skipped.
    let (batch, _) = counted(
        "every",
        &words(&["every", "2"]),
        rows(&[3, 4, 5]),
        &mut state,
    );
    assert_eq!(
        numbers(&batch),
        [4],
        "not 3, which the cycle has already passed"
    );
}

/// **`enumerate` counts the stream, not the batch.** Restarting at zero each batch is the quiet
/// wrong answer this state exists to prevent.
#[test]
fn enumerate_keeps_counting() {
    let mut state = Counted::default();
    let (batch, _) = counted(
        "enumerate",
        &words(&["enumerate"]),
        rows(&[10, 11]),
        &mut state,
    );
    let index = |r: &Record| match r.get("index") {
        Some(Val::Int(n)) => *n,
        _ => -1,
    };
    assert_eq!(index(&batch[0]), 0);
    assert_eq!(index(&batch[1]), 1);
    // The index leads, as the materialised verb puts it.
    assert_eq!(batch[0].columns(), ["index", "n"]);

    let (batch, _) = counted("enumerate", &words(&["enumerate"]), rows(&[12]), &mut state);
    assert_eq!(
        index(&batch[0]),
        2,
        "the third row of the stream, not the first of a batch"
    );
}

/// A count that is not a whole number falls back to one, as the materialised verbs do rather than
/// wrapping or panicking.
#[test]
fn a_missing_count_is_one() {
    let mut state = Counted::default();
    let (batch, done) = counted("first", &words(&["first"]), rows(&[1, 2, 3]), &mut state);
    assert_eq!(numbers(&batch), [1]);
    assert!(done);
}

/// **A fold answers once, at the end** — and holds only its bound while it waits. `final 2` keeps
/// two rows however many go past, which is what separates it from `sort-by`, which cannot stream at
/// all.
#[test]
fn final_keeps_a_window_and_answers_at_the_end() {
    let mut state = Counted::default();

    let batch = folded(
        "final",
        &words(&["final", "2"]),
        rows(&[1, 2, 3]),
        &mut state,
        false,
    );
    assert!(batch.is_empty(), "nothing flows past a fold mid-stream");
    assert_eq!(
        state.kept.len(),
        2,
        "the window is the bound, not the stream"
    );

    let batch = folded(
        "final",
        &words(&["final", "2"]),
        rows(&[4, 5]),
        &mut state,
        false,
    );
    assert!(batch.is_empty());
    assert_eq!(state.kept.len(), 2, "still two, after five rows");

    let batch = folded(
        "final",
        &words(&["final", "2"]),
        Vec::new(),
        &mut state,
        true,
    );
    assert_eq!(numbers(&batch), [4, 5], "the last two of the whole stream");
}

/// `length` holds a counter and nothing else, which is what makes it answerable for an upstream
/// far larger than the materialised path could hold.
#[test]
fn length_counts_the_stream_and_answers_once() {
    let mut state = Counted::default();

    assert!(
        folded(
            "length",
            &words(&["length"]),
            rows(&[1, 2, 3]),
            &mut state,
            false
        )
        .is_empty(),
        "no answer until the end"
    );
    folded(
        "length",
        &words(&["length"]),
        rows(&[4, 5]),
        &mut state,
        false,
    );
    assert!(state.kept.is_empty(), "a count holds no rows at all");

    let batch = folded("length", &words(&["length"]), Vec::new(), &mut state, true);
    assert_eq!(batch.len(), 1, "one row, as the materialised verb answers");
    assert_eq!(batch[0].get("length"), Some(&Val::Int(5)));
}

/// A fold that never saw a row still answers, because "none" is an answer and an empty stream is
/// the ordinary way to get it.
#[test]
fn length_of_an_empty_stream_is_zero() {
    let mut state = Counted::default();
    let batch = folded("length", &words(&["length"]), Vec::new(), &mut state, true);
    assert_eq!(batch[0].get("length"), Some(&Val::Int(0)));
}
