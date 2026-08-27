use super::*;
use oslo_base::value::Value;

/// Build the Lua table a spec is declared with, by running the source through the interpreter — so
/// what is tested is the shape somebody types, not a hand-built table that happens to parse.
fn declare(source: &str) -> Result<(), String> {
    oslo_ui::spec::custom::forget();
    let engine = oslo_luavm::Engine::new();
    let mut completion = Table::new();
    install(&mut completion);
    let mut oslo = Table::new();
    oslo.set_str("completion", Value::table(completion));
    oslo_luavm::Host::set_global(&engine, "oslo", Value::table(oslo));
    oslo_luavm::Host::eval(&engine, source, "=spec test")
        .map(|_| ())
        .map_err(|e| e.to_string())
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

/// The carapace flag syntax, as a key. One spelling of a fact both surfaces have to spell alike.
#[test]
fn a_flag_may_be_written_the_way_a_spec_file_writes_one() {
    declare(
        r#"oslo.completion.spec {
             command = "c",
             flags = { ["-f, --file="] = "which file", ["--dry-run"] = "say what would happen" },
           }"#,
    )
    .expect("the spec declares");
    let spec = oslo_ui::spec::custom::find("c").unwrap();
    let file = spec
        .options
        .iter()
        .find(|opt| opt.names.contains(&"--file".to_string()))
        .expect("the flag is there");
    assert_eq!(file.names, vec!["-f", "--file"]);
    assert_eq!(file.description, "which file");
    assert_eq!(file.takes, oslo_ui::spec::Arg::Required);
    assert_eq!(spec.options.len(), 2);
    oslo_ui::spec::custom::forget();
}

#[test]
fn positions_flag_values_and_the_rest_of_the_model_are_read() {
    declare(
        r#"oslo.completion.spec {
             command = "deploy",
             aliases = { "dep" },
             parsing = "non-interspersed",
             persistent = { { "--config=", desc = "which config", values = { "a.toml" } } },
             positional = { { "build", "clean\tremove it" }, { "$files([.yaml])" } },
             positional_any = { "$files" },
             dash = { { "d0" } },
             dash_any = { "rest" },
             subcommands = { { name = "build", aliases = { "b" }, hidden = false } },
           }"#,
    )
    .expect("the spec declares");
    let spec = oslo_ui::spec::custom::find("deploy").unwrap();
    assert_eq!(spec.aliases, vec!["dep"]);
    assert_eq!(spec.parsing, oslo_ui::spec::Parsing::NonInterspersed);
    assert_eq!(spec.persistent[0].names, vec!["--config"]);
    assert!(matches!(&spec.persistent[0].values,
        oslo_ui::spec::Action::List(list) if list == &["a.toml"]));
    assert_eq!(spec.positional.len(), 2);
    assert!(matches!(&spec.positional[0],
        oslo_ui::spec::Action::List(list) if list[1] == "clean\tremove it"));
    assert!(!spec.positional_any.is_none());
    assert_eq!(spec.dash.len(), 1);
    assert!(!spec.dash_any.is_none());
    assert_eq!(spec.subcommands[0].aliases, vec!["b"]);
    oslo_ui::spec::custom::forget();
}

/// **A position may be a function.** The string macros exist because YAML has none; a config is
/// written in a language that does.
///
/// A whole engine here rather than the bare table above, because the point of the test is that
/// `call_here` finds the interpreter again when Tab is pressed — long after the config was read.
#[test]
fn a_position_may_be_computed() {
    use oslo_ui::spec::action::Query;
    on_a_real_engine(
        r#"oslo.completion.spec {
             command = "branches",
             positional = { function(ctx)
               return { { value = "main", desc = "the trunk", tag = "branch" },
                        "next\tthe other one",
                        ctx.value .. "-echo" }
             end },
           }"#,
    )
    .expect("the spec declares");
    let spec = oslo_ui::spec::custom::find("branches").unwrap();
    let query = Query {
        value: "typed".into(),
        ..Query::default()
    };
    let offers = oslo_ui::spec::resolve::resolve(&spec.positional[0], &query).offers;
    assert_eq!(offers[0].value, "main");
    assert_eq!(offers[0].description.as_deref(), Some("the trunk"));
    assert_eq!(offers[0].tag.as_deref(), Some("branch"));
    assert_eq!(offers[1].description.as_deref(), Some("the other one"));
    // The context reaches it: what was typed is what it was asked about.
    assert_eq!(offers[2].value, "typed-echo");
    oslo_ui::spec::custom::forget();
}

/// carapace's own key names are accepted beside oslo's, so a spec transcribed out of a `.yaml`
/// file works before it has been translated.
#[test]
fn carapaces_own_spelling_of_persistent_flags_is_accepted() {
    declare(
        r#"oslo.completion.spec { command = "c", persistentflags = { ["--help"] = "bool flag" } }"#,
    )
    .expect("the spec declares");
    let spec = oslo_ui::spec::custom::find("c").unwrap();
    assert_eq!(spec.persistent[0].names, vec!["--help"]);
    oslo_ui::spec::custom::forget();
}

/// Declare against a whole interpreter, the way a config is read at startup.
fn on_a_real_engine(source: &str) -> Result<(), String> {
    oslo_ui::spec::custom::forget();
    let engine = crate::LuaEngine::new().map_err(|e| e.to_string())?;
    let env = std::sync::Arc::new(std::sync::Mutex::new(oslo_shell::env::Environment::new()));
    engine.setup_bindings(env).map_err(|e| e.to_string())?;
    engine
        .eval_as(source, "spec test")
        .map(|_| ())
        .map_err(|e| e.to_string())
}
