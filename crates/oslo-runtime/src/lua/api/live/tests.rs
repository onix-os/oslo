use super::*;

fn shell() -> Arc<Mutex<Environment>> {
    Arc::new(Mutex::new(Environment::new()))
}

fn call(name: &str, args: &[Json]) -> Result<Vec<Json>, String> {
    dispatch(name, args, &shell(), Duration::from_millis(50))
}

/// **The list is the contract.** Every name in [`VERBS`] dispatches, and the reverse — a verb that
/// is callable and unlisted is one nobody can discover, and one listed and uncallable is a lie the
/// client library repeats.
#[test]
fn every_listed_verb_dispatches() {
    for (name, _) in VERBS {
        let args = match *name {
            "env.get" | "macros.get" | "notify" => vec![Json::String("X".into())],
            "env.set" => vec![Json::String("X".into()), Json::String("y".into())],
            _ => Vec::new(),
        };
        let answered = call(name, &args);
        assert!(
            answered.is_ok(),
            "{name} is listed and did not dispatch: {answered:?}"
        );
    }
}

/// A name that is not a verb is refused **by name**, so a client written against a newer oslo finds
/// out rather than reading a `nil` it cannot tell from an empty answer.
#[test]
fn an_unknown_verb_is_refused_by_name() {
    let why = call("env.destroy", &[]).expect_err("must be refused");
    assert!(why.contains("env.destroy"), "{why}");
    assert!(
        why.contains("--verbs"),
        "and says how to find the real ones: {why}"
    );
}

/// **Nothing on this surface runs anything.** The check is mechanical because the rule is the whole
/// security argument: a socket that executes what a caller sends is remote code execution on
/// somebody's session, and a later verb must not be able to slip one in unnoticed.
#[test]
fn no_verb_executes_anything() {
    for (name, _) in VERBS {
        for forbidden in ["run", "exec", "eval", "source", "spawn", "system"] {
            assert!(
                !name.split('.').any(|part| part == forbidden),
                "{name} looks like it runs something — see the module header before adding it"
            );
        }
    }
    for candidate in ["run", "eval", "exec", "source"] {
        assert!(
            call(candidate, &[Json::String("echo hi".into())]).is_err(),
            "{candidate} must not be a verb"
        );
    }
}

/// A verb that wants a string and is handed nothing says which argument, rather than answering as
/// though it had been given an empty one.
#[test]
fn a_missing_argument_is_named() {
    let why = call("env.get", &[]).expect_err("must be refused");
    assert!(why.contains("argument 1"), "{why}");

    let why = call("env.set", &[Json::String("ONLY".into())]).expect_err("must be refused");
    assert!(why.contains("argument 2"), "{why}");
}

/// The environment round-trips through the surface, which is the case the whole thing exists for.
#[test]
fn a_variable_set_through_the_surface_is_readable_through_it() {
    let env = shell();
    let wait = Duration::from_millis(50);

    dispatch(
        "env.set",
        &[
            Json::String("OSLO_T_LIVE".into()),
            Json::String("here".into()),
        ],
        &env,
        wait,
    )
    .expect("set");

    let got = dispatch("env.get", &[Json::String("OSLO_T_LIVE".into())], &env, wait).expect("get");
    assert_eq!(got.first().and_then(Json::as_str), Some("here"));

    let missing = dispatch(
        "env.get",
        &[Json::String("OSLO_T_ABSENT".into())],
        &env,
        wait,
    )
    .expect("get");
    assert_eq!(
        missing.first(),
        Some(&Json::Null),
        "absent is null, not empty"
    );
}

/// **Binding must not need the environment lock**, because the two places that turn serving on both
/// already hold it: a key handler runs inside the line editor, and a config line runs where
/// `borrow_env` is live. A `serve()` that waited for the lock would deadlock the shell that asked
/// for it — the failure would look like the terminal freezing on a keypress.
#[test]
fn serving_can_be_switched_on_while_the_environment_is_locked() {
    let env = shell();
    let held = env.lock().expect("lock");

    // Nothing is asserted about success: `$XDG_RUNTIME_DIR` may be absent on a builder. What
    // matters is that the call *returns* rather than parking on the mutex this test is holding.
    let answered = server::start(&env);
    drop(held);

    if answered.is_ok() {
        assert!(
            server::serving().is_some(),
            "it said yes and is not serving"
        );
        assert!(server::stop(), "and stopping must undo it");
    }
    assert!(server::serving().is_none(), "nothing should be left bound");
}

/// The snapshot is a fallback, so it must not exist in a shell that never served — it is a copy of
/// the environment and there is no reason for a quiet shell to hold one.
#[test]
fn a_shell_that_never_served_holds_no_snapshot() {
    forget();
    let env = shell();
    publish(&env);
    assert!(
        SNAPSHOT.read().expect("read").is_none(),
        "publish must do nothing at all unless a socket is bound"
    );
}

/// A reply says whether it worked in a field a client reads before anything else.
#[test]
fn a_reply_frames_success_and_failure_apart() {
    let good = Reply::ok(vec![Json::String("x".into())]);
    assert!(good.contains("\"ok\":true"), "{good}");
    assert!(good.contains("\"n\":1"), "and how many values: {good}");

    let bad = Reply::failed("no");
    assert!(bad.contains("\"ok\":false"), "{bad}");
    assert!(bad.contains("\"error\":\"no\""), "{bad}");
}

/// The client library is carried in the binary and is plain Lua — it has to load in a VM that has
/// none of oslo's own globals.
#[test]
fn the_client_library_is_self_contained() {
    assert!(
        CLIENT.contains("function M.connect"),
        "it must offer connect"
    );
    assert!(
        !CLIENT.contains("require("),
        "it runs in foreign VMs and may require nothing"
    );
    // Every verb the client spells has to be one the server answers.
    for (name, _) in VERBS {
        assert!(
            CLIENT.contains(&format!("\"{name}\"")),
            "the client library does not offer {name}"
        );
    }
}
