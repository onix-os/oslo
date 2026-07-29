//! The `rush` binary: argument handling, script execution, and the interactive REPL.

mod cli;
mod startup;

// History expansion belongs to the *binary's* prompt, never to the library: it rewrites a line
// before it is parsed, so reaching it from `-c` or a script would let data turn into a different
// command. Declaring it here rather than from `interactive::mod` is what makes that unreachable.
#[path = "interactive/history_expand.rs"]
mod history_expand;

use cli::{Action, Invocation};
use history_expand::Expansion;
use rush::env::Environment;
use rush::env::builtins::run_exit_trap;
use rush::env::options::ShellOption;
use rush::error::{Result, ShellError};
use rush::exec::eval_command_list;
use rush::parser::parse_bash_script;
use std::env;
use std::fs;
use std::io::Read;

fn main() {
    let args: Vec<String> = env::args().collect();

    let invocation = match cli::parse(&args) {
        Ok(inv) => inv,
        Err(exit) => {
            if exit.to_stderr {
                eprintln!("{}", exit.message.trim_end());
            } else {
                println!("{}", exit.message.trim_end());
            }
            std::process::exit(exit.status);
        }
    };

    match invocation.action {
        Action::LuaScript(ref path) => std::process::exit(startup::lua_init::run_lua_script(path)),
        Action::Command(ref text) => run_program(&invocation, text),
        Action::Script(ref path) => match fs::read_to_string(path) {
            Ok(script) => run_program(&invocation, &script),
            Err(_) => {
                eprintln!("rush: {}: No such file or directory", path);
                std::process::exit(127);
            }
        },
        Action::Stdin => {
            if invocation.force_interactive || stdin_is_a_terminal() {
                startup::repl::run_repl();
            } else {
                let mut script = String::new();
                if let Err(e) = std::io::stdin().read_to_string(&mut script) {
                    eprintln!("rush: cannot read standard input: {}", e);
                    std::process::exit(1);
                }
                run_program(&invocation, &script);
            }
        }
    }
}

/// A shell is interactive by default only when it is talking to a person.
///
/// stderr is checked as well as stdin: `rush < script` has a terminal on stderr but must run the
/// script, and `rush 2>log` from a terminal must still prompt.
fn stdin_is_a_terminal() -> bool {
    nix::unistd::isatty(0).unwrap_or(false) && nix::unistd::isatty(2).unwrap_or(false)
}

fn run_program(invocation: &Invocation, script: &str) -> ! {
    let mut env = Environment::new();
    env.shell_name = invocation.name.clone();
    env.set_positional(invocation.positional.clone());
    apply_invocation_options(&mut env, invocation);
    startup::history::register(&mut env);

    // R9.10: a non-interactive shell still reads `$ENV` — that is what POSIX defines it for, and
    // it runs before the program so a function defined there is callable from it.
    if let Some(status) = startup::rc::load_startup_files(&mut env, invocation.force_interactive) {
        std::process::exit(run_exit_trap(&mut env, status));
    }

    // Parsing is kept out of `run_string` so the two kinds of failure stay distinguishable. A
    // script that does not parse never runs at all and exits 2; anything that goes wrong later
    // happened *during* execution, and gets the 127 below.
    let ast = match parse_bash_script(script) {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("rush: {}", e);
            std::process::exit(e.failure_status());
        }
    };

    let status = match absorb_loop_control(eval_command_list(&mut env, &ast)) {
        // The shell's exit status is that of the last command it ran.
        Ok(status) => status,
        Err(e) => exit_error_status(e),
    };
    // R6.5: every way out of a script converges here, so a cleanup handler that fires on a clean
    // finish also fires on `exit 3` and on a fatal error.
    std::process::exit(run_exit_trap(&mut env, status));
}

/// Put the invocation's own flags into the option set, so `$-` describes this shell.
///
/// The two halves are different in kind: `-e`/`-x` are ordinary options a script could also set,
/// while `c` and `s` say where the program came from and no `set` command can change them. Both
/// live in the same bitset because `$-` reports both.
fn apply_invocation_options(env: &mut Environment, invocation: &Invocation) {
    // `Invocation::options` covers both spellings. Walking `set_options` here instead would drop
    // every option that has no letter — `--posix` is exactly that shape.
    for option in invocation.options() {
        env.set_option(option, true);
    }
    match invocation.action {
        Action::Command(_) => env.set_option(ShellOption::CommandString, true),
        Action::Stdin => env.set_option(ShellOption::StdinInput, true),
        _ => {}
    }
    if invocation.force_interactive {
        env.set_option(ShellOption::Interactive, true);
    }
}

/// The status a non-interactive shell ends with after it could not finish its script.
///
/// Returns rather than exiting: the EXIT trap still has to run, because `trap 'rm -f "$tmp"' EXIT`
/// exists precisely for the runs that go wrong.
fn exit_error_status(err: ShellError) -> i32 {
    match err {
        ShellError::Exit(code) => code,
        // Everything else aborted the script mid-flight. `ShellError::fatal_exit_status` decides
        // what that is worth — deliberately *not* the status the same error produces elsewhere:
        // inside a subshell or a pipeline stage it is just a failed command, worth 1, and an
        // interactive shell only sets `$?` and carries on.
        e => {
            let status = e.fatal_exit_status();
            eprintln!("rush: {}", e);
            status
        }
    }
}

/// `break`, `continue` and `return` outside any loop or function are a no-op, not an error.
///
/// They unwind as errors so nested command lists can pass them up; if nothing catches one it has
/// reached the top level, where bash silently ignores it rather than printing a diagnostic.
fn absorb_loop_control(result: Result<i32>) -> Result<i32> {
    match result {
        Err(ShellError::Break(_)) | Err(ShellError::Continue(_)) => Ok(0),
        Err(ShellError::Return(code)) => Ok(code),
        other => other,
    }
}

/// Resolve `!`/`^` history references in a line typed at the prompt.
///
/// `None` means the line must not run: a reference that cannot be resolved is a mistake, and bash
/// answers it by discarding the line, printing the reason, and leaving `$?` untouched — nothing
/// ran, so nothing should have changed. A rewritten line is echoed to stderr first, because the
/// user has to be able to see what `!!` turned into before it takes effect.
fn expand_history(line: &str, history: &[String]) -> Option<String> {
    match history_expand::expand(line, history) {
        Ok(Expansion::Unchanged) => Some(line.to_string()),
        Ok(Expansion::Expanded(expanded)) => {
            eprintln!("{}", expanded);
            // `^a^b` can leave nothing behind; an empty line is not a command.
            if expanded.trim().is_empty() {
                return None;
            }
            Some(expanded)
        }
        Err(err) => {
            eprintln!("rush: {}", err);
            None
        }
    }
}
