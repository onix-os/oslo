//! What each invocation means.

use super::*;

fn parse_args(args: &[&str]) -> Result<Invocation, Exit> {
    let argv: Vec<String> = std::iter::once("oslo")
        .chain(args.iter().copied())
        .map(str::to_string)
        .collect();
    parse(&argv)
}

#[test]
fn no_arguments_reads_stdin() {
    let inv = parse_args(&[]).expect("parse");
    assert_eq!(inv.action, Action::Stdin);
    assert_eq!(inv.name, "oslo");
    assert!(inv.positional.is_empty());
}

#[test]
fn dash_c_takes_the_next_argument() {
    let inv = parse_args(&["-c", "echo hi"]).expect("parse");
    assert_eq!(inv.action, Action::Command("echo hi".to_string()));
}

#[test]
fn dash_c_takes_an_attached_argument() {
    let inv = parse_args(&["-cecho hi"]).expect("parse");
    assert_eq!(inv.action, Action::Command("echo hi".to_string()));
}

#[test]
fn dash_c_without_an_argument_is_a_usage_error() {
    let err = parse_args(&["-c"]).expect_err("must not start a REPL");
    assert_eq!(err.status, 2);
    assert!(err.to_stderr);
    assert!(
        err.message.contains("requires an argument"),
        "{}",
        err.message
    );
}

#[test]
fn dash_c_operands_supply_zero_and_positionals() {
    let inv = parse_args(&["-c", "echo", "myname", "a", "b"]).expect("parse");
    assert_eq!(inv.name, "myname");
    assert_eq!(inv.positional, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn script_operand_becomes_zero_and_the_rest_positional() {
    let inv = parse_args(&["run.sh", "one", "two"]).expect("parse");
    assert_eq!(inv.action, Action::Script("run.sh".to_string()));
    assert_eq!(inv.name, "run.sh");
    assert_eq!(inv.positional, vec!["one".to_string(), "two".to_string()]);
}

#[test]
fn dash_c_ends_option_parsing_at_its_command_string() {
    // `find -exec sh -c '…' -- {} +` passes `--` as `$0`; treating it as the end of options
    // would shift every positional by one and silently run the command against nothing.
    let inv = parse_args(&["-c", "echo", "--", "x", "y"]).expect("parse");
    assert_eq!(inv.name, "--");
    assert_eq!(inv.positional, vec!["x".to_string(), "y".to_string()]);

    // Nor is a later `-x` an option any more: it is `$1`, and tracing stays off.
    let inv = parse_args(&["-c", "echo", "name", "-x", "z"]).expect("parse");
    assert_eq!(inv.name, "name");
    assert_eq!(inv.positional, vec!["-x".to_string(), "z".to_string()]);
    assert_eq!(inv.set_options, "");
}

#[test]
fn options_before_dash_c_are_still_options() {
    let inv = parse_args(&["-x", "-c", "echo", "name"]).expect("parse");
    assert_eq!(inv.set_options, "x");
    assert_eq!(inv.name, "name");
}

#[test]
fn clustered_options_are_all_seen() {
    let inv = parse_args(&["-eix", "run.sh"]).expect("parse");
    assert!(inv.force_interactive);
    assert_eq!(inv.set_options, "ex");
    assert_eq!(inv.action, Action::Script("run.sh".to_string()));
}

#[test]
fn double_dash_ends_options() {
    let inv = parse_args(&["--", "-weird-name.sh"]).expect("parse");
    assert_eq!(inv.action, Action::Script("-weird-name.sh".to_string()));
}

#[test]
fn unknown_short_option_is_a_usage_error() {
    let err = parse_args(&["-Z"]).expect_err("unknown option");
    assert_eq!(err.status, 2);
    assert!(
        err.message.contains("-Z: invalid option"),
        "{}",
        err.message
    );
}

#[test]
fn unknown_long_option_is_a_usage_error() {
    let err = parse_args(&["--nope"]).expect_err("unknown option");
    assert_eq!(err.status, 2);
    assert!(
        err.message.contains("--nope: invalid option"),
        "{}",
        err.message
    );
}

#[test]
fn version_and_help_exit_zero_on_stdout() {
    let v = parse_args(&["--version"]).expect_err("terminates");
    assert_eq!(v.status, 0);
    assert!(!v.to_stderr);
    assert!(v.message.contains(env!("CARGO_PKG_VERSION")));

    let h = parse_args(&["--help"]).expect_err("terminates");
    assert_eq!(h.status, 0);
    assert!(!h.to_stderr);
    assert!(h.message.contains("USAGE"), "{}", h.message);
    assert!(h.message.contains("config"), "the tools are listed");
}

/// `--details` is the long form, and works written either side of `--help` — nobody should have
/// to remember an order.
#[test]
fn details_is_the_long_help_in_either_order() {
    let short = parse_args(&["--help"]).expect_err("terminates").message;
    for spelling in [
        vec!["--help", "--details"],
        vec!["--details", "--help"],
        vec!["--details"],
    ] {
        let long = parse_args(&spelling).expect_err("terminates");
        assert_eq!(long.status, 0, "{spelling:?}");
        assert!(!long.to_stderr, "{spelling:?}");
        assert!(
            long.message.len() > short.len(),
            "{spelling:?}: no more than the short help"
        );
        assert!(
            long.message.contains("xtrace"),
            "{spelling:?}: no option reference"
        );
    }
}

/// Past `--`, a `--details` is somebody's argument. Consulting the whole command line for it
/// would make `oslo --help -- --details` mean something different from what was written.
#[test]
fn details_after_a_double_dash_is_not_a_flag() {
    let h = parse_args(&["--help", "--", "--details"]).expect_err("terminates");
    assert!(!h.message.contains("xtrace"), "it was read as a flag");
}

/// `oslo history` reaches the tool, and its arguments come with it.
#[test]
fn a_tool_can_be_named_in_the_operand_slot() {
    let inv = parse_args(&["history", "search", "-n", "5"]).expect("parse");
    assert_eq!(
        inv.action,
        Action::Tool(
            "history".to_string(),
            vec!["search".to_string(), "-n".to_string(), "5".to_string()]
        )
    );
}

/// **`--` says the word is a path.** The escape hatch for the day somebody has a script named
/// `hook`, and the reason the operand slot is not silently narrowed.
#[test]
fn a_double_dash_forces_the_operand_to_be_a_script() {
    let inv = parse_args(&["--", "history"]).expect("parse");
    assert_eq!(inv.action, Action::Script("history".to_string()));
}

/// A tool name after `-c` is `$0`, not a tool. Everything past `-c`'s command string is an
/// operand, and `find -exec sh -c '…' -- {} +` depends on that staying true.
#[test]
fn a_tool_name_is_not_special_after_dash_c() {
    let inv = parse_args(&["-c", "echo hi", "history", "arg"]).expect("parse");
    assert_eq!(inv.action, Action::Command("echo hi".to_string()));
    assert_eq!(inv.name, "history", "it is $0");
    assert_eq!(inv.positional, vec!["arg".to_string()]);
}

/// A path is a script however it is spelled, so a `#!/bin/oslo` script keeps working whatever it
/// is called — the kernel always hands over a slashed path.
#[test]
fn a_slashed_tool_name_is_a_script() {
    let inv = parse_args(&["./history"]).expect("parse");
    assert_eq!(inv.action, Action::Script("./history".to_string()));
}

#[test]
fn dash_s_reads_stdin_with_positionals() {
    let inv = parse_args(&["-s", "a", "b"]).expect("parse");
    assert_eq!(inv.action, Action::Stdin);
    assert_eq!(inv.positional, vec!["a".to_string(), "b".to_string()]);
}

/// `--posix` was not an option at all: `oslo --posix -c 'echo x'` was a usage error, which is
/// what made the whole POSIX-mode code path unreachable in production.
#[test]
fn posix_is_a_long_option_and_reaches_the_option_set() {
    let inv = parse_args(&["--posix", "-c", "echo x"]).expect("parse");
    assert_eq!(inv.action, Action::Command("echo x".to_string()));
    assert!(inv.options().any(|o| o == ShellOption::Posix));
    // It has no letter, so it must not have leaked into the `$-` string either.
    assert_eq!(inv.set_options, "");
}

/// `options()` is the single reader, so a mixed command line yields both spellings.
#[test]
fn options_yields_letters_and_long_names_together() {
    let inv = parse_args(&["--posix", "-ex", "run.sh"]).expect("parse");
    let opts: Vec<_> = inv.options().collect();
    assert!(opts.contains(&ShellOption::ErrExit));
    assert!(opts.contains(&ShellOption::XTrace));
    assert!(opts.contains(&ShellOption::Posix));
}

/// Repeats collapse, as they do for letters.
#[test]
fn a_repeated_long_option_is_recorded_once() {
    let inv = parse_args(&["--posix", "--posix"]).expect("parse");
    assert_eq!(inv.long_options, vec![ShellOption::Posix]);
}

/// The long-name flags are an explicit list, not "everything `set -o` accepts": bash rejects
/// `--errexit`, and accepting it would be inventing an interface.
#[test]
fn a_set_o_name_is_not_automatically_a_long_option() {
    let err = parse_args(&["--errexit"]).expect_err("not a command-line flag");
    assert!(
        err.message.contains("--errexit: invalid option"),
        "{}",
        err.message
    );
}

/// A Lua file is an ordinary operand. There is no flag to run Lua, which is the point:
/// `--lua-script FILE` used to be the only way, in a shell whose scripting language is Lua.
#[test]
fn a_lua_file_is_just_a_script_operand() {
    let inv = parse_args(&["init.lua"]).expect("parse");
    assert_eq!(inv.action, Action::Script("init.lua".to_string()));
    assert_eq!(inv.name, "init.lua");
    assert_eq!(inv.force_language, None, "nothing was forced");
}

#[test]
fn the_language_can_still_be_forced_either_way() {
    let lua = parse_args(&["--lua", "script"]).expect("parse");
    assert_eq!(lua.force_language, Some(Language::Lua));
    assert_eq!(lua.action, Action::Script("script".to_string()));

    let sh = parse_args(&["--sh", "script"]).expect("parse");
    assert_eq!(sh.force_language, Some(Language::Shell));
}
/// **Off only.** vi is the default, so the flag turns it off and there is no flag that turns it
/// on — `--vi` would be a flag that asks for what you already have. Both spellings work.
#[test]
fn vi_can_be_turned_off() {
    assert_eq!(parse_args(&[]).expect("parse").vi, None, "config decides");
    assert_eq!(parse_args(&["--no-vi"]).expect("parse").vi, Some(false));
    assert_eq!(parse_args(&["--no-vim"]).expect("parse").vi, Some(false));
}

/// And `--vi` is *refused*, not quietly ignored. A flag that used to work and now does nothing is
/// the one failure a user cannot see: the shell starts, and the setting silently is not applied.
#[test]
fn the_old_vi_on_flag_is_gone() {
    for flag in ["--vi", "--vim"] {
        let err = parse_args(&[flag]).expect_err("must be rejected");
        assert!(
            err.message.contains("invalid option"),
            "{flag}: {}",
            err.message
        );
        assert_eq!(err.status, 2);
    }
}

/// The flag it replaced must not linger as a silently-accepted no-op.
#[test]
fn the_old_lua_script_flag_is_gone() {
    let err = parse_args(&["--lua-script", "init.lua"]).expect_err("must be rejected");
    assert!(
        err.message.contains("--lua-script: invalid option"),
        "{}",
        err.message
    );
}

/// `--profile` names which pair of stores a shell writes to.
#[test]
fn a_profile_can_be_named() {
    assert_eq!(parse_args(&[]).expect("parse").profile, None, "the default");
    assert_eq!(
        parse_args(&["--profile=claude"]).expect("parse").profile,
        Some("claude".to_string())
    );
    // Last one wins, as with every other repeated flag.
    assert_eq!(
        parse_args(&["--profile=a", "--profile=b"])
            .expect("parse")
            .profile,
        Some("b".to_string())
    );
}

/// A `--profile` with nothing after it is a usage error, not a silent fall back to the
/// default — a typo there would quietly write an agent's history into yours.
#[test]
fn a_profile_needs_a_name() {
    for args in [vec!["--profile"], vec!["--profile="], vec!["--profile=  "]] {
        let err = parse_args(&args).expect_err("must be refused");
        assert_eq!(err.status, 2, "{args:?}");
        assert!(err.message.contains("--profile"), "{}", err.message);
    }
}

/// A name oslo will not use is refused, not cleaned into a different one — the name *is* the
/// store, so a typo must not quietly write somewhere else.
#[test]
fn a_profile_name_is_letters_digits_underscore_and_dash() {
    for good in ["claude", "agent-1", "test_run", "a"] {
        assert_eq!(
            parse_args(&[&format!("--profile={good}")])
                .expect("accepted")
                .profile,
            Some(good.to_string())
        );
    }
    for bad in [
        "../escape",
        "9lives",
        "with space",
        "-dash",
        "dot.name",
        "a/b",
    ] {
        let err = parse_args(&[&format!("--profile={bad}")]).expect_err("must be refused");
        assert_eq!(err.status, 2, "{bad:?}");
        assert!(err.message.contains(bad), "{}", err.message);
    }
}
