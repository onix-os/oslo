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
                // Streamed: a file is what polyglots and partial execution are about.
                Language::Shell => run_program_reading(&invocation, &script, Reading::Streamed),
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
                    // A pipe is a stream, and bash and dash both run what they have read of one
                    // before a later syntax error stops them.
                    Language::Shell => run_program_reading(&invocation, &script, Reading::Streamed),
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

/// How a program's text reaches the evaluator.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reading {
    /// **A command at a time**, as a shell reads a file or a stream.
    ///
    /// POSIX says the shell reads its input and parses and executes it; every shell does so a
    /// command at a time, and scripts are written for that. Two things depend on it:
    ///
    /// - **`#!/bin/sh` polyglots.** `/usr/bin/pnmflip` and nine of its netpbm siblings are Perl
    ///   below an `exec perl -x -S -- "$0"` on line 23. Reading a command at a time reaches the
    ///   `exec`, and the process is replaced before the Perl is ever looked at. Parsing the file
    ///   first meant a syntax error at line 51 and nothing running at all.
    /// - **Partial execution.** A syntax error on line 500 leaves the first 499 lines run, which
    ///   is what bash, dash and busybox all do. oslo used to run none of them.
    ///
    /// `oslo -n` is still how you check a whole file without running it — that is what it is for,
    /// and it is what makes doing this implicitly on every run unnecessary.
    Streamed,
    /// All at once. `-c` only, because bash and dash both parse a `-c` string whole:
    /// `sh -c 'echo RAN; if true; then'` prints nothing in either.
    Whole,
}

fn run_program(invocation: &Invocation, script: &str) -> ! {
    run_program_reading(invocation, script, Reading::Whole)
}

fn run_program_reading(invocation: &Invocation, script: &str, reading: Reading) -> ! {
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

    // `-c` only, and only when `$OSLO_ALLHIST` asks: a script's *contents* are not commands
    // anybody typed, and recording them would put every line of every `#!/bin/sh` script on the
    // machine into the history the moment oslo becomes `/bin/sh`.
    if reading == Reading::Whole && startup::history::record_commands() {
        startup::history::record_command(&env, script);
    }

    let status = match reading {
        Reading::Whole => run_whole(&mut env, script),
        Reading::Streamed => run_streamed(&mut env, script),
    };
    // R6.5: every way out of a script converges here, so a cleanup handler that fires on a clean
    // finish also fires on `exit 3` and on a fatal error.
    std::process::exit(run_exit_trap(&mut env, status));
}

/// Parse the whole program, then run it.
fn run_whole(env: &mut Environment, script: &str) -> i32 {
    // Parsing is kept out of `run_string` so the two kinds of failure stay distinguishable. A
    // program that does not parse never runs at all and exits 2; anything that goes wrong later
    // happened *during* execution, and gets the 127 below.
    match run_chunk(env, script) {
        Ok(status) => status,
        Err(status) => status,
    }
}

/// Run a file or a stream: whole if it parses, a command at a time if it does not.
///
/// **The split is a performance one, and it changes nothing observable.** A program that parses
/// has no syntax error to be partial about, so running it whole and running it a command at a time
/// produce the same thing — and parsing once is enormously cheaper. Classifying after every line
/// re-parses a growing buffer, which is quadratic in the length of a single command: a one-megabyte
/// here-document is one command, and `tests/redirection_tests.rs` timed out at ten seconds on it.
///
/// Only a program that does *not* parse takes the slow path, and there the cost is the point: it
/// is the only way to run the lines before the mistake, which is what
/// [`Reading::Streamed`] exists for.
fn run_streamed(env: &mut Environment, script: &str) -> i32 {
    if let Ok(ast) = parse_with_aliases(script, &|n| env.get_alias(n).map(str::to_string)) {
        return match absorb_loop_control(eval_command_list(env, &ast)) {
            Ok(status) => status,
            Err(e) => exit_error_status(e),
        };
    }
    run_line_at_a_time(env, script)
}

/// Read a command at a time, running each before looking at the next.
///
/// The buffer is emptied after every complete command, so each re-parse covers one command's worth
/// of text rather than the whole file.
fn run_line_at_a_time(env: &mut Environment, script: &str) -> i32 {
    use oslo::interactive::syntax::{InputStatus, classify};

    let mut buffer = String::new();
    let mut status = 0;
    for line in script.split_inclusive('\n') {
        buffer.push_str(line);
        match classify(&buffer) {
            // The command has not ended yet — an open `if`, a heredoc still looking for its
            // delimiter, a trailing `|`.
            InputStatus::Incomplete => continue,
            // Not shell at all. Stop reading and let the parse below report it the usual way,
            // *after* everything already read has run — which is the whole point.
            InputStatus::Invalid => break,
            InputStatus::Complete => {
                // A blank line or a comment parses to nothing; there is no command to run and no
                // status to take from it.
                if !buffer.trim().is_empty() {
                    match run_chunk(env, &buffer) {
                        Ok(next) => status = next,
                        Err(failed) => return failed,
                    }
                }
                buffer.clear();
            }
        }
    }

    // Whatever is left never became a complete command: an unterminated `if`, or the invalid text
    // the loop stopped at. Running it produces the same diagnostic a whole-file parse would.
    if !buffer.trim().is_empty() {
        match run_chunk(env, &buffer) {
            Ok(next) => status = next,
            Err(failed) => return failed,
        }
    }
    status
}

/// Parse one piece of program text and run it.
///
/// `Err` carries the status the shell should exit with; the caller stops there.
fn run_chunk(env: &mut Environment, source: &str) -> std::result::Result<i32, i32> {
    let ast = match parse_with_aliases(source, &|n| env.get_alias(n).map(str::to_string)) {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("oslo: {}", e);
            return Err(e.failure_status());
        }
    };
    match absorb_loop_control(eval_command_list(env, &ast)) {
        // The shell's exit status is that of the last command it ran.
        Ok(status) => Ok(status),
        Err(e) => Err(exit_error_status(e)),
    }
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
