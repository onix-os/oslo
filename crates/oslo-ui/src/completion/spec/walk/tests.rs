use super::*;
use crate::spec::{Action, SubcommandSpec};

fn flag(names: &[&str], takes: Arg) -> OptionSpec {
    OptionSpec {
        names: names.iter().map(|n| n.to_string()).collect(),
        takes,
        ..OptionSpec::default()
    }
}

/// `deploy [-v] [--env=ENV] [--files… ] build|clean [target] [--] [passthrough]`
fn spec() -> CommandSpec {
    CommandSpec {
        name: "deploy".into(),
        options: vec![flag(&["-v", "--verbose"], Arg::None)],
        persistent: vec![
            flag(&["--env"], Arg::Required),
            OptionSpec {
                nargs: Nargs::Any,
                ..flag(&["--files"], Arg::Required)
            },
        ],
        subcommands: vec![SubcommandSpec {
            name: "build".into(),
            aliases: vec!["b".into()],
            options: vec![flag(&["--target"], Arg::Required)],
            positional: vec![Action::list(["one"]), Action::list(["two"])],
            dash: vec![Action::list(["d0"])],
            ..SubcommandSpec::default()
        }],
        ..CommandSpec::default()
    }
}

fn words(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

#[test]
fn nothing_typed_is_the_first_position_of_the_root() {
    let spec = spec();
    let w = walk(&spec, &[]);
    assert_eq!(w.node.name, "deploy");
    assert_eq!(w.at, At::Positional(0));
}

#[test]
fn a_subcommand_moves_the_walk_and_its_aliases_do_too() {
    let spec = spec();
    assert_eq!(walk(&spec, &words("build")).node.name, "build");
    assert_eq!(walk(&spec, &words("b")).node.name, "build");
    assert_eq!(walk(&spec, &words("nonsense")).node.name, "deploy");
}

/// **The word after a flag that takes a value is that value, not the first argument.** This is the
/// whole reason the walk parses flags instead of skipping them.
#[test]
fn a_flags_argument_is_not_a_positional() {
    let spec = spec();
    assert_eq!(
        walk(&spec, &words("build --target x")).at,
        At::Positional(0)
    );
    assert_eq!(walk(&spec, &words("build x")).at, At::Positional(1));
    // …and a switch swallows nothing.
    assert_eq!(walk(&spec, &words("-v build x")).at, At::Positional(1));
}

#[test]
fn the_cursor_after_a_value_flag_is_inside_that_flag() {
    let spec = spec();
    let w = walk(&spec, &words("build --target"));
    assert!(matches!(w.at, At::FlagValue(opt) if opt.names[0] == "--target"));
    // Written inline, the flag is finished and the cursor is back on the positional.
    assert_eq!(
        walk(&spec, &words("build --target=x")).at,
        At::Positional(0)
    );
}

#[test]
fn a_persistent_flag_is_known_at_every_depth() {
    let spec = spec();
    let w = walk(&spec, &words("build --env"));
    assert!(matches!(w.at, At::FlagValue(opt) if opt.names[0] == "--env"));
    let offered: Vec<_> = walk(&spec, &words("build"))
        .flags_on_offer()
        .flat_map(|opt| opt.names.clone())
        .collect();
    assert!(offered.contains(&"--target".to_string()), "{offered:?}");
    assert!(offered.contains(&"--env".to_string()), "{offered:?}");
}

#[test]
fn nargs_any_eats_up_to_the_next_flag() {
    let spec = spec();
    assert_eq!(walk(&spec, &words("--files a b c")).at, At::Positional(0));
    assert_eq!(
        walk(&spec, &words("--files a b -v x")).at,
        At::Positional(1)
    );
}

#[test]
fn a_bare_dash_dash_starts_counting_again() {
    let spec = spec();
    assert_eq!(walk(&spec, &words("build --")).at, At::Dash(0));
    assert_eq!(walk(&spec, &words("build -- x")).at, At::Dash(1));
    // Nothing after `--` is a flag, however it is spelled.
    assert_eq!(walk(&spec, &words("build -- -v")).at, At::Dash(1));
}

#[test]
fn what_was_typed_is_available_to_the_values() {
    let spec = spec();
    let w = walk(&spec, &words("build --target release first"));
    assert_eq!(w.args, vec!["first"]);
    assert_eq!(w.flags.get("TARGET").map(String::as_str), Some("release"));
}

/// A flag nobody declared consumes itself and nothing else — swallowing the next word would lose
/// the subcommand the rest of the walk depends on.
#[test]
fn an_unknown_flag_does_not_eat_the_subcommand() {
    let spec = spec();
    assert_eq!(walk(&spec, &words("--unknown build")).node.name, "build");
}

#[test]
fn parsing_disabled_makes_every_word_an_argument() {
    let mut spec = spec();
    spec.parsing = Parsing::Disabled;
    let w = walk(&spec, &words("-v build"));
    assert_eq!(w.node.name, "deploy");
    assert_eq!(w.at, At::Positional(2));
}

#[test]
fn non_interspersed_stops_at_the_first_argument() {
    let mut spec = spec();
    spec.parsing = Parsing::NonInterspersed;
    // `-v` before the argument is still a flag; after it, it is an argument.
    assert_eq!(walk(&spec, &words("-v x -v")).at, At::Positional(2));
}

/// **A dashed word can name a subcommand.** `nix-store --gc`, `cmake -E`: 505 of the shipped specs
/// declare one, and reading every dashed word as a flag left every one of them unreachable.
#[test]
fn a_dashed_subcommand_is_reachable() {
    let mut spec = spec();
    spec.subcommands.push(SubcommandSpec {
        name: "--gc".into(),
        positional: vec![Action::list(["collected"])],
        ..SubcommandSpec::default()
    });
    let w = walk(&spec, &words("--gc"));
    assert_eq!(w.node.name, "--gc");
    assert_eq!(w.at, At::Positional(0));
}

/// …but a flag that was actually declared still wins, because that is both the common case and
/// the one where guessing wrong swallows a word.
#[test]
fn a_declared_flag_outranks_a_subcommand_of_the_same_name() {
    let mut spec = spec();
    spec.subcommands.push(SubcommandSpec {
        name: "-v".into(),
        ..SubcommandSpec::default()
    });
    // `-v` is declared as a switch on the root spec.
    let w = walk(&spec, &words("-v"));
    assert_eq!(w.node.name, "deploy");
    assert_eq!(w.at, At::Positional(0));
}

/// And a dashed subcommand is only a subcommand before the first argument, like any other.
#[test]
fn a_dashed_subcommand_is_not_looked_for_after_an_argument() {
    let mut spec = spec();
    spec.subcommands.push(SubcommandSpec {
        name: "--gc".into(),
        ..SubcommandSpec::default()
    });
    let w = walk(&spec, &words("first --gc"));
    assert_eq!(w.node.name, "deploy");
}
