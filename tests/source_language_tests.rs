//! `source` detects the language of what it is given.
//!
//! `oslo script.lua` has always asked `language::detect` what it is holding. `source script.lua` did
//! not, so a Lua file reached the shell parser and came back as `syntax error at line 2 col 20` —
//! in a shell whose own rule is that Lua never needs an opt-in flag.
//!
//! The gap that made it worth fixing: a tool a config registers works at the prompt and does not
//! exist in a script, because `init.lua` is read by the REPL and by nothing else. Sourcing the
//! declarations is how a script asks for them, and it could not.

mod common;

use std::io::Write;

fn write(dir: &std::path::Path, name: &str, text: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).expect("fixture");
    file.write_all(text.as_bytes()).expect("write");
    path
}

const TOOLS: &str = r#"
oslo.register_tool{
  name     = "hosts",
  accepts  = "nothing",
  produces = "rows",
  rows = function(argv)
    return { { host = "alpha", ip = "10.0.0.1" }, { host = "beta", ip = "192.168.0.9" } }
  end,
}
"#;

/// **The gap.** A registered tool used to exist only at an interactive prompt.
#[test]
fn a_script_can_source_the_tools_it_needs() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "tools.lua", TOOLS);

    let run = common::run_in(dir.path(), "source tools.lua\nhosts | length");
    assert_eq!(run.out(), "2", "stderr: {}", run.stderr);
    assert_eq!(run.status, 0);
}

/// And the verbs apply to it, which is the whole reason a script wants the tool rather than a
/// function that prints.
#[test]
fn a_sourced_tool_is_a_stage_like_any_other() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "tools.lua", TOOLS);

    let run = common::run_in(
        dir.path(),
        "source tools.lua\nhosts | where 'ip:match(\"^10%.\")' | cols host",
    );
    assert_eq!(run.out(), "alpha", "stderr: {}", run.stderr);
}

/// **Sourcing shell is untouched**, which is the whole POSIX question: `.` and `source` are how
/// every script on the machine reads its own fragments, and a file that was shell before is shell
/// now.
#[test]
fn sourcing_a_shell_file_is_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        dir.path(),
        "frag.sh",
        "GREETING=hello\ngreet() { echo \"$GREETING $1\"; }\n",
    );

    let run = common::run_in(dir.path(), ". ./frag.sh\ngreet world\necho \"$GREETING\"");
    assert_eq!(run.out(), "hello world\nhello", "stderr: {}", run.stderr);
    assert_eq!(run.status, 0);
}

/// A shell fragment with no extension and no shebang stays shell. Detection resolves *every*
/// ambiguous case that way, which is the direction a POSIX shell has to fail in.
#[test]
fn a_shell_fragment_with_no_extension_stays_shell() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "frag", "X=1\nexport X\n");

    let run = common::run_in(dir.path(), ". ./frag\necho \"$X\"");
    assert_eq!(run.out(), "1", "stderr: {}", run.stderr);
}

/// A Lua file that raises reports the failure and leaves the script running, exactly as a shell
/// fragment that fails does.
#[test]
fn a_broken_lua_file_does_not_take_the_script_with_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "bad.lua", "error('deliberate')\n");

    let run = common::run_in(dir.path(), "source bad.lua\necho ALIVE");
    assert!(run.out().contains("ALIVE"), "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("deliberate"),
        "the failure is reported: {}",
        run.stderr
    );
}
