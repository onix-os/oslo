use super::*;

/// Build the table `oslo.make.names()` answers with, so what is tested is the shape the runner
/// actually produces rather than one invented here.
fn names(source: &str) -> Value {
    let engine = crate::LuaEngine::new().expect("an engine");
    let env = std::sync::Arc::new(std::sync::Mutex::new(oslo_shell::env::Environment::new()));
    assert!(crate::startup::lua_init::install_bindings(&engine, env));
    engine
        .eval_as(source, "recipe test")
        .expect("the file loads");
    engine
        .eval_as("oslo.make.__names = oslo.make.names()", "oslo.make")
        .expect("names answers");
    field(&engine.oslo_table(), &["make", "__names"]).expect("a value")
}

#[test]
fn a_declared_recipe_becomes_a_subcommand_with_its_description() {
    let spec = from_names(&names(
        r#"make.recipe{ name = "build", desc = "the release binary", run = function() end }
           make.recipe{ name = "test",  desc = "the suite",          run = function() end }"#,
    ));
    assert_eq!(spec.name, "make");
    let offered: Vec<&str> = spec.subcommands.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(offered, vec!["build", "test"]);
    assert_eq!(spec.subcommands[0].description, "the release binary");
}

/// **A `_`-prefixed recipe is left out of the listing, so it is left out of the menu.** It is still
/// runnable — the completion follows the runner's own rule rather than inventing a second one.
#[test]
fn a_private_recipe_is_not_offered() {
    let spec = from_names(&names(
        r#"make.recipe{ name = "_inner", desc = "plumbing", run = function() end }
           make.recipe{ name = "outer",  desc = "the one you type", run = function() end }"#,
    ));
    let offered: Vec<&str> = spec.subcommands.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(offered, vec!["outer"]);
}

/// A recipe's `params` are the other half of what gets typed after its name.
#[test]
fn a_recipes_params_become_its_flags() {
    let spec = from_names(&names(
        r#"make.recipe{
             name = "install", desc = "put it somewhere",
             params = {
               { "--dest", desc = "somewhere else" },
               { "--system", flag = true, desc = "also /usr/bin" },
             },
             run = function() end,
           }"#,
    ));
    let install = &spec.subcommands[0];
    assert_eq!(install.options.len(), 2);
    assert_eq!(install.options[0].names, vec!["--dest"]);
    assert_eq!(install.options[0].description, "somewhere else");
    // A parameter taking a value is what makes the walk hand the next word to the flag.
    assert_eq!(install.options[0].takes, Arg::Required);
    // …and `flag = true` is a switch, which must not swallow the next word.
    assert_eq!(install.options[1].names, vec!["--system"]);
    assert_eq!(install.options[1].takes, Arg::None);
}

/// A file that declares nothing is a spec with nothing in it rather than a failure.
#[test]
fn a_file_with_no_recipes_answers_an_empty_spec() {
    let spec = from_names(&names("local unused = 1"));
    assert!(spec.subcommands.is_empty());
    assert_eq!(spec.name, "make");
}
