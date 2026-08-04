//! The `oslo` binary: argument handling, script execution, and the interactive REPL.

mod cli;
mod startup;

// History expansion belongs to the *binary's* prompt, never to the library: it rewrites a line
// before it is parsed, so reaching it from `-c` or a script would let data turn into a different
// command. Declaring it here rather than from `interactive::mod` is what makes that unreachable.
#[path = "interactive/history_expand.rs"]
mod history_expand;

use cli::{Action, Invocation};
use history_expand::Expansion;
use oslo::env::Environment;
use oslo::env::builtins::run_exit_trap;
use oslo::env::options::ShellOption;
use oslo::error::{Result, ShellError};
use oslo::exec::eval_command_list;
use oslo::parser::parse_with_aliases;
use startup::language::{self, Language};
use std::env;
use std::fs;
use std::io::Read;

/// Undo the Rust runtime's `SIG_IGN` for SIGPIPE, which a shell must not have.
///
/// Rust ignores SIGPIPE before `main` so that a write to a closed pipe surfaces as an `EPIPE`
/// error rather than killing the process. For an ordinary program that is a kindness; for a shell
/// it is a hang. `oslo -c 'while :; do echo x; done' | head -1` ran for ever, because nothing ever
/// told the loop that its reader had gone — bash exits immediately. It also made `kill -s PIPE $$`
/// a no-op, which is how modernish detects the condition and why it refuses to load `var/loop`
/// on such a shell.
///
/// Children already got `SIG_DFL` back (see [`oslo::exec::JobManager`]); this is the shell's own
/// disposition, and it has to be set here rather than in the library because the test binary links
/// the library and would arm *itself* to die on any write to a closed pipe.
fn restore_default_sigpipe() {
    // Safety: called before any thread is started and before anything is written, and `signal(2)`
    // with `SIG_DFL` touches nothing but this process's own disposition table.
    unsafe {
        let _ = nix::sys::signal::signal(
            nix::sys::signal::Signal::SIGPIPE,
            nix::sys::signal::SigHandler::SigDfl,
        );
    }
}

/// Report how many structured pipeline edges this run planned, when asked.
///
/// The differential corpus runs oslo as a subprocess, so the in-process counter is not visible to
/// it. With `OSLO_AUDIT_STRUCTURED=1` the count is written to stderr as the process ends, and the
/// corpus asserts it is zero for every POSIX script — which is what turns "structure cannot affect
/// a script written before oslo existed" into a test rather than a promise.
fn report_structured_audit() {
    if std::env::var("OSLO_AUDIT_STRUCTURED").is_err() {
        return;
    }
    extern "C" fn report() {
        // `eprintln!` is not signal-safe, but `atexit` handlers run on a normal return from the
        // process rather than from a handler, so this is an ordinary write.
        eprintln!(
            "oslo-audit: structured-edges={}",
            oslo::data::entered_structured_path()
        );
    }
    // Registered rather than called at the end of `main`: nearly every path out of this shell is a
    // `process::exit` from somewhere deeper, and a report that only fires on one of them would
    // give a clean answer for the wrong reason.
    // SAFETY: `report` is `extern "C"`, takes nothing, returns nothing, and touches only an
    // atomic and stderr.
    unsafe {
        nix::libc::atexit(report);
    }
}

fn main() {
    // Before any thread exists, as the safety note on the function requires.
    restore_default_sigpipe();
    report_structured_audit();
    // The names that can carry structure. Declared once, here, for every mode the shell runs in —
    // a script and a prompt must agree about what `df` is.
    oslo::data::tools::register_all();

    // The shell runs on a stack oslo chose rather than one it inherited; see
    // [`oslo::INTERPRETER_STACK`]. `main` itself does nothing afterwards but wait.
    //
    // Signals need care, and getting it wrong is subtle. `kill` directed at a *process* is
    // delivered to any one thread that is not blocking it, so with `main` merely parked in
    // `join` the kernel was free to hand it there — and `kill -USR1 $$` would then return to the
    // shell before its own trap had run, printing the next command's output first. Blocking
    // everything here and unblocking on the worker leaves exactly one candidate thread, which is
    // what restores the single-threaded ordering the rest of the shell is written against.
    let inherited = block_every_signal();
    let worker = std::thread::Builder::new()
        .name("oslo".to_string())
        .stack_size(oslo::INTERPRETER_STACK)
        .spawn(move || {
            // Anything raised in the gap is merely pending, and arrives the moment this returns.
            restore_signal_mask(&inherited);
            dispatch();
        })
        .expect("oslo: cannot start");
    if worker.join().is_err() {
        // The worker panicked and has already printed its message.
        std::process::exit(2);
    }
}

/// Block every signal on the calling thread, answering the mask that was in force.
fn block_every_signal() -> nix::sys::signal::SigSet {
    let mut previous = nix::sys::signal::SigSet::empty();
    let _ = nix::sys::signal::pthread_sigmask(
        nix::sys::signal::SigmaskHow::SIG_SETMASK,
        Some(&nix::sys::signal::SigSet::all()),
        Some(&mut previous),
    );
    previous
}

/// Put a saved mask back, so the shell starts with whatever its caller handed it.
fn restore_signal_mask(mask: &nix::sys::signal::SigSet) {
    let _ = nix::sys::signal::pthread_sigmask(
        nix::sys::signal::SigmaskHow::SIG_SETMASK,
        Some(mask),
        None,
    );
}

fn dispatch() {
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
        // `oslo history …` — reached only when no file of that name exists, so this never takes
        // an invocation a script could have wanted. See `cli::tools::as_operand`.
        Action::Tool(ref name, ref args) => {
            let tool = cli::tools::from_name(name).expect("the parser only names tools it found");
            std::process::exit(cli::tools::run(tool, args));
        }
        // **`-c` is always shell.** Every `sh -c` idiom in the world depends on it, and no amount
        // of detection is worth being wrong about that one.
        Action::Command(ref text) => run_program(&invocation, text),
        // A script operand names a file whose language is worked out from the file itself.
        Action::Script(ref path) => match fs::read_to_string(path) {
            Ok(script) => match language::detect(Some(path), &script) {
                Language::Lua => std::process::exit(startup::lua_init::run_lua_source(
                    &script,
                    path,
                    &invocation.positional,
                )),
                Language::Shell => run_program(&invocation, &script),
            },
            Err(_) => {
                eprintln!("oslo: {}: No such file or directory", path);
                std::process::exit(127);
            }
        },
        Action::Stdin => {
            if invocation.force_interactive || stdin_is_a_terminal() {
                startup::repl::run_repl();
            } else {
                let mut script = String::new();
                if let Err(e) = std::io::stdin().read_to_string(&mut script) {
                    eprintln!("oslo: cannot read standard input: {}", e);
                    std::process::exit(1);
                }
                match language::detect(None, &script) {
                    Language::Lua => std::process::exit(startup::lua_init::run_lua_source(
                        &script,
                        "stdin",
                        &invocation.positional,
                    )),
                    Language::Shell => run_program(&invocation, &script),
                }
            }
        }
    }
}

/// A shell is interactive by default only when it is talking to a person.
///
/// stderr is checked as well as stdin: `oslo < script` has a terminal on stderr but must run the
/// script, and `oslo 2>log` from a terminal must still prompt.
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
    let ast = match parse_with_aliases(script, &|n| env.get_alias(n).map(str::to_string)) {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("oslo: {}", e);
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
            eprintln!("oslo: {}", e);
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
            eprintln!("oslo: {}", err);
            None
        }
    }
}
