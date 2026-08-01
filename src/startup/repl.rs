//! The interactive loop.
//!
//! It is kept out of `main` because it is where the three startup concerns meet: [`super::rc`]
//! decides what the prompt says and what was sourced before the first one, [`super::history`]
//! decides what is remembered, and [`super::lua_init`] adds the optional Lua layer on top.

use crate::absorb_loop_control;
use crate::startup::mode::{Mode, ToggleRequest};
use crate::startup::read::{Input, read_command};
use crate::startup::{config, history, history_db, keybind, lua_init, mode, rc};
use oslo::Environment;
use oslo::LuaEngine;
use oslo::env::builtins::run_exit_trap;
use oslo::env::options::ShellOption;
use oslo::error::ShellError;
use oslo::exec::{JobManager, eval_command_list};
use oslo::interactive::OsloHelper;
use oslo::parser::parse_with_aliases;
use rustyline::error::ReadlineError;
use rustyline::history::{History, SearchDirection};
use rustyline::{Editor, history::FileHistory};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub type Repl = Editor<OsloHelper, FileHistory>;

pub fn run_repl() -> ! {
    // Everything downstream that behaves differently for a person than for a script — the job
    // notice, whether a background job keeps the terminal's stdin — reads this.
    // (Addressed by path rather than a re-export: `exec::mod` is being edited elsewhere.)
    oslo::exec::pipeline::set_interactive(true);
    // Semantic marks (OSC 133), so a terminal or multiplexer can see where each command's output
    // starts and stops. oslo only declares the boundaries; folding them is the job of whatever
    // owns the grid. See `oslo::interactive::marks`.
    oslo::interactive::marks::enable(true);
    // Asked once, before anything is drawn: the terminal's background decides whether the syntax
    // palette should be the dark one. A terminal that does not answer leaves the default standing
    // — see `oslo::interactive::query` for why the *silence* is the case worth engineering for.
    if let Some(background) = oslo::interactive::query::background() {
        oslo::interactive::theme::set_background(background);
    }

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
    // The lock is taken and released *before* the config runs. Holding it across `load_config`
    // is a deadlock in disguise: `borrow_env` uses `try_lock`, so every `oslo.*` call in the
    // config fails with "shell state is busy" and the whole file silently does nothing.
    let config = lua_init::config_path(&env_struct.lock().unwrap());
    if lua_init::install_bindings(&lua, Arc::clone(&env_struct))
        && let Some(path) = config
    {
        lua_init::load_config(&lua, &path);
        // The theme is read after the config has run, so `oslo.theme = {…}` in it takes effect
        // before the first prompt is drawn rather than after the first command.
        config::apply(&lua);
    }

    let settings = history::settings(&env_struct.lock().unwrap());
    // The database keeps the language each line was typed in, which a flat file cannot: recalling
    // a Lua line while the prompt is in shell mode has to run it as Lua. `$HISTFILE` still works
    // and still gets appended to, so nothing that reads it breaks.
    let db = history_db::database_path(
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
    .and_then(|path| history_db::History::open(&path));
    let mut rl = build_editor(&settings);

    // The mode the prompt is reading, and the flag the toggle key sets. Both live for the whole
    // session: switching language is a property of the session, not of one line.
    let mut current = mode::starting_mode(&env_struct.lock().unwrap());
    let toggle = ToggleRequest::new();
    keybind::apply(&mut rl, &env_struct, &toggle);

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
            Err(e) => eprintln!(
                "oslo: {}: {}",
                oslo::interactive::marks::path(&path.display().to_string()),
                e
            ),
        }
    }
    // Seeded from the database when there is one, so a session started on a machine with no
    // `$HISTFILE` still has its history back.
    if let Some(db) = &db {
        for entry in db.recent(settings.max_size.max(1)) {
            let _ = rl.add_history_entry(&entry.line);
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
                if let Some(db) = &db
                    && !secret
                {
                    db.append(
                        &text,
                        match mode {
                            Mode::Lua => history_db::MODE_LUA,
                            Mode::Shell => history_db::MODE_SHELL,
                        },
                    );
                    // `$HISTSIZE` bounds the table as well as the editor's copy, or the file
                    // grows without limit while the shell politely forgets.
                    db.trim(settings.max_size.max(1));
                }

                // Handed the command as typed, which is what a `precmd` hook is for: logging it,
                // timing it, or setting a title from it.
                lua.fire_hook("precmd", vec![LuaEngine::hook_arg(&text)]);
                // The title says what is running while it runs, and goes back to the directory
                // when the prompt returns. A row of tabs then says what each is *doing*.
                announce(&oslo::interactive::marks::title(&title_for_command(&text)));
                // Everything after this belongs to the command, not to the prompt.
                print!("{}", oslo::interactive::marks::output_start());
                let _ = std::io::Write::flush(&mut std::io::stdout());
                let before = current_directory();
                let started = std::time::Instant::now();

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
                    // The database is the history now, so `history -c` has to reach it too —
                    // clearing only the editor's copy would put every line back on the next start.
                    if let Some(db) = &db {
                        db.clear();
                    }
                    publish_history(&rl);
                }

                // Fired before the status is acted on, so a `cd` hook sees the directory the
                // command left behind even when that command was the last one of the session.
                let after = current_directory();
                if after != before {
                    lua.fire_hook("cd", vec![LuaEngine::hook_arg(&after)]);
                    // The terminal is told too, so a new split or tab opens here rather than in
                    // `$HOME`. One write, only when the directory actually changed.
                    announce(&oslo::interactive::marks::working_directory(&after));
                }
                let elapsed = started.elapsed();
                note_command_duration(elapsed);
                announce(&slow_command_notice(&text, elapsed, &res));
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
        // `oslo.vi.enabled`. Read here rather than toggled later because the keymap is fixed when
        // the editor is built.
        .edit_mode(if oslo::interactive::settings::current().vi.enabled {
            rustyline::EditMode::Vi
        } else {
            rustyline::EditMode::Emacs
        })
        // `List`, not `Circular`, and the reason is the dropdown.
        //
        // oslo's completer opens its own menu, waits for a choice, and returns that one candidate
        // already decided. Under `Circular` rustyline then starts a *second* selection loop over
        // that single candidate: it inserts it, waits for a key, and reads Tab as "next" — which
        // with one candidate wraps to the index past the end, whose meaning is *restore the
        // original line*. So accepting a completion and then pressing Tab silently deleted it.
        //
        // `List` applies a lone candidate and returns immediately, leaving Tab to start a fresh
        // completion, which is what the menu having already asked makes correct.
        .completion_type(rustyline::CompletionType::List)
        .build();
    Editor::with_config(config).expect("Failed to initialize line editor")
}

/// A notification for a command that ran long enough to be worth one, or nothing.
///
/// The threshold is `oslo.notify.after`, in seconds, and `0` turns it off. There is deliberately no
/// check for whether the terminal is focused: reporting focus needs a mode the shell would have to
/// enable and then read replies for, and getting that wrong costs stray characters at the prompt —
/// a worse failure than a notification you did not need.
fn slow_command_notice(
    text: &str,
    elapsed: std::time::Duration,
    result: &Result<i32, ShellError>,
) -> String {
    let after = oslo::interactive::settings::current().notify.after;
    if after == 0 || elapsed.as_secs() < after {
        return String::new();
    }
    let status = result.as_ref().copied().unwrap_or(1);
    let outcome = if status == 0 {
        "finished".to_string()
    } else {
        format!("failed ({status})")
    };
    let took = oslo::interactive::prompt::notable_duration(elapsed)
        .unwrap_or_else(|| format!("{}s", elapsed.as_secs()));
    // The first word, as the title bar gets: a notification is narrow too.
    let what = text.split_whitespace().next().unwrap_or("command");
    oslo::interactive::marks::notify(what, &format!("{outcome} after {took}"))
}

/// Write a terminal sequence, if there is one to write.
///
/// Empty when the sequence is disabled, which is every script and every `-c`, so this is the one
/// place that has to know the difference.
fn announce(sequence: &str) {
    if sequence.is_empty() {
        return;
    }
    print!("{sequence}");
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

/// The title to show while `text` runs: the command, shortened to its first word and trimmed.
///
/// The first word rather than the whole line, because a title bar is narrow and `cargo` in a tab
/// is more use than the first forty characters of a `for` loop.
fn title_for_command(text: &str) -> String {
    let first = text.split_whitespace().next().unwrap_or("");
    if first.is_empty() {
        current_directory()
    } else {
        format!(
            "{first} — {}",
            oslo::interactive::prompt::tilde(&current_directory())
        )
    }
}

/// Where the shell is now, for the `cd` hook to compare against.
pub(super) fn cwd() -> String {
    current_directory()
}

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
        eprintln!(
            "oslo: {}: {}",
            oslo::interactive::marks::path(&path.display().to_string()),
            e
        );
    }
}

thread_local! {
    /// How long the command before this prompt took, for the right prompt to mention.
    ///
    /// Thread-local rather than threaded through `read_command`: the duration is a property of the
    /// session's last command, and every caller that wants it is on the REPL's own thread.
    static LAST_DURATION: std::cell::Cell<Option<std::time::Duration>> =
        const { std::cell::Cell::new(None) };
}

/// How long the last command took, if one has run.
pub(super) fn last_command_duration() -> Option<std::time::Duration> {
    LAST_DURATION.get()
}

fn note_command_duration(elapsed: std::time::Duration) {
    LAST_DURATION.set(Some(elapsed));
}

pub(super) fn history_entries(rl: &Repl) -> Vec<String> {
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
    use super::Mode;
    use crate::startup::read::{HeredocTracker, is_complete};

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
