use oslo_ui::completion::provider::{Ctx, forget, names, offers};

/// A real engine, so what is tested is `api::install` wiring the table and `call_here` finding the
/// interpreter again when Tab is pressed.
fn declare(source: &str) -> Result<(), String> {
    forget();
    let engine = crate::LuaEngine::new().map_err(|e| e.to_string())?;
    let env = std::sync::Arc::new(std::sync::Mutex::new(oslo_shell::env::Environment::new()));
    engine.setup_bindings(env).map_err(|e| e.to_string())?;
    engine
        .eval_as(source, "completion test")
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn ctx(command: &str, current: &str) -> Ctx {
    Ctx {
        command: command.to_string(),
        words: vec![command.to_string()],
        current: current.to_string(),
        arg: 1,
        cwd: "/w".to_string(),
    }
}

#[test]
fn a_declared_provider_offers_candidates_with_descriptions() {
    declare(
        r#"oslo.completion.provider {
             name = "tldr", kind = "example",
             answer = function(ctx)
               return { { display = "git commit --amend", desc = "change the last commit" } }
             end,
           }"#,
    )
    .expect("declares");
    assert_eq!(names(), vec!["tldr".to_string()]);

    let out = offers(&ctx("git", ""));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0.display, "git commit --amend");
    assert_eq!(
        out[0].0.description.as_deref(),
        Some("change the last commit")
    );
    assert_eq!(out[0].0.kind.as_deref(), Some("example"));
    forget();
}

/// A bare string is an offer with nothing to say about it — the simple case stays simple.
#[test]
fn a_list_of_strings_is_a_list_of_offers() {
    declare(
        r#"oslo.completion.provider {
             name = "plain",
             answer = function() return { "one", "two" } end,
           }"#,
    )
    .expect("declares");
    let out: Vec<String> = offers(&ctx("git", ""))
        .into_iter()
        .map(|(c, _)| c.display)
        .collect();
    assert_eq!(out, vec!["one".to_string(), "two".to_string()]);
    forget();
}

/// Declaring no kind is still a kind: its own name, so `sources` can name it either way.
#[test]
fn a_provider_without_a_kind_is_badged_with_its_name() {
    declare(r#"oslo.completion.provider { name = "tldr", answer = function() return {"x"} end }"#)
        .expect("declares");
    assert_eq!(offers(&ctx("git", ""))[0].0.kind.as_deref(), Some("tldr"));
    forget();
}

#[test]
fn when_limits_a_provider_to_one_command() {
    declare(
        r#"oslo.completion.provider {
             name = "gitonly", when = "git",
             answer = function() return {"git status"} end,
           }"#,
    )
    .expect("declares");
    assert_eq!(offers(&ctx("git", "")).len(), 1);
    assert!(offers(&ctx("ls", "")).is_empty());
    forget();
}

#[test]
fn the_offset_and_the_cap_are_read() {
    declare(
        r#"oslo.completion.provider {
             name = "many", score_offset = 20, max_items = 2,
             answer = function() return { "a", "b", "c", "d" } end,
           }"#,
    )
    .expect("declares");
    let out = offers(&ctx("git", ""));
    assert_eq!(out.len(), 2, "capped");
    assert_eq!(out[0].1, 20.0, "and nudged");
    forget();
}

/// The context is what a provider answers from.
#[test]
fn the_context_carries_the_command_and_the_word() {
    declare(
        r#"oslo.completion.provider {
             name = "spy",
             answer = function(ctx)
               return { ctx.command .. "|" .. ctx.current .. "|" .. ctx.arg .. "|" .. #ctx.words }
             end,
           }"#,
    )
    .expect("declares");
    assert_eq!(offers(&ctx("git", "com"))[0].0.display, "git|com|1|1");
    forget();
}

/// A provider that raises loses its own candidates and nothing else.
#[test]
fn a_provider_that_raises_offers_nothing() {
    declare(r#"oslo.completion.provider { name = "broken", answer = function() error("no") end }"#)
        .expect("declares");
    assert!(offers(&ctx("git", "")).is_empty());
    forget();
}

#[test]
fn a_declaration_missing_its_parts_is_a_mistake_worth_raising() {
    let problem = declare(r#"oslo.completion.provider { answer = function() end }"#).unwrap_err();
    assert!(problem.contains("name"), "{problem}");
    let problem = declare(r#"oslo.completion.provider { name = "x" }"#).unwrap_err();
    assert!(problem.contains("answer"), "{problem}");
    forget();
}
