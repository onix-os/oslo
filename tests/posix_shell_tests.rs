use rush::env::Environment;
use rush::exec::eval_command_list;
use rush::lua::LuaEngine;
use rush::parser::parse_bash_script;
use std::sync::{Arc, Mutex};

fn run_cmd(env: &mut Environment, input: &str) -> i32 {
    let ast = parse_bash_script(input).expect("Parsing failed");
    eval_command_list(env, &ast).expect("Execution failed")
}

#[test]
fn test_variable_assignment_and_expansion() {
    let mut env = Environment::new();
    let status = run_cmd(&mut env, "FOO=bar; export BAZ=qux");
    assert_eq!(status, 0);
    eprintln!("TEST FOO: {:?}", env.get_param("FOO"));
    eprintln!("TEST BAZ: {:?}", env.get_param("BAZ"));
    assert_eq!(env.get_param("FOO"), Some("bar".to_string()));
    assert_eq!(env.get_param("BAZ"), Some("qux".to_string()));
}

#[test]
fn test_arithmetic_expansion() {
    let mut env = Environment::new();
    run_cmd(&mut env, "X=10; Y=$((X + 5 * 2))");
    assert_eq!(env.get_param("Y"), Some("20".to_string()));
}

#[test]
fn test_if_else_compound_command() {
    let mut env = Environment::new();
    let status = run_cmd(&mut env, "if true; then OUT=yes; else OUT=no; fi");
    assert_eq!(status, 0);
    assert_eq!(env.get_param("OUT"), Some("yes".to_string()));
}

#[test]
fn test_while_loop() {
    let mut env = Environment::new();
    run_cmd(
        &mut env,
        "COUNT=0; while [ $COUNT -lt 3 ]; do COUNT=$((COUNT + 1)); done",
    );
    assert_eq!(env.get_param("COUNT"), Some("3".to_string()));
}

#[test]
fn test_for_loop() {
    let mut env = Environment::new();
    run_cmd(
        &mut env,
        "VALS=\"\"; for i in a b c; do VALS=\"$VALS$i\"; done",
    );
    assert_eq!(env.get_param("VALS"), Some("abc".to_string()));
}

#[test]
fn test_pipeline_execution() {
    let mut env = Environment::new();
    let status = run_cmd(&mut env, "echo hello | grep -q hello");
    assert_eq!(status, 0);
}

#[test]
fn test_lua_integration_exec() {
    let env = Arc::new(Mutex::new(Environment::new()));
    let lua = LuaEngine::new().expect("Lua init failed");
    lua.setup_bindings(Arc::clone(&env))
        .expect("Bindings failed");

    let script = r#"
        rush.exec("MYLUA=works")
        res = rush.get_var("MYLUA")
        rush.set_alias("l", "ls -l")
        alias_val = rush.get_alias("l")
        pwd = rush.get_pwd()
    "#;

    lua.eval_script(script).expect("Script execution failed");
    let guard = env.lock().unwrap();
    assert_eq!(guard.get_param("MYLUA"), Some("works".to_string()));
    assert_eq!(guard.get_alias("l"), Some("ls -l"));
}

#[test]
fn test_bash_script_parsing() {
    let mut env = Environment::new();
    let script = r#"
        A=20
        B=30
        C=$((A + B))
        if [ $C -eq 50 ]; then
            RESULT="success"
        fi
    "#;
    let status = run_cmd(&mut env, script);
    assert_eq!(status, 0);
    assert_eq!(env.get_param("RESULT"), Some("success".to_string()));
}

#[test]
fn test_eval_builtin() {
    let mut env = Environment::new();
    let status = run_cmd(&mut env, "eval 'EVAL_VAR=100'");
    assert_eq!(status, 0);
    assert_eq!(env.get_param("EVAL_VAR"), Some("100".to_string()));
}

#[test]
fn test_source_builtin() {
    let _guard = TEST_DIR_MUTEX.lock().unwrap();
    let mut env = Environment::new();
    let temp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(temp.path(), "SOURCED_VAR=hello").unwrap();
    let cmd = format!("source {}", temp.path().to_str().unwrap());
    let status = run_cmd(&mut env, &cmd);
    assert_eq!(status, 0);
    assert_eq!(env.get_param("SOURCED_VAR"), Some("hello".to_string()));
}

#[test]
fn test_builtin_conditions() {
    let mut env = Environment::new();
    let status1 = run_cmd(&mut env, "test -z ''");
    assert_eq!(status1, 0);

    let status2 = run_cmd(&mut env, "[ 'abc' = 'abc' ]");
    assert_eq!(status2, 0);

    let status3 = run_cmd(&mut env, "[ 10 -gt 5 ]");
    assert_eq!(status3, 0);
}

#[test]
fn test_trap_builtin() {
    let mut env = Environment::new();
    let status = run_cmd(&mut env, "trap 'echo cleanup' EXIT");
    assert_eq!(status, 0);
    assert_eq!(env.get_trap("EXIT"), Some("echo cleanup"));
}

#[test]
fn test_umask_builtin() {
    let mut env = Environment::new();
    let status = run_cmd(&mut env, "umask 0022");
    assert_eq!(status, 0);
}

#[test]
fn test_interactive_prompt() {
    let left = rush::interactive::prompt::render_default_left_prompt(0);
    assert!(!left.is_empty());
    let right = rush::interactive::prompt::render_default_right_prompt();
    assert!(!right.is_empty());
}

static TEST_DIR_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_pushd_popd_dirs() {
    let orig_dir = std::env::current_dir().unwrap();
    let _guard = TEST_DIR_MUTEX.lock().unwrap();
    let mut env = Environment::new();
    let test_dir = std::env::temp_dir().join(format!("rush_test_pushd_dir_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&test_dir);

    let cmd = format!("pushd {}", test_dir.display());
    let status = run_cmd(&mut env, &cmd);
    assert_eq!(status, 0);

    let status_pop = run_cmd(&mut env, "popd");
    assert_eq!(status_pop, 0);

    let _ = std::env::set_current_dir(&orig_dir);
    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_readonly_vars() {
    let mut env = Environment::new();
    let status = run_cmd(&mut env, "readonly IMMUTABLE=123");
    assert_eq!(status, 0);

    run_cmd(&mut env, "IMMUTABLE=456");
    assert_eq!(env.get_param("IMMUTABLE"), Some("123".to_string()));
}

#[test]
fn test_function_local_scope() {
    let mut env = Environment::new();
    let script = r#"
        GLOBAL_VAR="outer"
        my_func() {
            local GLOBAL_VAR="inner"
        }
        my_func
    "#;
    let status = run_cmd(&mut env, script);
    assert_eq!(status, 0);
    assert_eq!(env.get_param("GLOBAL_VAR"), Some("outer".to_string()));
}

#[test]
fn test_auto_cd() {
    let orig_dir = std::env::current_dir().unwrap();
    let _guard = TEST_DIR_MUTEX.lock().unwrap();
    let mut env = Environment::new();
    let test_dir =
        std::env::temp_dir().join(format!("rush_test_auto_cd_dir_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&test_dir);

    let status = run_cmd(&mut env, test_dir.to_str().unwrap());
    assert_eq!(status, 0);

    let _ = std::env::set_current_dir(&orig_dir);
    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_dropdown_menu_render() {
    let candidates = vec![
        rush::interactive::dropdown::CompletionCandidate::new(
            "cargo".to_string(),
            "cargo".to_string(),
            Some("Rust package manager".to_string()),
        ),
        rush::interactive::dropdown::CompletionCandidate::new(
            "cd".to_string(),
            "cd".to_string(),
            Some("Change working directory".to_string()),
        ),
    ];
    let (rendered, lines) =
        rush::interactive::dropdown::render_vertical_dropdown(&candidates, 0, 8, 0);
    assert!(rendered.contains("cargo"));
    assert!(rendered.contains("cd"));
    assert_eq!(lines, 4);
}
