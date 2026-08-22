//! `.make.lua`, end to end — because the interesting answers need a real process.
//!
//! The unit tests cover discovery and the help page. Everything here needs the whole thing: a
//! recipe file on disk, an engine that reads it, and commands that actually run. That is also the
//! only honest way to test the two claims the feature is built on — that a recipe can run a
//! command at all, and that a build which failed is not remembered as having succeeded.
//!
//! # Sandboxed, always
//!
//! `XDG_DATA_HOME` points into the temporary directory, so the content-staleness cache is this
//! test's and never the person's. `HOME` and `XDG_CONFIG_HOME` go with it, so no `init.lua` of
//! anyone's is loaded into a case that did not ask for one.

#![cfg(feature = "make")]

mod common;

use common::oslo_bin;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// A project: a temporary directory holding a `.make.lua`.
struct Project(tempfile::TempDir);

impl Project {
    fn new(recipes: &str) -> Project {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(".make.lua"), recipes).expect("write .make.lua");
        Project(dir)
    }

    fn path(&self) -> &Path {
        self.0.path()
    }

    fn write(&self, rel: &str, body: &str) {
        let path = self.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(path, body).expect("write");
    }

    fn exists(&self, rel: &str) -> bool {
        self.path().join(rel).exists()
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path().join(rel)).unwrap_or_default()
    }

    /// `oslo make …`, run from `cwd` inside the project.
    fn make_in(&self, cwd: &Path, words: &[&str]) -> Output {
        let mut args = vec!["make"];
        args.extend_from_slice(words);
        Command::new(oslo_bin())
            .args(&args)
            .current_dir(cwd)
            .env("HOME", self.path())
            .env("XDG_CONFIG_HOME", self.path().join("config"))
            .env("XDG_DATA_HOME", self.path().join("data"))
            .env_remove("ENV")
            .stdin(Stdio::null())
            .output()
            .expect("spawn oslo make")
    }

    fn make(&self, words: &[&str]) -> Output {
        self.make_in(self.path(), words)
    }
}

fn out(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn err(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

const BASIC: &str = r#"
local make = oslo.make
make.recipe{ name = "hello", desc = "say something",
  run = function() print("hello from a recipe") end }
make.recipe{ name = "chained", desc = "after hello", deps = { "hello" },
  run = function() print("and then this") end }
make.recipe{ name = "_hidden", desc = "runnable, unlisted",
  run = function() print("hidden ran") end }
make.alias("h", "hello")
"#;

/// **The claim the whole design rests on.** A registered builtin cannot run a command — the shell is
/// holding its state — so the runner is a child process. If this ever fails, the feature does not
/// work at all and every other test here is measuring something else.
#[test]
fn a_recipe_can_run_a_command() {
    let project = Project::new(
        r#"
local make = oslo.make
make.recipe{ name = "shellout", run = function() sh.echo("ran a command") end }
"#,
    );
    let output = project.make(&["shellout"]);
    assert!(
        out(&output).contains("ran a command"),
        "stdout {:?} stderr {:?}",
        out(&output),
        err(&output)
    );
    assert_eq!(code(&output), 0);
}

#[test]
fn no_recipe_lists_them_with_their_descriptions() {
    let project = Project::new(BASIC);
    let listed = out(&project.make(&[]));
    assert!(listed.contains("hello"), "{listed}");
    assert!(listed.contains("say something"), "{listed}");
    assert!(
        listed.contains("h") && listed.contains("→ hello"),
        "{listed}"
    );
}

/// A leading `_` keeps a name out of the listing without making it unrunnable — just's rule.
#[test]
fn a_private_recipe_is_unlisted_but_still_runs() {
    let project = Project::new(BASIC);
    assert!(!out(&project.make(&[])).contains("_hidden"));
    assert!(out(&project.make(&["_hidden"])).contains("hidden ran"));
}

#[test]
fn a_dependency_runs_first_and_only_once() {
    let project = Project::new(
        r#"
local make = oslo.make
make.recipe{ name = "once", run = function() print("once") end }
make.recipe{ name = "a", deps = { "once" }, run = function() print("a") end }
make.recipe{ name = "b", deps = { "once" }, run = function() print("b") end }
make.recipe{ name = "both", deps = { "a", "b" }, run = function() print("both") end }
"#,
    );
    let text = out(&project.make(&["both"]));
    assert_eq!(text.matches("once").count(), 2, "once ran twice: {text}");
    let at = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("{needle}: {text}"))
    };
    assert!(at("once") < at("\na\n"), "{text}");
    assert!(at("\na\n") < at("both"), "{text}");
}

#[test]
fn an_alias_reaches_the_recipe() {
    let project = Project::new(BASIC);
    assert!(out(&project.make(&["h"])).contains("hello from a recipe"));
}

/// The working directory is the file's, however deep you were standing — make's rule and just's.
#[test]
fn a_recipe_runs_from_the_projects_own_directory() {
    let project = Project::new(
        r#"
local make = oslo.make
make.recipe{ name = "where", run = function() print("cwd=" .. oslo.fs.cwd()) end }
"#,
    );
    let deep = project.path().join("a/b/c");
    std::fs::create_dir_all(&deep).expect("deep");
    let text = out(&project.make_in(&deep, &["where"]));
    let root = project.path().canonicalize().expect("canonical");
    assert!(
        text.contains(&format!("cwd={}", root.display())),
        "{text} (expected {})",
        root.display()
    );
}

/// **`oslo.run` never raises; inside a recipe the `sh` sugar does.** A build that carried on past a
/// failed command would be worse than no build tool at all.
#[test]
fn a_failing_command_ends_the_recipe_and_the_run() {
    let project = Project::new(
        r#"
local make = oslo.make
make.recipe{ name = "boom",
  run = function() sh.sh("-c", "exit 3"); print("never reached") end }
make.recipe{ name = "after", deps = { "boom" }, run = function() print("after ran") end }
"#,
    );
    let output = project.make(&["after"]);
    assert!(!out(&output).contains("never reached"), "{}", out(&output));
    assert!(!out(&output).contains("after ran"), "{}", out(&output));
    assert!(err(&output).contains("exited 3"), "{}", err(&output));
    assert_eq!(code(&output), 1);
}

/// `-k` runs the rest anyway, and still reports the failure in the status.
#[test]
fn keep_going_carries_on_but_the_status_still_fails() {
    let project = Project::new(
        r#"
local make = oslo.make
make.recipe{ name = "boom", run = function() sh.sh("-c", "exit 3") end }
make.recipe{ name = "after", deps = { "boom" }, run = function() print("after ran") end }
"#,
    );
    let output = project.make(&["-k", "after"]);
    assert!(out(&output).contains("after ran"), "{}", out(&output));
    assert_eq!(code(&output), 1, "a kept-going failure is still a failure");
}

#[test]
fn a_dependency_cycle_is_named_rather_than_hung() {
    let project = Project::new(
        r#"
local make = oslo.make
make.recipe{ name = "a", deps = { "b" }, run = function() end }
make.recipe{ name = "b", deps = { "a" }, run = function() end }
"#,
    );
    let output = project.make(&["a"]);
    assert!(
        err(&output).contains("dependency cycle"),
        "{}",
        err(&output)
    );
    assert!(err(&output).contains("a → b → a"), "{}", err(&output));
    assert_eq!(code(&output), 2);
}

#[test]
fn an_unknown_recipe_suggests_the_ones_that_exist() {
    let project = Project::new(BASIC);
    let output = project.make(&["hell"]);
    assert!(
        err(&output).contains("no recipe called"),
        "{}",
        err(&output)
    );
    assert!(err(&output).contains("hello"), "{}", err(&output));
    assert_eq!(code(&output), 2);
}

const STALE: &str = r#"
local make = oslo.make
make.recipe{ name = "bundle", inputs = { "src/*.txt" }, outputs = { "out.txt" },
  run = function() sh.sh("-c", "cat src/*.txt > out.txt") end }
"#;

#[test]
fn a_recipe_with_outputs_is_skipped_when_it_is_up_to_date() {
    let project = Project::new(STALE);
    project.write("src/a.txt", "one\n");
    assert!(out(&project.make(&["bundle"])).contains("→ bundle"));
    assert!(project.exists("out.txt"));
    let again = out(&project.make(&["bundle"]));
    assert!(again.contains("up to date"), "{again}");
}

#[test]
fn an_input_newer_than_the_output_rebuilds() {
    let project = Project::new(STALE);
    project.write("src/a.txt", "one\n");
    project.make(&["bundle"]);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    project.write("src/a.txt", "changed\n");
    assert!(out(&project.make(&["bundle"])).contains("→ bundle"));
    assert_eq!(project.read("out.txt"), "changed\n");
}

/// `--force` is the escape hatch for the day the staleness rule is wrong about your project.
#[test]
fn force_runs_a_recipe_that_is_up_to_date() {
    let project = Project::new(STALE);
    project.write("src/a.txt", "one\n");
    project.make(&["bundle"]);
    assert!(out(&project.make(&["-f", "bundle"])).contains("→ bundle"));
}

/// **The reason `stale = "content"` exists.** A `git checkout` that lands the same bytes moves every
/// mtime in the tree; mtime staleness rebuilds the world, and content staleness does not.
#[test]
fn content_staleness_ignores_a_touch_and_sees_an_edit() {
    let project = Project::new(
        r#"
local make = oslo.make
make.recipe{ name = "bundle", inputs = { "src/*.txt" }, outputs = { "out.txt" },
  stale = "content", run = function() sh.sh("-c", "cat src/*.txt > out.txt") end }
"#,
    );
    project.write("src/a.txt", "one\n");
    assert!(out(&project.make(&["bundle"])).contains("→ bundle"));
    assert!(
        out(&project.make(&["bundle"])).contains("up to date"),
        "a fresh build must record itself, or every project builds twice"
    );

    std::thread::sleep(std::time::Duration::from_millis(1100));
    project.write("src/a.txt", "one\n"); // same bytes, new mtime
    let touched = out(&project.make(&["bundle"]));
    assert!(touched.contains("up to date"), "a touch rebuilt: {touched}");

    project.write("src/a.txt", "two\n");
    assert!(out(&project.make(&["bundle"])).contains("→ bundle"));
}

/// **A build that failed is not a build that happened.** The stamp is written after the body
/// returns, never during the staleness check — the first version recorded it up front, so a failed
/// build reported "up to date" on the next run.
#[test]
fn a_failed_build_is_not_remembered_as_done() {
    let project = Project::new(
        r#"
local make = oslo.make
make.recipe{ name = "bad", inputs = { "src/*.txt" }, outputs = { "bad.txt" },
  stale = "content", run = function() sh.sh("-c", "touch bad.txt; exit 3") end }
"#,
    );
    project.write("src/a.txt", "one\n");
    assert_eq!(code(&project.make(&["bad"])), 1);
    let again = project.make(&["bad"]);
    assert!(
        !out(&again).contains("up to date"),
        "a failed build was remembered as done: {}",
        out(&again)
    );
    assert_eq!(code(&again), 1);
}

#[test]
fn a_dry_run_names_what_would_happen_and_runs_none_of_it() {
    let project = Project::new(BASIC);
    let output = project.make(&["-n", "chained"]);
    let text = out(&output);
    assert!(text.contains("hello") && text.contains("chained"), "{text}");
    assert!(!text.contains("hello from a recipe"), "it ran: {text}");
    assert_eq!(code(&output), 0);
}

#[test]
fn parameters_arrive_with_their_declared_defaults() {
    let project = Project::new(
        r#"
local make = oslo.make
make.recipe{ name = "greet", params = { { "--who", default = "world" } },
  run = function(a) print("hello " .. a.who) end }
"#,
    );
    assert!(out(&project.make(&["greet"])).contains("hello world"));
    assert!(out(&project.make(&["greet", "--who", "oslo"])).contains("hello oslo"));
    assert!(out(&project.make(&["greet", "--who=oslo"])).contains("hello oslo"));
}

/// The help has to answer where there is no project, because that is where it gets asked.
#[test]
fn the_help_answers_outside_a_project() {
    let elsewhere = tempfile::tempdir().expect("tempdir");
    let output = Command::new(oslo_bin())
        .args(["make", "--help"])
        .current_dir(elsewhere.path())
        .env("HOME", elsewhere.path())
        .env("XDG_CONFIG_HOME", elsewhere.path().join("config"))
        .stdin(Stdio::null())
        .output()
        .expect("spawn");
    assert_eq!(code(&output), 0);
    assert!(out(&output).contains("--dry-run"), "{}", out(&output));
    assert!(out(&output).contains(".make.lua"), "{}", out(&output));
}

#[test]
fn a_directory_with_no_recipe_file_says_so() {
    let elsewhere = tempfile::tempdir().expect("tempdir");
    let output = Command::new(oslo_bin())
        .args(["make"])
        .current_dir(elsewhere.path())
        .env("HOME", elsewhere.path())
        .env("XDG_CONFIG_HOME", elsewhere.path().join("config"))
        .stdin(Stdio::null())
        .output()
        .expect("spawn");
    assert!(err(&output).contains(".make.lua"), "{}", err(&output));
    assert_eq!(code(&output), 1);
}

/// A recipe declaring outputs and no inputs could never be up to date, so saying so at declaration
/// time beats a build that silently reruns for ever.
#[test]
fn outputs_without_inputs_is_refused_when_the_file_is_read() {
    let project = Project::new(
        r#"
local make = oslo.make
make.recipe{ name = "wrong", outputs = { "out.txt" }, run = function() end }
"#,
    );
    let output = project.make(&["wrong"]);
    assert!(
        err(&output).contains("could never be up to date"),
        "{}",
        err(&output)
    );
    assert_ne!(code(&output), 0);
}

#[test]
fn a_recipe_declared_twice_is_refused() {
    let project = Project::new(
        r#"
local make = oslo.make
make.recipe{ name = "same", run = function() end }
make.recipe{ name = "same", run = function() end }
"#,
    );
    assert!(err(&project.make(&["same"])).contains("declared twice"));
}

/// **A rebuild inside one second is still a rebuild.**
///
/// `oslo.fs.stat` reported `mtime` in whole seconds and the runner compared those, so an input
/// edited and an output written within the same second compared equal — and `oslo make` printed
/// `· art  up to date` while the output still held the previous content. The fast inner loop is
/// exactly where a build finishes inside one second, so it was worst where it mattered most.
///
/// No sleep here, deliberately: the sleep is what used to hide it.
#[test]
fn a_rebuild_inside_one_second_is_not_reported_up_to_date() {
    let project = Project::new(
        r#"
local make = oslo.make
make.recipe{ name = "art", inputs = { "src.txt" }, outputs = { "out.bin" },
  run = function() sh.cp("src.txt", "out.bin") end }
"#,
    );
    project.write("src.txt", "v1\n");
    project.make(&["art"]);
    assert_eq!(project.read("out.bin"), "v1\n");

    project.write("src.txt", "v2\n");
    let second = project.make(&["art"]);
    assert!(
        !out(&second).contains("up to date"),
        "a changed input was called up to date: {}",
        out(&second)
    );
    assert_eq!(
        project.read("out.bin"),
        "v2\n",
        "the stale output survived the rebuild"
    );
}

/// The nanosecond field the runner compares is on every entry, and it is a whole timestamp rather
/// than a sub-second remainder — so two files compare with one `<`.
#[test]
fn stat_reports_whole_nanoseconds() {
    let project = Project::new(
        r#"
local make = oslo.make
make.recipe{ name = "probe", run = function()
  local a = oslo.fs.stat(".make.lua")
  print("ns=" .. tostring(a.mtime_ns) .. " s=" .. tostring(a.mtime))
  print("integer=" .. tostring(math.type(a.mtime_ns)))
  print("consistent=" .. tostring(a.mtime_ns // 1000000000 == a.mtime))
end }
"#,
    );
    let said = out(&project.make(&["probe"]));
    assert!(said.contains("integer=integer"), "{said}");
    assert!(
        said.contains("consistent=true"),
        "mtime_ns is not the same instant as mtime: {said}"
    );
}
