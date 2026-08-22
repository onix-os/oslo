//! `oslo.command.when` — hiding a program on `$PATH` in the directories that should not see it.
//!
//! The sibling of `feature_tests.rs`, and tested the same two ways for the same reasons.
//!
//! **Hiding** is observed through a spawned binary, because the claim is about what `type` says and
//! what actually runs, and because the mask is process-global.
//!
//! **The predicate** is exercised in process against `command::decide`, which is the function
//! `arrival::arrive` calls on every move.

mod common;

use common::oslo_bin;
use oslo::env::Environment;
use oslo::lua::LuaEngine;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};

/// A directory with a runnable `moo` in it, and a marker in the half that may see it.
fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).expect("bin");
    let program = bin.join("moo");
    std::fs::write(&program, "#!/bin/sh\necho moo\n").expect("write moo");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    std::fs::create_dir_all(dir.path().join("seen")).expect("seen");
    std::fs::write(dir.path().join("seen/.marker"), "").expect("marker");
    std::fs::create_dir_all(dir.path().join("unseen")).expect("unseen");
    dir
}

/// Run `script` in a shell whose config hides `moo` outside a directory holding `.marker`.
fn run(dir: &Path, script: &str) -> String {
    let home = dir.join("home");
    std::fs::create_dir_all(home.join(".config/oslo")).expect("config dir");
    std::fs::write(
        home.join(".config/oslo/init.lua"),
        "oslo.misc.welcome = false\n\
         oslo.command.when(\"moo\", function(d) return oslo.fs.find_up(\".marker\", d) ~= nil end)\n",
    )
    .expect("init.lua");

    let output: Output = Command::new(oslo_bin())
        .arg("-i")
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("TERM", "dumb")
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", dir.join("bin").display()),
        )
        .env_remove("ENV")
        // **The undo record, which is inherited.** `$OSLO_DIRENV` records the `$PATH` a directory
        // environment is holding open so that leaving can put it back — so a shell spawned from
        // inside one is born owing a restore, and the first thing it does is overwrite the `$PATH`
        // this test just set with the one its parent had. The command is then never found and the
        // test fails saying the mask hid it, which is the wrong answer to the wrong question.
        .env_remove("OSLO_DIRENV")
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .as_mut()
                .expect("stdin")
                .write_all(script.as_bytes())?;
            child.wait_with_output()
        })
        .expect("spawn oslo");
    String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr)
}

/// **The case the whole thing exists for.** The same word, in two directories, one answer each.
#[test]
fn a_command_is_visible_where_its_predicate_says_so() {
    let dir = tree();
    let said = run(
        dir.path(),
        &format!(
            "cd {}\nmoo\ncd {}\nmoo\nexit\n",
            dir.path().join("seen").display(),
            dir.path().join("unseen").display()
        ),
    );
    assert!(
        said.contains("moo\n"),
        "the marked directory should run it: {said}"
    );
    assert!(
        said.contains("command not found"),
        "the unmarked one should not: {said}"
    );
}

/// `type` has to agree with what would run. A `type` naming a path the shell then refuses is worse
/// than either answer alone, which is why the mask is read in `path_matches` as well as in the
/// `$PATH` search.
#[test]
fn type_agrees_with_what_would_run() {
    let dir = tree();
    let said = run(
        dir.path(),
        &format!(
            "cd {}\ntype moo\ncd {}\ntype moo\nexit\n",
            dir.path().join("seen").display(),
            dir.path().join("unseen").display()
        ),
    );
    assert!(
        said.contains("moo is "),
        "type should find it where it is visible: {said}"
    );
    assert!(
        said.contains("type: moo: not found"),
        "and find nothing where it is not: {said}"
    );
}

/// A word with a slash is a path, not a search — POSIX 2.9.1.1 — so it is never hidden. The same
/// escape hatch `command` gives for a shadowed builtin.
#[test]
fn a_path_reaches_a_hidden_program_anyway() {
    let dir = tree();
    let said = run(
        dir.path(),
        &format!(
            "cd {}\n{}\nexit\n",
            dir.path().join("unseen").display(),
            dir.path().join("bin/moo").display()
        ),
    );
    assert!(
        said.contains("moo\n"),
        "an absolute path is not a $PATH search: {said}"
    );
}

/// Leaving is arriving with the other answer, and there is no restore step to forget.
#[test]
fn walking_back_out_unhides_it() {
    let dir = tree();
    let said = run(
        dir.path(),
        &format!(
            "cd {}\ncd {}\nmoo\nexit\n",
            dir.path().join("unseen").display(),
            dir.path().join("seen").display()
        ),
    );
    assert!(said.contains("moo\n"), "hidden then visible again: {said}");
}

/// An engine with the bindings installed, kept alive by the caller.
fn engine() -> (LuaEngine, Arc<Mutex<Environment>>) {
    let env = Arc::new(Mutex::new(Environment::new()));
    let lua = LuaEngine::new().expect("Lua init");
    lua.setup_bindings(Arc::clone(&env)).expect("bindings");
    (lua, env)
}

/// A predicate that returns nothing has not answered, and must not be read as "hide it".
///
/// The same rule `oslo.feature.when` follows, and for the sharper reason: a handler with a missing
/// `return` on one branch would make a command disappear, which is about the hardest failure there
/// is to attribute to the config that caused it.
#[test]
fn returning_nothing_leaves_the_command_alone() {
    let (lua, _env) = engine();
    lua.eval_script(r#"oslo.command.when("quiet-one", function() end)"#)
        .expect("register");

    oslo::lua::api::command::decide(Path::new("/anywhere"));
    assert!(
        !oslo::command::hidden("quiet-one"),
        "a handler that said nothing must not hide anything"
    );
}

/// A name with a slash could never be consulted, since a path is not a `$PATH` search. Refused at
/// registration rather than accepted and quietly never asked.
#[test]
fn a_path_is_not_a_command_name() {
    let (lua, _env) = engine();
    let refused =
        lua.eval_script(r#"oslo.command.when("/usr/bin/moo", function() return false end)"#);
    assert!(refused.is_err(), "a path should be refused");
}
