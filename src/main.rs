//! The `rush` binary: argument handling, script execution, and the interactive REPL.

mod cli;

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
use rush::exec::{JobManager, eval_command_list};
use rush::interactive::RushHelper;
use rush::lua::LuaEngine;
use rush::parser::parse_bash_script;
use rustyline::Editor;
use rustyline::error::ReadlineError;
use std::env;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
        Action::LuaScript(ref path) => run_lua_script(path),
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
                run_repl();
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
    for letter in invocation.set_options.chars() {
        if let Some(option) = ShellOption::from_letter(letter) {
            env.set_option(option, true);
        }
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

fn run_lua_script(path: &str) -> ! {
    let env = Arc::new(Mutex::new(Environment::new()));
    let lua = LuaEngine::new().expect("Failed to initialize Lua");
    let _ = lua.setup_bindings(env);
    if let Err(e) = lua.load_file(path) {
        eprintln!("rush: lua error: {}", e);
        std::process::exit(1);
    }
    std::process::exit(0);
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

fn run_repl() -> ! {
    // Everything downstream that behaves differently for a person than for a script — the job
    // notice, whether a background job keeps the terminal's stdin — reads this.
    // (Addressed by path rather than a re-export: `exec::mod` is being edited elsewhere.)
    rush::exec::pipeline::set_interactive(true);

    let mut interactive_env = Environment::new();
    // A REPL is interactive and reads its program from the terminal: `$-` says so with `i` and
    // `s`, which is how a sourced script tells an interactive shell from a batch one.
    interactive_env.set_option(ShellOption::Interactive, true);
    interactive_env.set_option(ShellOption::StdinInput, true);
    let env_struct = Arc::new(Mutex::new(interactive_env));
    let lua = LuaEngine::new().expect("Failed to initialize Lua engine");
    let _ = lua.setup_bindings(Arc::clone(&env_struct));

    // Try loading ~/.config/rush/init.lua
    if let Some(home) = env::var_os("HOME") {
        let init_path = PathBuf::from(home).join(".config/rush/init.lua");
        if init_path.exists()
            && let Some(path) = init_path.to_str()
        {
            let _ = lua.load_file(path);
        }
    }

    // History is added by hand below, not automatically: what belongs in the history is the line
    // *after* history expansion, so that `!!` recalls the command it stood for rather than itself.
    // Repeats are kept rather than folded into the previous entry. rustyline drops a consecutive
    // duplicate by default, which would silently renumber every later event and make `!-2` point
    // one line too far back — bash's default `HISTCONTROL` keeps them, and `!n` only means
    // anything if the numbering agrees.
    let config = rustyline::Config::builder()
        .auto_add_history(false)
        .history_ignore_dups(false)
        .expect("history duplicate policy")
        .completion_type(rustyline::CompletionType::Circular)
        .build();

    let mut rl = Editor::with_config(config).expect("Failed to initialize line editor");
    let helper = RushHelper::new(Arc::clone(&env_struct));
    rl.set_helper(Some(helper));

    let history_path = env::var_os("HOME").map(|h| PathBuf::from(h).join(".rush_history"));

    if let Some(ref path) = history_path {
        let _ = rl.load_history(path);
    }

    let mut jobs = JobManager::new();
    jobs.setup_signals();

    println!(
        "rush {} - POSIX Compatible Shell with Lua & Fish-style Features",
        env!("CARGO_PKG_VERSION")
    );
    println!("Type 'exit' or Ctrl-D to exit.");

    let mut last_status = 0;

    loop {
        let left_prompt = if let Some(p) = lua.render_prompt() {
            p
        } else {
            rush::interactive::prompt::render_default_left_prompt(last_status)
        };

        let readline = rl.readline(&left_prompt);
        match readline {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Owned so the immutable borrow of the editor ends before the entry is added.
                let history: Vec<String> = rl.history().iter().cloned().collect();
                let Some(line) = expand_history(trimmed, &history) else {
                    continue;
                };

                let _ = rl.add_history_entry(&line);

                let mut env_guard = env_struct.lock().unwrap();
                let res = absorb_loop_control(
                    parse_bash_script(&line)
                        .and_then(|ast| eval_command_list(&mut env_guard, &ast)),
                );
                drop(env_guard);

                match res {
                    Ok(status) => {
                        last_status = status;
                    }
                    Err(ShellError::Exit(code)) => {
                        if let Some(ref path) = history_path {
                            let _ = rl.save_history(path);
                        }
                        // R6.5: `exit` from the prompt is still a shell ending, so the EXIT trap
                        // fires here too. A REPL that skipped it would leave behind exactly the
                        // temp files an interactive session accumulates most of.
                        let mut env_guard = env_struct.lock().unwrap();
                        let code = run_exit_trap(&mut env_guard, code);
                        drop(env_guard);
                        std::process::exit(code);
                    }
                    Err(err) => {
                        // An interactive shell survives what would kill a script: the error
                        // becomes `$?` (1, or 2 for a syntax error) and the prompt comes back.
                        last_status = err.failure_status();
                        eprintln!("rush: {}", err);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
            }
            Err(ReadlineError::Eof) => {
                println!("exit");
                break;
            }
            Err(err) => {
                eprintln!("Error: {:?}", err);
                break;
            }
        }
    }

    if let Some(ref path) = history_path {
        let _ = rl.save_history(path);
    }
    // End of input (Ctrl-D) is the other way a REPL ends, and POSIX makes no distinction: the
    // EXIT trap fires on both.
    let mut env_guard = env_struct.lock().unwrap();
    let last_status = run_exit_trap(&mut env_guard, last_status);
    drop(env_guard);
    std::process::exit(last_status);
}
