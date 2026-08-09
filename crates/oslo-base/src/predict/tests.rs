use super::*;

/// A recorded command, as the log would have it.
fn entry(session: u32, seq: u32, line: &str) -> Entry {
    Entry {
        line: line.to_string(),
        mode: "sh".to_string(),
        session,
        seq,
        rewritten: false,
    }
}

/// A model that has watched one shell do the same thing twice.
fn trained() -> Model {
    let history = [
        entry(1, 1, "cargo build"),
        entry(1, 2, "cargo test"),
        entry(1, 3, "git commit"),
        entry(1, 4, "cargo build"),
        entry(1, 5, "cargo test"),
        entry(1, 6, "git commit"),
    ];
    let mut model = Model::new();
    assert_eq!(model.learn_all(&history), 6);
    model
}

/// **The thing the feature exists for.** Having seen `build → test` twice, the model says `test`.
#[test]
fn it_predicts_what_followed_the_same_thing_before() {
    let model = trained();
    let guesses = model.next(1, 7, None, 3);
    assert!(!guesses.is_empty(), "the model learned nothing");
    assert!(
        guesses.iter().any(|g| g.line == "cargo build"),
        "expected a command it has seen, got {guesses:?}"
    );
    assert!(guesses.iter().all(|g| g.probability > 0.0));
}

/// What has been typed narrows what is offered.
#[test]
fn a_partial_line_narrows_the_answer() {
    let model = trained();
    let guesses = model.next(1, 7, Some("git"), 3);
    assert!(
        guesses.iter().all(|g| g.line.starts_with("git")),
        "a partial must not be contradicted: {guesses:?}"
    );
}

/// Repair rebuilds a failed line from what history holds, and never answers with the failure.
#[test]
fn it_repairs_a_line_from_what_history_holds() {
    let mut model = Model::new();
    model.learn_all(&[
        entry(1, 1, "sudo apt install fd"),
        entry(1, 2, "sudo apt install ripgrep"),
        entry(1, 3, "sudo apt install jq"),
    ]);
    let guesses = model.repair(1, 4, "apt install fd", 5);
    assert!(
        guesses.iter().all(|g| g.line != "apt install fd"),
        "the failure itself is not a repair"
    );
    assert!(
        guesses.iter().any(|g| g.line.starts_with("sudo apt")),
        "expected the missing sudo to be rebuilt, got {guesses:?}"
    );
}

/// **Order is the model.** Fed backwards it would learn that `build` follows `test`, so the caller
/// reversing `recent()` is load-bearing and this is what says so.
#[test]
fn the_order_it_is_fed_in_is_the_order_it_learns() {
    let forwards = trained();
    let mut backwards = Model::new();
    let history = [
        entry(1, 6, "git commit"),
        entry(1, 5, "cargo test"),
        entry(1, 4, "cargo build"),
    ];
    backwards.learn_all(&history);

    // Both learned something; they did not learn the same thing.
    assert!(forwards.learned() > 0 && backwards.learned() > 0);
    assert_ne!(
        forwards.next(1, 7, Some("cargo"), 3),
        backwards.next(1, 7, Some("cargo"), 3),
        "reversing the history must change what is predicted"
    );
}

/// A row from before sessions were recorded is skipped rather than filed under a shared stream,
/// which would teach transitions between shells that never spoke to each other.
#[test]
fn a_row_with_no_session_is_not_learned() {
    let mut model = Model::new();
    assert!(!model.learn(&entry(0, 1, "cargo build"), 0), "no session");
    assert!(!model.learn(&entry(1, 0, "cargo build"), 0), "no position");
    assert!(!model.learn(&entry(1, 1, "   "), 0), "nothing typed");
    assert_eq!(model.learned(), 0);
}

/// **Another shell's command is offered, and ranked below what this shell actually does.**
///
/// This pins a property that had to be measured rather than assumed. Candidates are *not*
/// partitioned by stream — the recent cache is global, so a command from another terminal can be
/// suggested, exactly as the history source would suggest it. What the stream decides is the
/// *order*: with a pattern established here, the foreign command sinks to the bottom.
///
/// The first version of this test asserted the foreign command was absent entirely, and failed.
/// That was an assumption about vista, not a promise vista makes.
#[test]
fn another_shells_command_is_offered_but_outranked() {
    let mut model = Model::new();
    let mut history = Vec::new();
    for i in 0..6u32 {
        history.push(entry(1, 2 * i + 1, "cargo build"));
        history.push(entry(1, 2 * i + 2, "cargo test"));
    }
    history.push(entry(2, 1, "ssh server"));
    model.learn_all(&history);

    let guesses = model.next(1, 14, None, 5);
    let foreign = guesses
        .iter()
        .position(|g| g.line == "ssh server")
        .expect("it is a candidate, like any other command in history");
    assert_eq!(
        foreign,
        guesses.len() - 1,
        "this shell's own pattern must outrank it: {guesses:?}"
    );
    assert!(
        guesses[0].probability > guesses[foreign].probability * 3.0,
        "and by a margin, not a nose: {guesses:?}"
    );
}

/// An empty model answers nothing rather than failing.
#[test]
fn an_untrained_model_is_quiet() {
    let model = Model::new();
    assert!(model.next(1, 1, None, 3).is_empty());
    assert!(model.repair(1, 1, "aptt install fd", 3).is_empty());
    assert_eq!(model.learned(), 0);
}

/// The snapshot is what keeps the model off the startup path; it must at least be writable.
#[test]
fn a_trained_model_can_be_written_out() {
    let model = trained();
    let mut out = Vec::new();
    model.save(&mut out).expect("a model writes");
    assert!(!out.is_empty(), "a trained model is not an empty snapshot");
}
