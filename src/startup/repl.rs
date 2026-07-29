//! The interactive loop.
//!
//! It is kept out of `main` because it is where the three startup concerns meet: [`super::rc`]
//! decides what the prompt says and what was sourced before the first one, [`super::history`]
//! decides what is remembered, and [`super::lua_init`] adds the optional Lua layer on top.

use crate::absorb_loop_control;
use crate::expand_history;
use crate::startup::{history, lua_init, rc};
use rush::Environment;
use rush::LuaEngine;
use rush::env::builtins::run_exit_trap;
use rush::env::options::ShellOption;
use rush::error::ShellError;
use rush::exec::{JobManager, eval_command_list};
use rush::interactive::{InputStatus, RushHelper};
use rush::parser::parse_bash_script;
use rustyline::error::ReadlineError;
use rustyline::history::{History, SearchDirection};
use rustyline::{Editor, history::FileHistory};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

type Repl = Editor<RushHelper, FileHistory>;

/// One trip round the prompt.
enum Input {
    /// A complete command, and whether the user asked for it not to be remembered.
    Command { text: String, secret: bool },
    /// Nothing to run: a blank line, or a history reference that did not resolve.
    Nothing,
    /// Ctrl-C. The partial command is dropped and the prompt comes back.
    Interrupted,
    /// End of input. `$IGNOREEOF` may ask for this to be ignored.
    Eof,
    /// The editor failed. Unlike an end of input this is never ignored: retrying would print the
    /// same error forever.
    Fatal,
}

pub fn run_repl() -> ! {
    // Everything downstream that behaves differently for a person than for a script — the job
    // notice, whether a background job keeps the terminal's stdin — reads this.
    // (Addressed by path rather than a re-export: `exec::mod` is being edited elsewhere.)
    rush::exec::pipeline::set_interactive(true);

    let mut interactive_env = Environment::new();
    // A REPL is interactive and reads its program from the terminal: `$-` says so with `i` and
    // `s`, which is how a sourced script tells an interactive shell from a batch one.
    interactive_env.set_option(ShellOption::Interactive, true);
    interactive_env.set_option(ShellOption::StdinInput, true);
    history::register(&mut interactive_env);

    // `.rushrc` runs before anything else reads a variable, so a `HISTSIZE=` or `PS1=` in it is
    // in force for this session rather than for the next one.
    if let Some(status) = rc::load_startup_files(&mut interactive_env, true) {
        std::process::exit(run_exit_trap(&mut interactive_env, status));
    }

    let env_struct = Arc::new(Mutex::new(interactive_env));
    let lua = match LuaEngine::new() {
        Ok(lua) => lua,
        Err(e) => {
            eprintln!("rush: lua: {}", e);
            std::process::exit(1);
        }
    };
    if lua_init::install_bindings(&lua, Arc::clone(&env_struct)) {
        let init_path = lua_init::init_lua_path(&env_struct.lock().unwrap());
        if let Some(path) = init_path {
            lua_init::load_init_lua(&lua, &path);
        }
    }

    let settings = history::settings(&env_struct.lock().unwrap());
    let mut rl = build_editor(&settings);
    let mut helper = RushHelper::new(Arc::clone(&env_struct));
    // R9.10 needs a real `PS2`, and rustyline draws no prompt on a continuation row of its own
    // multi-line editor. `RushHelper` exposes the switch for exactly this: with editor multi-line
    // off, an unfinished line comes back from `readline` and `read_command` below asks for the
    // next one under `PS2`, the way every POSIX shell does.
    helper.set_editor_multiline(false);
    rl.set_helper(Some(helper));
    if let Some(ref path) = settings.file {
        // A missing history file on a first run is not worth a diagnostic; anything else is,
        // because a history that silently fails to load looks exactly like one that was lost.
        match rl.load_history(path) {
            Ok(()) => {}
            Err(ReadlineError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("rush: {}: {}", path.display(), e),
        }
    }
    publish_history(&rl);

    let mut jobs = JobManager::new();
    jobs.setup_signals();

    println!(
        "rush {} - POSIX Compatible Shell with Lua & Fish-style Features",
        env!("CARGO_PKG_VERSION")
    );
    println!("Type 'exit' or Ctrl-D to exit.");

    let mut last_status = 0;
    let mut eof_count = 0usize;

    loop {
        match read_command(&mut rl, &env_struct, &lua, last_status) {
            Input::Nothing | Input::Interrupted => continue,
            Input::Fatal => break,
            Input::Eof => {
                eof_count += 1;
                match ignore_eof_limit(&env_struct) {
                    Some(limit) if eof_count <= limit => {
                        println!("Use \"exit\" to leave the shell.");
                        continue;
                    }
                    _ => {
                        println!("exit");
                        break;
                    }
                }
            }
            Input::Command { text, secret } => {
                eof_count = 0;
                remember(&mut rl, &settings.file, &text, secret);

                let mut env_guard = env_struct.lock().unwrap();
                let res = absorb_loop_control(
                    parse_bash_script(&text)
                        .and_then(|ast| eval_command_list(&mut env_guard, &ast)),
                );
                drop(env_guard);

                // `history -c` cannot reach the editor from inside a builtin, so it leaves a
                // request behind and the loop carries it out.
                if history::take_clear_request() {
                    let _ = rl.clear_history();
                    publish_history(&rl);
                }

                match res {
                    Ok(status) => last_status = status,
                    Err(ShellError::Exit(code)) => {
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
        }
    }

    // End of input (Ctrl-D) is the other way a REPL ends, and POSIX makes no distinction: the
    // EXIT trap fires on both.
    let mut env_guard = env_struct.lock().unwrap();
    let last_status = run_exit_trap(&mut env_guard, last_status);
    drop(env_guard);
    std::process::exit(last_status);
}

/// R9.11: the editor's history configuration, which used to be left entirely at its defaults.
fn build_editor(settings: &history::Settings) -> Repl {
    // History is added by hand in `remember`, not automatically: what belongs in the history is
    // the line *after* history expansion, so that `!!` recalls the command it stood for rather
    // than itself, and a multi-line command belongs there as one entry rather than three.
    // Repeats are kept rather than folded into the previous entry. rustyline drops a consecutive
    // duplicate by default, which would silently renumber every later event and make `!-2` point
    // one line too far back — bash's default `HISTCONTROL` keeps them, and `!n` only means
    // anything if the numbering agrees.
    let config = rustyline::Config::builder()
        .auto_add_history(false)
        .history_ignore_dups(false)
        .expect("history duplicate policy")
        // rustyline's own default is 100 entries, which loses a working day's commands.
        .max_history_size(settings.max_size)
        .expect("history size")
        // Honoured for anything rustyline adds itself; `history::is_secret` covers the entries
        // this file adds by hand, which is all of them.
        .history_ignore_space(true)
        .completion_type(rustyline::CompletionType::Circular)
        .build();
    Editor::with_config(config).expect("Failed to initialize line editor")
}

/// Read one complete command, continuing onto further lines while the parser wants more.
///
/// This is where `PS2` earns its keep: a command that is not finished gets the continuation
/// prompt, instead of the hard syntax error `for i in 1 2 3` used to produce the moment you
/// pressed Enter.
fn read_command(
    rl: &mut Repl,
    env_struct: &Arc<Mutex<Environment>>,
    lua: &LuaEngine,
    last_status: i32,
) -> Input {
    let mut buffer = String::new();
    let mut secret = false;

    loop {
        let prompt = if buffer.is_empty() {
            primary_prompt(env_struct, lua, last_status)
        } else {
            rc::ps2(&mut env_struct.lock().unwrap())
        };

        let raw = match rl.readline(&prompt) {
            Ok(raw) => raw,
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                return Input::Interrupted;
            }
            Err(ReadlineError::Eof) => return Input::Eof,
            Err(err) => {
                eprintln!("rush: {}", err);
                return Input::Fatal;
            }
        };

        if buffer.is_empty() {
            if raw.trim().is_empty() {
                return Input::Nothing;
            }
            // The leading space that asks for a line not to be remembered is a property of the
            // line *as typed*, so it has to be read before anything trims or rewrites it.
            secret = history::is_secret(&raw);
        }

        // Only the first line is trimmed. A continuation line goes in exactly as typed, because
        // the body of a here-document is data: `cat <<EOF` followed by an indented line must keep
        // its indentation.
        let line = if buffer.is_empty() {
            raw.trim()
        } else {
            raw.as_str()
        };

        // Owned so the immutable borrow of the editor ends before the entry is added.
        let previous: Vec<String> = history_entries(rl);
        let Some(expanded) = expand_history(line, &previous) else {
            return Input::Nothing;
        };

        buffer.push_str(&expanded);
        if is_complete(&buffer) {
            return Input::Command {
                text: buffer,
                secret,
            };
        }
        buffer.push('\n');
    }
}

/// Whether the parser is satisfied with what has been typed so far.
///
/// A *syntax error* counts as complete: it is the executor's job to report it, and asking for
/// another line would leave the user unable to get the prompt back with no way to see why. The
/// three-way answer comes from the same classifier the editor's validator uses, so the prompt
/// and the loop can never disagree about whether a line is finished.
fn is_complete(source: &str) -> bool {
    !matches!(
        rush::interactive::syntax::classify(source),
        InputStatus::Incomplete
    )
}

fn primary_prompt(
    env_struct: &Arc<Mutex<Environment>>,
    lua: &LuaEngine,
    last_status: i32,
) -> String {
    // A Lua prompt is an explicit choice by the user and outranks `PS1`, which in turn outranks
    // the built-in default.
    if let Some(p) = lua.render_prompt() {
        return p;
    }
    rc::ps1(&mut env_struct.lock().unwrap(), last_status)
}

/// `$IGNOREEOF`: how many end-of-file characters to ignore before ending the shell.
///
/// `None` means the variable is unset and Ctrl-D exits immediately, as it always has. bash's
/// documented fallback for a value that is not a number is 10.
fn ignore_eof_limit(env_struct: &Arc<Mutex<Environment>>) -> Option<usize> {
    let guard = env_struct.lock().unwrap();
    let raw = guard.get_var("IGNOREEOF")?;
    Some(raw.trim().parse::<usize>().unwrap_or(10))
}

/// Add a command to the history, and to the history *file*, before it runs.
///
/// Appending rather than rewriting is the fix for the third of R9.11's defects: `save_history`
/// writes the whole file, so two sessions open at once each ended with only their own commands.
/// Writing before the command runs is deliberate too — a command that exits the shell, or hangs
/// until it is killed, is exactly the one you want to find in the history afterwards.
fn remember(rl: &mut Repl, file: &Option<PathBuf>, text: &str, secret: bool) {
    if secret {
        return;
    }
    let _ = rl.add_history_entry(text);
    publish_history(rl);
    if let Some(path) = file
        && let Err(e) = rl.append_history(path)
    {
        eprintln!("rush: {}: {}", path.display(), e);
    }
}

fn history_entries(rl: &Repl) -> Vec<String> {
    let history = rl.history();
    (0..history.len())
        .filter_map(|i| {
            history
                .get(i, SearchDirection::Forward)
                .ok()
                .flatten()
                .map(|r| r.entry.into_owned())
        })
        .collect()
}

/// Hand the `history` builtin the entries it prints.
fn publish_history(rl: &Repl) {
    history::publish(history_entries(rl));
}

#[cfg(test)]
mod tests {
    use super::is_complete;

    #[test]
    fn an_unfinished_compound_command_asks_for_more() {
        assert!(!is_complete("for i in 1 2 3; do"));
        assert!(!is_complete("if true; then"));
        assert!(!is_complete("while true; do echo hi"));
        assert!(!is_complete("case x in"));
        assert!(!is_complete("echo hi |"));
        assert!(!is_complete("echo \"unterminated"));
        assert!(!is_complete("x=$(echo hi"));
    }

    #[test]
    fn a_finished_command_runs() {
        assert!(is_complete("echo hi"));
        assert!(is_complete("for i in 1 2 3; do echo $i; done"));
        assert!(is_complete("if true; then echo y; fi"));
    }

    #[test]
    fn a_real_syntax_error_is_not_a_continuation() {
        // Otherwise a typo would wedge the prompt: every further line is also an error, and
        // there is no way back to PS1.
        assert!(is_complete("echo )"));
        assert!(is_complete("fi"));
        assert!(is_complete("done"));
    }
}
