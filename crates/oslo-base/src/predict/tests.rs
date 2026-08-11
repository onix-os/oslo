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

/// A model written and read back predicts the same things.
///
/// This is what keeps the model off the startup path: if a snapshot did not round-trip, the only
/// way to have a model would be to replay history at every prompt, which is the cost the whole
/// design exists to avoid.
#[test]
fn a_snapshot_round_trips() {
    let before = trained();
    let mut bytes = Vec::new();
    before.save(&mut bytes).expect("write");
    let after = Model::load(&bytes[..]).expect("read back what was written");
    assert_eq!(
        before.next(1, 7, Some("cargo"), 3),
        after.next(1, 7, Some("cargo"), 3),
        "a model is not itself after a round trip"
    );
}

/// On disk it is owner-only and never half-written.
#[test]
fn the_snapshot_file_is_private_and_replaced_atomically() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("nested").join("oslo.model");

    trained().save_to(&path).expect("write");
    let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "it holds every command you have run");
    assert!(
        !path.with_extension("model.new").exists(),
        "the scratch file is renamed, not left behind"
    );

    // Written again over a live file, and still readable afterwards.
    trained().save_to(&path).expect("rewrite");
    assert!(Model::load_from(&path).is_some(), "still readable");
}

/// Anything unreadable is simply no model. Losing it costs only learning again.
#[test]
fn an_unusable_snapshot_is_no_model_rather_than_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(Model::load_from(&dir.path().join("absent")).is_none());

    let torn = dir.path().join("torn.model");
    std::fs::write(&torn, b"not a snapshot").expect("write");
    assert!(Model::load_from(&torn).is_none());
}

/// What forgets the history must forget the model, or the shell keeps what it was told to drop.
#[test]
fn the_saved_model_can_be_forgotten() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("oslo.model");
    trained().save_to(&path).expect("write");
    assert!(path.exists());

    forget_saved(&path).expect("forget");
    assert!(!path.exists());
    // Forgetting what is already gone is not an error.
    forget_saved(&path).expect("idempotent");
}

/// It lands beside the history it was learned from, under the profile.
#[test]
fn the_model_is_filed_beside_the_history() {
    let path = default_path(Some("/data"), None).expect("a path");
    assert!(path.starts_with("/data/oslo"), "{path:?}");
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("model"));

    let from_home = default_path(None, Some("/home/u")).expect("a path");
    assert!(
        from_home.starts_with("/home/u/.local/share/oslo"),
        "{from_home:?}"
    );
    // No home and no data directory is a shell that runs without one.
    assert!(default_path(None, None).is_none());
}

/// **A mistyped line must not be in the model, or its own repair goes silent.**
///
/// The measurement behind [`succeeded`], and the reason a *failed* line is not learned rather than
/// only an unrunnable one. The repair everybody wants is of the command they have already run and
/// watched fail — so by the time it is asked for, a model that learns failures has swallowed it.
#[test]
fn a_line_that_failed_is_not_learned() {
    let history = [
        entry(1, 1, "git status --short"),
        entry(1, 2, "git status --short"),
        entry(1, 3, "git status --short"),
    ];
    let typo = "git stauts --short";

    let mut clean = Model::new();
    clean.learn_all(&history);
    assert_eq!(
        clean.repair(1, 4, typo, 3).first().map(|g| g.line.as_str()),
        Some("git status --short"),
        "the repair this whole feature is for"
    );

    let mut polluted = Model::new();
    polluted.learn_all(&history);
    polluted.learn(&entry(1, 4, typo), 4);
    assert!(
        polluted.repair(1, 5, typo, 3).is_empty(),
        "if this ever stops being true, `succeeded` can be relaxed"
    );
}

/// The rule itself. Only a command that worked is worth predicting or aligning against.
#[test]
fn only_a_line_that_worked_is_learned() {
    assert!(succeeded(Some(0)));
    assert!(!succeeded(Some(127)), "no such command");
    assert!(!succeeded(Some(1)), "the typo case, and the whole point");
    assert!(!succeeded(None), "never reached execution");
}

/// The held line is learned at the command boundary, not when it was logged.
#[test]
fn a_line_is_learned_when_its_status_arrives() {
    forget_shared();
    record(&entry(9, 1, "cargo build"), 1);
    assert!(!ready(), "learning before the status is what this prevents");

    settle(Some(0));
    assert!(ready(), "the boundary is what learns it");

    // The position moved on with the line, not with the status: the next prompt asks from it and
    // cannot wait for a command that is still running.
    assert_eq!(position(), (9, 2));
    forget_shared();
}

/// **What just failed is remembered, and a success forgets it.**
///
/// One line rather than a history: a correction older than the last command is not a correction
/// anybody asked for, and offering to fix something that has scrolled off screen is worse than
/// offering nothing.
#[test]
fn the_last_failure_is_remembered_until_something_works() {
    forget_shared();
    record(&entry(8, 1, "ehco alpha"), 1);
    settle(Some(127));
    assert_eq!(last_failed().as_deref(), Some("ehco alpha"));

    record(&entry(8, 2, "echo alpha"), 2);
    settle(Some(0));
    assert_eq!(last_failed(), None, "a success clears it");
    forget_shared();
}
