use super::*;
use crate::env::Environment;

fn run(script: &str, args: &[&str]) -> (i32, Environment) {
    let mut env = Environment::new();
    env.shell_name = "demo".to_string();
    let mut words = vec!["argc".to_string()];
    words.extend(args.iter().map(|a| a.to_string()));
    let status = with_source(&mut env, script, &words);
    (status, env)
}

/// The builtin reads the script by name; a test has the text instead, so this is the same call with
/// the source handed in.
///
/// **The script is run first.** In a real one the `argc "$@"` line is the last, so every function it
/// declares — a subcommand, a computed default — is already defined by the time the parse asks for
/// it. A test that skipped that would be testing a shell no script ever runs in.
fn with_source(env: &mut Environment, source: &str, args: &[String]) -> i32 {
    if let Ok(program) = crate::syntax::parse_bash_script(source) {
        let _ = crate::exec::eval_command_list(env, &program);
    }
    let mut words = vec![env.shell_name.clone()];
    words.extend(args.iter().skip(1).cloned());
    let runtime = Shell::new(env);
    let values = argc::eval(runtime, source, &words, Some("demo"), Some(80)).expect("parses");
    match apply(env, &values) {
        Ok(status) => status,
        Err(error) => error.control_flow_status().unwrap_or(1),
    }
}

const SCRIPT: &str = "\
# @describe Deploy a thing
# @flag   -n --dry-run     say what would happen
# @option -t --tries <N>   how many times
# @option -f --file* <F>   files, repeatable
# @arg    target!          where to
";

#[test]
fn a_flag_an_option_and_an_argument_land_in_variables() {
    let (status, env) = run(SCRIPT, &["--dry-run", "-t", "3", "prod"]);
    assert_eq!(status, 0);
    assert_eq!(env.get_var("argc_dry_run"), Some("1"));
    assert_eq!(env.get_var("argc_tries"), Some("3"));
    assert_eq!(env.get_var("argc_target"), Some("prod"));
}

/// A dash in a declaration is an underscore in the variable, because a dash is not a shell name.
#[test]
fn a_dashed_name_becomes_a_shell_name() {
    assert_eq!(argc_name("dry-run"), "argc_dry_run");
    assert_eq!(argc_name("tries"), "argc_tries");
}

/// A repeatable option is an array, which is what the bash rendering writes as `name=( … )`.
#[test]
fn a_repeatable_option_is_an_array() {
    let (status, env) = run(SCRIPT, &["-f", "a.txt", "-f", "b.txt", "prod"]);
    assert_eq!(status, 0);
    let array = env.get_array("argc_file").expect("an array");
    let values: Vec<&str> = array.values().collect();
    assert_eq!(values, ["a.txt", "b.txt"]);
}

/// **`--help` ends the script.** The bash rendering finishes with `exit 0`; a builtin that only
/// returned a status would leave the body to run with nothing set, which is what happened before
/// this returned the control-flow error instead.
#[test]
fn help_stops_the_script_rather_than_setting_nothing() {
    let mut env = Environment::new();
    env.shell_name = "demo".to_string();
    let runtime = Shell::new(&mut env);
    let words = vec!["demo".to_string(), "--help".to_string()];
    let values = argc::eval(runtime, SCRIPT, &words, Some("demo"), Some(80)).expect("parses");

    match apply(&mut env, &values) {
        Err(error) => assert_eq!(error.control_flow_status(), Some(0), "help succeeds"),
        Ok(status) => panic!("help carried on with status {status}"),
    }
}

/// A missing required argument is the script's error, and it exits non-zero.
#[test]
fn a_missing_argument_is_an_error_that_ends_it() {
    let mut env = Environment::new();
    env.shell_name = "demo".to_string();
    let runtime = Shell::new(&mut env);
    let words = vec!["demo".to_string()];
    let values = argc::eval(runtime, SCRIPT, &words, Some("demo"), Some(80)).expect("parses");
    match apply(&mut env, &values) {
        Err(error) => assert_eq!(error.control_flow_status(), Some(1)),
        Ok(status) => panic!("a missing argument carried on with status {status}"),
    }
}

/// The subcommand the arguments chose is called, in this shell.
#[test]
fn a_subcommand_runs_the_function_it_names() {
    let script = "\
# @cmd Say hello
# @arg who!
hello() {
    printf 'hello %s' \"$argc_who\"
}
";
    let (status, env) = run(script, &["hello", "world"]);
    assert_eq!(status, 0);
    assert_eq!(env.get_var("argc_who"), Some("world"));
}

/// **A default computed by a function runs in this shell, not in a bash somewhere.** That is the
/// whole point of implementing `argc::Runtime` rather than using the one upstream ships.
#[test]
fn a_default_from_a_function_is_computed_here() {
    let script = "\
# @option --dir=`_here`
_here() {
    printf /somewhere
}
";
    let (status, env) = run(script, &[]);
    assert_eq!(status, 0);
    assert_eq!(env.get_var("argc_dir"), Some("/somewhere"));
}

/// **At a prompt there is no script**, and every other builtin answers `--help` with what it is
/// for. Reporting "cannot read the script" about the shell binary explains nothing to somebody who
/// typed `argc` to find out what it does.
#[test]
fn asked_outside_a_script_it_says_what_it_is_for() {
    let usage = self_help("/usr/bin/oslo");
    assert!(usage.starts_with("usage: argc"), "{usage}");
    assert!(usage.contains("argc \"$@\""), "how to call it");
    assert!(usage.contains("--argc-eval"), "and the bash spelling");
    assert!(usage.contains("/usr/bin/oslo"), "what $0 actually was");
}
