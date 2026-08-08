//! The stdlib, exercised the way an `.envrc` reaches it.

use super::*;
use crate::env::Environment;

fn shell() -> Environment {
    let mut env = Environment::new();
    crate::env::builtins::register_default_builtins(&mut env);
    install(&mut env);
    env
}

fn call(env: &mut Environment, words: &[&str]) -> i32 {
    let args: Vec<String> = words.iter().map(|word| word.to_string()).collect();
    let func = env.get_builtin(words[0]).expect("a stdlib builtin");
    func(env, &args).expect("ran")
}

/// **`PATH_add` twice does not put the directory on twice.**
///
/// A directory environment is loaded and reloaded — on an edit, on a nested shell, on
/// `direnv allow` — and this is the difference between a `$PATH` that stays the length it should
/// be and one that grows an entry every time until it is pages long.
#[test]
fn path_add_is_idempotent_and_puts_the_newest_first() {
    let mut env = shell();
    env.set_var("PATH", "/usr/bin", true);
    call(&mut env, &["PATH_add", "/opt/a"]);
    call(&mut env, &["PATH_add", "/opt/b"]);
    call(&mut env, &["PATH_add", "/opt/a"]);
    assert_eq!(env.get_var("PATH"), Some("/opt/a:/opt/b:/usr/bin"));
}

/// A relative directory means the project's, and the two spellings of it are one entry.
#[test]
fn a_relative_directory_is_resolved_and_spelled_once() {
    let mut env = shell();
    env.set_var("PATH", "/usr/bin", true);
    call(&mut env, &["PATH_add", "./bin"]);
    let after = env.get_var("PATH").expect("a path").to_string();
    call(&mut env, &["PATH_add", "bin"]);
    assert_eq!(env.get_var("PATH"), Some(after.as_str()));
    assert!(after.starts_with(&here().join("bin").to_string_lossy().to_string()));
}

/// `path_add` names its own variable, and `path_rm` takes entries back out by pattern.
#[test]
fn any_variable_can_be_added_to_and_pruned() {
    let mut env = shell();
    env.set_var(
        "LDFLAGS_PATH",
        "/nix/store/one:/usr/lib:/nix/store/two",
        true,
    );
    call(&mut env, &["path_rm", "LDFLAGS_PATH", "/nix/store/*"]);
    assert_eq!(env.get_var("LDFLAGS_PATH"), Some("/usr/lib"));
    call(&mut env, &["path_add", "LDFLAGS_PATH", "/opt/lib"]);
    assert_eq!(env.get_var("LDFLAGS_PATH"), Some("/opt/lib:/usr/lib"));
}

/// `expand_path` works on paths that do not exist, which is most of what a layout builds.
#[test]
fn a_path_is_expanded_without_being_visited() {
    let base = std::path::Path::new("/tmp/nowhere");
    assert_eq!(
        absolute("./a/../b", base),
        std::path::Path::new("/tmp/nowhere/b")
    );
    assert_eq!(absolute("/x/./y", base), std::path::Path::new("/x/y"));
}

/// **`dotenv` is not shell**, and the quoting rules are the ones every other reader of these uses.
#[test]
fn a_dotenv_is_read_with_its_own_grammar() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(".env");
    std::fs::write(
        &path,
        "# a comment\n\
         export A=plain\n\
         B=\"with \\n escape\"\n\
         C='literal \\n'\n\
         D=bare value # trailing\n\
         \n",
    )
    .expect("write");

    let mut env = shell();
    let args = ["dotenv".to_string(), path.to_string_lossy().into_owned()];
    let func = env.get_builtin("dotenv").expect("dotenv");
    assert_eq!(func(&mut env, &args).expect("ran"), 0);

    assert_eq!(env.get_var("A"), Some("plain"));
    assert_eq!(env.get_var("B"), Some("with \n escape"));
    assert_eq!(
        env.get_var("C"),
        Some(r"literal \n"),
        "single quotes are literal"
    );
    assert_eq!(env.get_var("D"), Some("bare value"));
}

/// The file it read is watched, or an edited `.env` goes unnoticed until something else reloads.
#[test]
fn reading_a_dotenv_makes_the_environment_depend_on_it() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(".env");
    std::fs::write(&path, "A=1\n").expect("write");
    let _ = take_watches();

    let mut env = shell();
    let args = ["dotenv".to_string(), path.to_string_lossy().into_owned()];
    let func = env.get_builtin("dotenv").expect("dotenv");
    func(&mut env, &args).expect("ran");
    assert!(take_watches().contains(&path), "the .env must be watched");
}

/// `has` answers for every route the shell would take, not only `$PATH`.
#[test]
fn has_counts_builtins_and_functions_too() {
    let mut env = shell();
    assert_eq!(call(&mut env, &["has", "cd"]), 0, "a builtin");
    assert_eq!(
        call(&mut env, &["has", "definitely-not-a-command-anywhere"]),
        1
    );
}

/// `env_vars_required` names all of them at once rather than one edit at a time.
#[test]
fn required_variables_are_reported_together() {
    let mut env = shell();
    env.set_var("PRESENT", "1", true);
    assert_eq!(call(&mut env, &["env_vars_required", "PRESENT"]), 0);
    assert_eq!(
        call(&mut env, &["env_vars_required", "PRESENT", "ABSENT"]),
        1
    );
}

/// **The stdlib is in scope for an `.envrc` and nowhere else.**
///
/// `PATH_add` at the prompt would edit an environment no file is holding open, and the undo record
/// would never hear about it.
#[test]
fn the_stdlib_leaves_no_names_behind() {
    let mut env = Environment::new();
    crate::env::builtins::register_default_builtins(&mut env);
    let before: Vec<String> = env.builtin_names().map(str::to_string).collect();

    install(&mut env);
    assert!(
        env.get_builtin("PATH_add").is_some(),
        "in scope while loading"
    );

    remove(&mut env);
    let after: Vec<String> = env.builtin_names().map(str::to_string).collect();
    assert!(env.get_builtin("PATH_add").is_none(), "and gone afterwards");
    assert_eq!(before.len(), after.len(), "and nothing else moved");
}

/// Every name in the table can be taken back out, which is what keeps install and remove in step.
#[test]
fn every_installed_name_is_removable() {
    let mut env = shell();
    for (name, _) in STDLIB {
        assert!(
            env.get_builtin(name).is_some(),
            "{name} should be installed"
        );
    }
    remove(&mut env);
    for (name, _) in STDLIB {
        assert!(env.get_builtin(name).is_none(), "{name} should be gone");
    }
}

/// A pattern with `*` crosses `/`, because `PATH_rm "/nix/*"` is meant to take out everything under
/// it — these are not filenames.
#[test]
fn a_star_in_a_pattern_spans_separators() {
    let mut env = shell();
    env.set_var("P", "/nix/store/a/bin:/home/me/bin:/nix/b", true);
    call(&mut env, &["path_rm", "P", "/nix/*"]);
    assert_eq!(env.get_var("P"), Some("/home/me/bin"));
}
