//! The interactive loop.
//!
//! It is kept out of `main` because it is where the three startup concerns meet: [`super::rc`]
//! decides what the prompt says and what was sourced before the first one, [`super::history`]
//! decides what is remembered, and [`super::lua_init`] adds the optional Lua layer on top.

use crate::absorb_loop_control;
use crate::expand_history;
use crate::startup::mode::{Line, Mode, ToggleRequest};
use crate::startup::{history, lua_init, mode, rc};
use oslo::Environment;
use oslo::LuaEngine;
use oslo::env::builtins::run_exit_trap;
use oslo::env::options::ShellOption;
use oslo::error::ShellError;
use oslo::exec::{JobManager, eval_command_list};
use oslo::interactive::{InputStatus, OsloHelper};
use oslo::parser::parse_with_aliases;
use rustyline::error::ReadlineError;
use rustyline::history::{History, SearchDirection};
use rustyline::{Editor, history::FileHistory};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

type Repl = Editor<OsloHelper, FileHistory>;

/// One trip round the prompt.
enum Input {
    /// A complete command, the language to run it in, and whether the user asked for it not to be
    /// remembered.
    Command {
        text: String,
        mode: Mode,
        secret: bool,
    },
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
    oslo::exec::pipeline::set_interactive(true);
    // Semantic marks (OSC 133), so a terminal or multiplexer can see where each command's output
    // starts and stops. oslo only declares the boundaries; folding them is the job of whatever
    // owns the grid. See `oslo::interactive::marks`.
    oslo::interactive::marks::enable(true);

    let mut interactive_env = Environment::new();
    // A REPL is interactive and reads its program from the terminal: `$-` says so with `i` and
    // `s`, which is how a sourced script tells an interactive shell from a batch one.
    interactive_env.set_option(ShellOption::Interactive, true);
    interactive_env.set_option(ShellOption::StdinInput, true);
    history::register(&mut interactive_env);

    // `.oslorc` runs before anything else reads a variable, so a `HISTSIZE=` or `PS1=` in it is
    // in force for this session rather than for the next one.
    if let Some(status) = rc::load_startup_files(&mut interactive_env, true) {
        std::process::exit(run_exit_trap(&mut interactive_env, status));
    }

    let env_struct = Arc::new(Mutex::new(interactive_env));
    let lua = match LuaEngine::new() {
        Ok(lua) => lua,
        Err(e) => {
            eprintln!("oslo: lua: {}", e);
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

    // The mode the prompt is reading, and the flag the toggle key sets. Both live for the whole
    // session: switching language is a property of the session, not of one line.
    let mut current = mode::starting_mode(&env_struct.lock().unwrap());
    let toggle = ToggleRequest::new();
    if let Some(key) = mode::toggle_key(&env_struct.lock().unwrap()) {
        rl.bind_sequence(
            key,
            rustyline::EventHandler::Conditional(Box::new(toggle.clone())),
        );
    }

    let mut helper = OsloHelper::new(Arc::clone(&env_struct));
    // R9.10 needs a real `PS2`, and rustyline draws no prompt on a continuation row of its own
    // multi-line editor. `OsloHelper` exposes the switch for exactly this: with editor multi-line
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
            Err(e) => eprintln!("oslo: {}: {}", path.display(), e),
        }
    }
    publish_history(&rl);

    let mut jobs = JobManager::new();
    jobs.setup_signals();

    println!(
        "oslo {} - POSIX Compatible Shell with Lua & Fish-style Features",
        env!("CARGO_PKG_VERSION")
    );
    println!("Type 'exit' or Ctrl-D to exit.");

    let mut last_status = 0;
    let mut eof_count = 0usize;

    loop {
        match read_command(
            &mut rl,
            &env_struct,
            &lua,
            last_status,
            &mut current,
            &toggle,
        ) {
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
            Input::Command { text, mode, secret } => {
                eof_count = 0;
                remember(&mut rl, &settings.file, &text, secret);

                // Handed the command as typed, which is what a `precmd` hook is for: logging it,
                // timing it, or setting a title from it.
                lua.fire_hook("precmd", vec![LuaEngine::hook_arg(&text)]);
                // Everything after this belongs to the command, not to the prompt.
                print!("{}", oslo::interactive::marks::output_start());
                let _ = std::io::Write::flush(&mut std::io::stdout());
                let before = current_directory();

                let res = match mode {
                    // A Lua line leaves `$?` where it was unless it asked otherwise: `oslo.exit`
                    // is the way to choose a status, and a chunk that merely printed something
                    // has not run a command.
                    Mode::Lua => run_lua_line(&lua, &text, last_status),
                    Mode::Shell => {
                        let mut env_guard = env_struct.lock().unwrap();
                        let res = absorb_loop_control(
                            parse_with_aliases(&text, &|n| {
                                env_guard.get_alias(n).map(str::to_string)
                            })
                            .and_then(|ast| eval_command_list(&mut env_guard, &ast)),
                        );
                        drop(env_guard);
                        res
                    }
                };

                // `history -c` cannot reach the editor from inside a builtin, so it leaves a
                // request behind and the loop carries it out.
                if history::take_clear_request() {
                    let _ = rl.clear_history();
                    publish_history(&rl);
                }

                // Fired before the status is acted on, so a `cd` hook sees the directory the
                // command left behind even when that command was the last one of the session.
                let after = current_directory();
                if after != before {
                    lua.fire_hook("cd", vec![LuaEngine::hook_arg(&after)]);
                }
                // The command is over and its status is known: close the block before anything
                // else prints, so nothing that follows lands inside it.
                print!(
                    "{}",
                    oslo::interactive::marks::command_end(res.as_ref().copied().unwrap_or(1))
                );
                let _ = std::io::Write::flush(&mut std::io::stdout());
                if let Ok(status) = res {
                    lua.fire_hook("postcmd", vec![LuaEngine::hook_status(status)]);
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
                        eprintln!("oslo: {}", err);
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
    current: &mut Mode,
    toggle: &ToggleRequest,
) -> Input {
    let mut buffer = String::new();
    let mut secret = false;
    let mut heredoc = HeredocTracker::default();
    // The language *this* command is being read in. It follows `current` until a `!` or `=`
    // prefix sends one line the other way; a continuation line never re-decides.
    let mut reading = *current;
    // Carried across a toggle, so switching language mid-command does not lose what was typed.
    let mut typed = String::new();

    loop {
        let prompt = if buffer.is_empty() {
            primary_prompt(env_struct, lua, last_status, *current)
        } else {
            rc::ps2(&mut env_struct.lock().unwrap())
        };

        // Written before the prompt rather than inside it: the line editor measures the prompt
        // string to know where the line starts, and an OSC in there is counted as visible width.
        print!("{}", oslo::interactive::marks::prompt_start());
        let _ = std::io::Write::flush(&mut std::io::stdout());

        let raw = match rl.readline_with_initial(&prompt, (&typed, "")) {
            Ok(raw) => raw,
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                return Input::Interrupted;
            }
            Err(ReadlineError::Eof) => return Input::Eof,
            Err(err) => {
                eprintln!("oslo: {}", err);
                return Input::Fatal;
            }
        };
        typed.clear();

        // The toggle key accepts the line to hand control back here; nothing was submitted.
        if toggle.take() {
            *current = current.other();
            reading = *current;
            typed = raw;
            continue;
        }

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

        // `=` and `!` are read once, off the first line. They send this one command the other way
        // without touching the mode the prompt goes back to.
        let line = if buffer.is_empty() {
            match mode::classify(*current, line) {
                Line::Normal(text) => text,
                Line::OneOff { mode, text } => {
                    reading = mode;
                    text
                }
            }
        } else {
            line
        };

        // History expansion belongs to shell syntax. In Lua a `!` is `~=`'s other half and a
        // string may hold anything; rewriting a Lua line against the history would corrupt it.
        let expanded = if reading == Mode::Shell && heredoc.expands_history() {
            // Owned so the immutable borrow of the editor ends before the entry is added.
            let previous: Vec<String> = history_entries(rl);
            match expand_history(line, &previous) {
                Some(expanded) => expanded,
                None => return Input::Nothing,
            }
        } else {
            line.to_string()
        };
        // Observed on the expanded text, which is what actually goes into the buffer.
        heredoc.observe(&expanded);

        buffer.push_str(&expanded);
        if is_complete(&buffer, reading) {
            // The frecency table is fed from here rather than from the editor's `validate`,
            // because with editor multi-line off (which is what `PS2` costs) `validate` never
            // sees a multi-line command whole — see `OsloHelper::record_command_use`.
            if let Some(helper) = rl.helper() {
                helper.record_command_use(&buffer);
            }
            return Input::Command {
                text: buffer,
                mode: reading,
                secret,
            };
        }
        buffer.push('\n');
    }
}

/// Whether the line about to be read is the body of a here-document.
///
/// The body of a here-document is **data**, and history expansion rewrites a line before it is
/// parsed — so `cat > note <<EOF` followed by a line containing `!` would silently write some
/// earlier command into the file instead of what was typed. bash does not expand there, and
/// [`oslo::interactive::syntax::opens_here_document`] exists precisely to tell "unfinished
/// because a document is open" from "unfinished because a quote is open". It had no callers at
/// all, so every heredoc body typed at oslo's prompt was being rewritten (PLAN C8).
///
/// One bit of state, given a name because it is the bit that decides whether a typed line is a
/// command or data, and because the loop that owns it needs a terminal to exercise.
#[derive(Default)]
struct HeredocTracker(bool);

impl HeredocTracker {
    /// Whether history expansion may rewrite the next line.
    fn expands_history(&self) -> bool {
        !self.0
    }

    /// Take account of a line that has been accepted into the buffer.
    ///
    /// Only ever turns the bit on. A document whose delimiter arrives part-way through an
    /// unfinished command leaves the rest of that command unexpanded too — the safe direction:
    /// the cost is a `!!` that has to be typed out in full, against a `!` inside a heredoc
    /// quietly becoming somebody else's command.
    fn observe(&mut self, line: &str) {
        self.0 |= oslo::interactive::syntax::opens_here_document(line);
    }
}

/// Whether the parser is satisfied with what has been typed so far.
///
/// A *syntax error* counts as complete: it is the executor's job to report it, and asking for
/// another line would leave the user unable to get the prompt back with no way to see why. The
/// shell's three-way answer comes from the same classifier the editor's validator uses, so the
/// prompt and the loop can never disagree about whether a line is finished.
///
/// Lua answers through its own parser rather than through Lua's `<eof>`-in-the-message trick,
/// which is all the reference implementation's C API can expose. Having our own parser means
/// asking it directly.
fn is_complete(source: &str, mode: Mode) -> bool {
    match mode {
        Mode::Lua => oslo::lua::eval::is_complete(source),
        Mode::Shell => !matches!(
            oslo::interactive::syntax::classify(source),
            InputStatus::Incomplete
        ),
    }
}

fn primary_prompt(
    env_struct: &Arc<Mutex<Environment>>,
    lua: &LuaEngine,
    last_status: i32,
    mode: Mode,
) -> String {
    // Published before the prompt is drawn, so a `PS1` or a Lua prompt function can say which
    // language it is prompting for.
    env_struct
        .lock()
        .unwrap()
        .set_var("OSLO_MODE", mode.name(), false);

    // A Lua prompt is an explicit choice by the user and outranks `PS1`, which in turn outranks
    // the built-in default.
    if let Some(p) = lua.render_prompt() {
        return p;
    }
    // `PS1` is the shell's prompt and describes a shell line; drawing it over a Lua prompt would
    // say `oslo$` in front of something that is not a shell command.
    if mode == Mode::Lua {
        return mode.fallback_prompt().to_string();
    }
    rc::ps1(&mut env_struct.lock().unwrap(), last_status)
}

/// Where the shell is now, for the `cd` hook to compare against.
fn current_directory() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Run one Lua line typed at the prompt.
///
/// A chunk that merely printed something has not run a command, so `$?` stays where it was;
/// `oslo.exit(n)` is how a script chooses a status, and it ends the shell rather than setting one.
fn run_lua_line(lua: &LuaEngine, text: &str, last_status: i32) -> Result<i32, ShellError> {
    match lua.eval_script(text) {
        Ok(()) => Ok(last_status),
        Err(ShellError::Lua(e)) if e.exit.is_some() => Err(ShellError::Exit(e.exit.unwrap_or(0))),
        Err(e) => Err(e),
    }
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
        eprintln!("oslo: {}: {}", path.display(), e);
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
    use super::{HeredocTracker, Mode, is_complete};

    /// The command line that opens a here-document is still a command, so it is expanded; every
    /// line after it is body, so none of them are.
    #[test]
    fn history_expansion_stops_at_a_here_document_body() {
        let mut heredoc = HeredocTracker::default();
        assert!(heredoc.expands_history());

        heredoc.observe("cat > note <<EOF");
        assert!(!heredoc.expands_history(), "the body must not be rewritten");

        heredoc.observe("remember: 10! is 3628800");
        assert!(!heredoc.expands_history());
        heredoc.observe("EOF");
        assert!(!heredoc.expands_history(), "conservative to the end");
    }

    /// An ordinary unfinished command keeps its history expansion: `for i in …` continued onto a
    /// second line is code on both lines, and `!!` there still means what it always did.
    #[test]
    fn an_ordinary_continuation_is_still_expanded() {
        let mut heredoc = HeredocTracker::default();
        for line in ["for i in 1 2 3; do", "  echo $i", "done"] {
            assert!(heredoc.expands_history(), "{line:?}");
            heredoc.observe(line);
        }
        // A here-string takes its body from the same line, so it opens nothing.
        let mut heredoc = HeredocTracker::default();
        heredoc.observe("wc -l <<<\"$text\" &&");
        assert!(heredoc.expands_history());
    }

    /// C10: a multi-line command teaches the ranker about every command in it, not just the one
    /// on the line that happened to parse on its own.
    #[test]
    fn a_multi_line_command_feeds_the_frecency_table() {
        use oslo::Environment;
        use oslo::interactive::OsloHelper;
        use std::sync::{Arc, Mutex};

        // Not interactive, so the table is in memory and no file in `$HOME` is touched.
        let helper = OsloHelper::new(Arc::new(Mutex::new(Environment::new())));
        assert_eq!(helper.frecency_score("zzrepl_a"), 0.0);
        assert_eq!(helper.frecency_score("zzrepl_b"), 0.0);

        helper.record_command_use("for i in 1 2\ndo\n  zzrepl_a $i\n  zzrepl_b\ndone");
        assert!(helper.frecency_score("zzrepl_a") > 0.0);
        assert!(helper.frecency_score("zzrepl_b") > 0.0);
    }

    #[test]
    fn an_unfinished_compound_command_asks_for_more() {
        assert!(!is_complete("for i in 1 2 3; do", Mode::Shell));
        assert!(!is_complete("if true; then", Mode::Shell));
        assert!(!is_complete("while true; do echo hi", Mode::Shell));
        assert!(!is_complete("case x in", Mode::Shell));
        assert!(!is_complete("echo hi |", Mode::Shell));
        assert!(!is_complete("echo \"unterminated", Mode::Shell));
        assert!(!is_complete("x=$(echo hi", Mode::Shell));
    }

    #[test]
    fn a_finished_command_runs() {
        assert!(is_complete("echo hi", Mode::Shell));
        assert!(is_complete("for i in 1 2 3; do echo $i; done", Mode::Shell));
        assert!(is_complete("if true; then echo y; fi", Mode::Shell));
    }

    /// Lua asks its own parser, rather than string-matching `<eof>` in an error message the way
    /// the reference implementation's C API forces it to.
    #[test]
    fn lua_mode_continues_an_unfinished_chunk() {
        assert!(!is_complete("if true then", Mode::Lua));
        assert!(!is_complete("local t = {", Mode::Lua));
        assert!(!is_complete("function f(", Mode::Lua));
        assert!(is_complete("print(1)", Mode::Lua));
        // A real mistake never becomes valid, so asking for another line would wedge the prompt.
        assert!(is_complete("x = = 2", Mode::Lua));
    }

    #[test]
    fn a_real_syntax_error_is_not_a_continuation() {
        // Otherwise a typo would wedge the prompt: every further line is also an error, and
        // there is no way back to PS1.
        assert!(is_complete("echo )", Mode::Shell));
        assert!(is_complete("fi", Mode::Shell));
        assert!(is_complete("done", Mode::Shell));
    }
}
