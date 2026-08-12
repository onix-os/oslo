use oslo_ui::suggest::{Ctx, ask, forget, names};

/// Run `source` on a real engine, so what is tested is the whole path: `api::install` putting
/// `provider` into `oslo.suggest`, the declaration, and `engine::call_here` finding the interpreter
/// again when the editor asks. A hand-built table would test none of those — and testing against one
/// is how the first version of this file passed while the feature did not work at all.
fn declare(source: &str) -> Result<(), String> {
    forget();
    let engine = crate::LuaEngine::new().map_err(|e| e.to_string())?;
    let env = std::sync::Arc::new(std::sync::Mutex::new(oslo_shell::env::Environment::new()));
    engine.setup_bindings(env).map_err(|e| e.to_string())?;
    engine
        .eval_as(source, "suggest test")
        .map_err(|e| e.to_string())
}

fn ctx(line: &str) -> Ctx {
    Ctx {
        line: line.to_string(),
        cursor: line.len(),
        cwd: "/w".to_string(),
        language: "sh".to_string(),
    }
}

#[test]
fn a_declared_provider_answers_the_remainder() {
    declare(
        r#"oslo.suggest.provider {
             name = "tldr",
             answer = function(ctx) return "git commit --amend" end,
           }"#,
    )
    .expect("declares");
    assert_eq!(names(), vec!["tldr".to_string()]);
    assert_eq!(ask(&ctx("git com")), Some("mit --amend".to_string()));
    forget();
}

/// The context is what a provider answers from, so every field has to arrive.
#[test]
fn the_context_carries_the_line_the_cursor_and_the_language() {
    declare(
        r#"oslo.suggest.provider {
             name = "spy",
             answer = function(ctx)
               return ctx.line .. "|" .. ctx.cursor .. "|" .. ctx.language .. "|" .. ctx.cwd
             end,
           }"#,
    )
    .expect("declares");
    // The answer has to continue the line to be drawn at all, and it does — so what comes back is
    // everything after it, which is the rest of the context.
    assert_eq!(
        ask(&ctx("git st")),
        Some("|6|sh|/w".to_string()),
        "line, cursor, language and cwd all reached the provider"
    );
    forget();
}

#[test]
fn returning_nothing_is_a_decline_rather_than_an_error() {
    declare(r#"oslo.suggest.provider { name = "quiet", answer = function() return nil end }"#)
        .expect("declares");
    assert_eq!(ask(&ctx("git st")), None);
    forget();
}

/// A provider that raises must not take the keystroke with it.
#[test]
fn a_provider_that_raises_declines_instead_of_failing_the_frame() {
    declare(r#"oslo.suggest.provider { name = "broken", answer = function() error("no") end }"#)
        .expect("declares");
    assert_eq!(ask(&ctx("git st")), None);
    forget();
}

/// Anything that is not a string is a decline: a number cannot be drawn after the line.
#[test]
fn a_non_string_answer_is_a_decline() {
    declare(r#"oslo.suggest.provider { name = "odd", answer = function() return 42 end }"#)
        .expect("declares");
    assert_eq!(ask(&ctx("git st")), None);
    forget();
}

#[test]
fn a_declaration_missing_its_parts_is_a_mistake_worth_raising() {
    let problem = declare(r#"oslo.suggest.provider { answer = function() end }"#).unwrap_err();
    assert!(problem.contains("name"), "{problem}");

    let problem = declare(r#"oslo.suggest.provider { name = "x" }"#).unwrap_err();
    assert!(problem.contains("answer"), "{problem}");

    let problem = declare(r#"oslo.suggest.provider("tldr")"#).unwrap_err();
    assert!(problem.contains("table"), "{problem}");
    forget();
}

#[test]
fn the_names_come_back_in_the_order_they_are_asked() {
    declare(
        r#"oslo.suggest.provider { name = "first",  answer = function() return nil end }
           oslo.suggest.provider { name = "second", answer = function() return nil end }"#,
    )
    .expect("declares");
    assert_eq!(names(), vec!["first".to_string(), "second".to_string()]);
    forget();
}
