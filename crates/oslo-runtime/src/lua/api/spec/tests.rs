use super::*;
use oslo_lua::value::Value;

/// Build the Lua table a spec is declared with, by running the source through the interpreter — so
/// what is tested is the shape somebody types, not a hand-built table that happens to parse.
fn declare(source: &str) -> Result<(), String> {
    oslo_ui::spec::custom::forget();
    let interp = oslo_lua::Interp::new("spec test");
    let mut completion = Table::new();
    install(&mut completion);
    let mut oslo = Table::new();
    oslo.set(Value::str("completion"), Value::table(completion));
    interp.set_global("oslo", Value::table(oslo));
    let ast = oslo_lua::parse(source).map_err(|e| e.to_string())?;
    interp.run_ast(&ast).map(|_| ()).map_err(|e| e.to_string())
}

#[test]
fn a_declared_spec_is_found_under_its_command() {
    declare(
        r#"oslo.completion.spec {
             command = "notes",
             desc = "notes kept in the shell",
             subcommands = {
               { name = "new", desc = "start one" },
               { name = "list", desc = "every note",
                 flags = { { "--since", desc = "only newer than" } } },
             },
             flags = { { "-v", "--verbose", desc = "say more" } },
           }"#,
    )
    .expect("the spec declares");

    let spec = oslo_ui::spec::custom::find("notes").expect("registered");
    assert_eq!(spec.description, "notes kept in the shell");
    assert_eq!(spec.subcommands.len(), 2);
    assert_eq!(spec.subcommands[0].name, "new");
    // Both spellings of a flag, in the order written, from the array part of the table.
    assert_eq!(spec.options[0].names, vec!["-v", "--verbose"]);
    assert_eq!(spec.options[0].description, "say more");
    // A flag on a subcommand rather than on the command.
    assert_eq!(spec.subcommands[1].options[0].names, vec!["--since"]);
    oslo_ui::spec::custom::forget();
}

/// `docker compose up` — two levels down, which is the case `for_command` makes hardest.
#[test]
fn subcommands_nest() {
    declare(
        r#"oslo.completion.spec {
             command = "outer",
             subcommands = { { name = "middle",
               subcommands = { { name = "inner", desc = "the deep one" } } } },
           }"#,
    )
    .expect("the spec declares");
    let spec = oslo_ui::spec::custom::find("outer").unwrap();
    assert_eq!(
        spec.subcommands[0].subcommands[0].description,
        "the deep one"
    );
    oslo_ui::spec::custom::forget();
}

#[test]
fn a_spec_with_no_command_is_a_mistake_worth_raising() {
    let problem = declare(r#"oslo.completion.spec { desc = "no command" }"#).unwrap_err();
    assert!(problem.contains("command"), "{problem}");
    let problem = declare(r#"oslo.completion.spec("notes")"#).unwrap_err();
    assert!(problem.contains("table"), "{problem}");
}

/// **One malformed entry costs that entry.** A spec is often generated, and a list where the third
/// item came out wrong should still complete the other nine.
#[test]
fn entries_that_are_not_usable_are_skipped_rather_than_fatal() {
    declare(
        r#"oslo.completion.spec {
             command = "mixed",
             subcommands = { { name = "good" }, "not a table", { desc = "no name" } },
             flags = { { desc = "no names at all" }, { "--real" } },
           }"#,
    )
    .expect("the spec declares");
    let spec = oslo_ui::spec::custom::find("mixed").unwrap();
    assert_eq!(spec.subcommands.len(), 1, "only the one with a name");
    assert_eq!(spec.options.len(), 1, "only the one with a spelling");
    assert_eq!(spec.options[0].names, vec!["--real"]);
    // A description nobody wrote is empty rather than absent: every candidate carries one.
    assert_eq!(spec.subcommands[0].description, "");
    oslo_ui::spec::custom::forget();
}

/// `name =` for a flag, which is what somebody types after reading the `subcommands` shape.
#[test]
fn a_flag_may_be_named_the_long_way_round() {
    declare(r#"oslo.completion.spec { command = "c", flags = { { name = "--only" } } }"#)
        .expect("the spec declares");
    let spec = oslo_ui::spec::custom::find("c").unwrap();
    assert_eq!(spec.options[0].names, vec!["--only"]);
    oslo_ui::spec::custom::forget();
}
