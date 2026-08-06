//! The interactive loop.
//!
//! It is kept out of `main` because it is where the three startup concerns meet: [`super::rc`]
//! decides what the prompt says and what was sourced before the first one, [`super::history`]
//! decides what is remembered, and [`super::lua_init`] adds the optional Lua layer on top.

use crate::absorb_loop_control;
use crate::startup::integration;
use crate::startup::mode::Mode;
use crate::startup::read::{Input, read_command};
use crate::startup::recall::{remember_history, seed_history};
use crate::startup::{config, environments, history, lua_init, mode, prompt, rc, tracking};
use oslo::Environment;
use oslo::LuaEngine;
use oslo::env::builtins::run_exit_trap;
use oslo::env::options::ShellOption;
use oslo::error::ShellError;
use oslo::exec::{JobManager, eval_command_list};
use oslo::interactive::OsloHelper;
use oslo::lua::api::hooks;
use oslo::parser::parse_with_aliases;
use std::sync::{Arc, Mutex};

// A sibling file rather than an entry in `startup::mod`, and a child module rather than a peer,
// because it is the loop's editor and nothing else's: `run_repl` is its only caller. Declared the
// way `main.rs` declares `history_expand`, for the same reason — the narrowest name a thing can
// have is the one that keeps it from being reached by accident.
#[path = "editor.rs"]
mod editor;
#[path = "notify.rs"]
mod notify;

use super::history::store::History;
use editor::{publish_history, remember};

pub fn run_repl(login: bool) -> ! {
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

    // The config runs before anything else reads a variable, so a `HISTSIZE=` or `PS1=` in it is
    // in force for this session rather than for the next one.
    if let Some(status) = rc::load_startup_files(&mut interactive_env, true, login) {
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
    let config = lua_init::config_files(&env_struct.lock().unwrap());
    if lua_init::install_bindings(&lua, Arc::clone(&env_struct)) && !config.is_empty() {
        for path in &config {
            lua_init::load_config(&lua, path);
        }
        // The settings are read after **every** config file has run, so a `conf.d` snippet and the
        // config proper are one decision rather than each one being applied and then overwritten.
        // Reading per file would also mean a snippet that set nothing reverted what an earlier one
        // set, since what is read is the whole `oslo` table each time.
        config::apply(&lua);
    }

    let settings = history::settings(&env_struct.lock().unwrap());
    // Start walking `$PATH` now, in the background. Whatever is left to do here — opening the
    // history database, building the editor, reading the config — is time the scan gets for free,
    // and it is the difference between the first Tab being instant and it being the one keystroke
    // that visibly stalls.
    if let Some(path) = env_struct.lock().unwrap().get_var("PATH") {
        oslo::interactive::command_index::warm(path.to_string());
    }

    // The command log keeps the language each line was typed in, which a flat file cannot:
    // recalling a Lua line while the prompt is in shell mode has to run it as Lua. `$HISTFILE`
    // still works and still gets appended to, so nothing that reads it breaks.
    //
    // It lives in the **same store** as the aggregate — one file, one open, one commit per
    // command. Opened from here because this is the one place in the program that knows a person
    // is typing: `tracking::Tracker::start` installs the process-wide handle, so a script, an
    // `oslo -c` or a subshell has none to write to.
    let here = current_directory();
    // The directory a session begins in is one the shell has been in, so `cd -1` names it and means
    // what `cd -` means from the first move rather than from the second. Here and nowhere else: the
    // ring is appended to by `change_directory`, which scripts also go through, and a script's ring
    // is nobody's wandering.
    oslo::env::builtins::remember_directory(&here);
    let mut tracker = tracking::Tracker::start(&here, &settings);
    // The directory environment for wherever the shell was started, which is a directory the user
    // walked into as much as any other — `cd` is not the only way to arrive somewhere.
    environments::start();
    environments::arrive(&env_struct, &lua, std::path::Path::new(&here));
    // The mode the prompt is reading. It lives for the whole session: switching language is a
    // property of the session, not of one line.
    let mut current = mode::starting_mode(&env_struct.lock().unwrap());

    let helper = OsloHelper::new(Arc::clone(&env_struct));
    let mut history = History::open(settings.file.clone(), settings.max_size);
    // Seeded from the database when there is one, so a session started on a machine with no
    // `$HISTFILE` still has its history back.
    if let Some(db) = oslo::track::store() {
        let entries = db.recent(settings.max_size.max(1));
        seed_history(entries.iter().map(|e| (e.line.clone(), e.mode.clone())));
    }
    publish_history(&history);

    let mut jobs = JobManager::new();
    jobs.setup_signals();

    // `oslo.misc.welcome = false` takes these two rows back. Printed here rather than earlier
    // because the config has run by now and can have turned them off — a banner that appeared
    // before the setting was read could not be suppressed by it.
    // A greeting of your own replaces the banner outright, fish's `fish_greeting`. It is a
    // separate setting from `welcome` rather than an empty-string special case, because "say
    // nothing" and "say this" are different intentions and one of them should not be spelled `""`.
    let misc = oslo::interactive::settings::current().misc;
    match &misc.greeting {
        Some(greeting) => println!("{greeting}"),
        None if misc.welcome => {
            println!(
                "oslo {} - POSIX Compatible Shell with Lua & Fish-style Features",
                env!("CARGO_PKG_VERSION")
            );
            println!("Type 'exit' or Ctrl-D to exit.");
        }
        None => {}
    }

    let mut last_status = 0;
    let mut eof_count = 0usize;

    // Universal variables, seeded once and then re-read whenever another shell writes the file.
    // `seen` is the set this loop put there, so a name the *user* assigns afterwards is not
    // quietly overwritten by the next reload — see `universal::apply`.
    let mut universal_stamp = oslo::env::universal::changed_at();
    oslo::env::universal::apply(&mut env_struct.lock().unwrap());

    loop {
        // Another shell may have set a universal variable since the last prompt. A `stat` decides
        // whether to read the file, so the common case — nobody changed anything — costs one
        // syscall per prompt rather than a parse.
        universal_stamp =
            oslo::env::universal::refresh(&mut env_struct.lock().unwrap(), universal_stamp);

        publish_terminal_size(&env_struct);

        // A prompt is about to be drawn. This is bash's `PROMPT_COMMAND` and zsh's `precmd`, and
        // the hook a prompt integration written in Lua needs — the shell-side one already exists
        // as `$PROMPT_COMMAND` below.
        lua.fire_at(hooks::at::PRE_PROMPT, Vec::new());

        // `$PROMPT_COMMAND` runs before every prompt. It is the other half of the DEBUG trap —
        // together they are bash's preexec/precmd pair, and every integration written for bash is
        // built on the two of them: starship redraws `PS1` here, hexe reports the command that
        // just finished. Fired before `read_command` rather than after a command, so it also runs
        // for the first prompt of the session, which is where a prompt integration draws itself.
        integration::prompt_command(&env_struct, last_status);

        match read_command(
            &helper,
            &history,
            &env_struct,
            &lua,
            last_status,
            &mut current,
        ) {
            Input::Nothing | Input::Interrupted => continue,
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
                remember(&mut history, &text, secret);
                if !secret {
                    // Kept alongside the editor's own copy so a later language switch, which
                    // refills that copy from scratch, still finds this line.
                    remember_history(&text, mode);
                }
                if let Some(db) = oslo::track::store()
                    && !secret
                {
                    db.append(
                        &text,
                        match mode {
                            Mode::Lua => oslo::track::log::MODE_LUA,
                            Mode::Shell => oslo::track::log::MODE_SHELL,
                        },
                    );
                    // `$HISTSIZE` bounds the log as well as the editor's copy, or the file grows
                    // without limit while the shell politely forgets. Amortised, because the trim
                    // is a full scan and this used to be one per line typed.
                    db.trim_soon(settings.max_size.max(1));
                }

                // Handed the command as typed, which is what a `precmd` hook is for: logging it,
                // timing it, or setting a title from it.
                // `preexec` is the accurate name; `precmd` is what oslo called it first and still
                // answers to. Both are handed the command line as typed.
                // Again here: this shell has been blocked in its line editor since before the
                // command was typed, so the check at the top of the loop last ran a command ago.
                // Without this, `universal X=1` in one terminal and `echo $X` in another shows the
                // old value once and the new one from then on — the worst possible behaviour,
                // since it looks like it works and does not.
                universal_stamp =
                    oslo::env::universal::refresh(&mut env_struct.lock().unwrap(), universal_stamp);
                // Read here rather than below because the hook is told it: a `preexec` handler
                // that logs where a command ran needs the directory it started in, and asking for
                // it from inside the handler would answer after any `cd` the last command did.
                let before = current_directory();
                let about = LuaEngine::command_started(&text, &before, mode.name());
                // **`pre-cmd` may answer.** A string replaces the line; `false` cancels it. Only
                // here, at a prompt — a script and `sh -c` never load a config, so no hook exists
                // to change what they run.
                let text = match oslo::lua::engine::answer_hook_with(
                    hooks::at::PRE_CMD,
                    vec![about.clone()],
                ) {
                    Some(oslo::lua::eval::value::Value::Bool(false)) => {
                        // Cancelled. 130 is the status a line abandoned at the prompt already
                        // reports, so nothing downstream needs a new case for this.
                        last_status = 130;
                        continue;
                    }
                    Some(oslo::lua::eval::value::Value::Str(replacement)) => {
                        replacement.to_string()
                    }
                    _ => text,
                };
                // The title says what is running while it runs, and goes back to the directory
                // when the prompt returns. A row of tabs then says what each is *doing*.
                announce(&oslo::interactive::marks::title(
                    &lua.render_with(
                        "prompt.title",
                        &prompt::title_context(last_status, current, &text),
                    )
                    .unwrap_or_else(|| title_for_command(&text)),
                ));
                // Everything after this belongs to the command, not to the prompt.
                print!("{}", oslo::interactive::marks::output_start());
                let _ = std::io::Write::flush(&mut std::io::stdout());
                let started = std::time::Instant::now();

                let res = match mode {
                    // A Lua line leaves `$?` where it was unless it asked otherwise: `oslo.proc.exit`
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
                    history.clear();
                    // Every copy, or the ones left behind go on answering. The database, because
                    // clearing only the editor's would put every line back on the next start; and
                    // oslo's own recall set, which is what the ghost suggestion, the Up/Down walk
                    // and history expansion all read — those kept offering the cleared commands.
                    // The log and the tracker's folded runs, which are now the same store — but
                    // not its directories: "forget my command lines" is not "forget where I work".
                    if let Some(track) = oslo::track::store() {
                        track.clear();
                        track.forget_runs();
                    }
                    oslo::interactive::recall::clear();
                    publish_history(&history);
                }

                // Fired before the status is acted on, so a `cd` hook sees the directory the
                // command left behind even when that command was the last one of the session.
                // A `direnv allow` or `deny` cannot reload itself: running `.env.lua` needs the
                // Lua engine, which lives here and not in a builtin's arguments. So the builtin
                // leaves a request and this carries it out — before the directory check below, so
                // that allowing and then `cd`-ing does not do the work twice.
                if oslo::direnv::take_reload_request() {
                    let here = current_directory();
                    environments::arrive(&env_struct, &lua, std::path::Path::new(&here));
                }

                let after = current_directory();
                if after != before {
                    // Before the `cd` hook, so a Lua hook that reads an environment variable sees
                    // the one this directory sets rather than the one the last directory did.
                    environments::arrive(&env_struct, &lua, std::path::Path::new(&after));
                    // The `post-change-dir` hook is **not** fired here. It fires from
                    // `attempt_directory`, which every `cd`, `pushd`, `popd` and jump passes
                    // through — so it also catches a move made inside a function or a subshell,
                    // which comparing the directory across a whole command line cannot see. Firing
                    // in both places would fire it twice for every ordinary `cd`.
                    // The terminal is told too, so a new split or tab opens here rather than in
                    // `$HOME`. One write, only when the directory actually changed.
                    announce(&oslo::interactive::marks::working_directory(&after));
                }
                let elapsed = started.elapsed();
                note_command_duration(elapsed);
                announce(&notify::slow_command_notice(&text, elapsed, &res));
                // The command is over and its status is known: close the block before anything
                // else prints, so nothing that follows lands inside it.
                print!(
                    "{}",
                    oslo::interactive::marks::command_end(res.as_ref().copied().unwrap_or(1))
                );
                let _ = std::io::Write::flush(&mut std::io::stdout());
                // **Fired whether the command succeeded or not.** It used to run only on `Ok`, so
                // the hook was silent for exactly the commands a hook is most often installed to
                // notice — a parse error, an `exit`, anything that did not return a status. A
                // `postexec` that skips failures cannot be used to report them.
                //
                // `after` rather than `before`, so a `cd` is reflected in the directory the hook is
                // told about; and the same `elapsed` the notice and the history column use.
                let done = LuaEngine::command_finished(
                    &text,
                    &after,
                    mode.name(),
                    res.as_ref().copied().unwrap_or(1),
                    elapsed,
                );
                lua.fire_at(hooks::at::POST_CMD, vec![done]);
                // Beside the hook rather than through it: a command that failed is exactly the one
                // the `fails` column exists to count. Every argument here is a local this loop
                // already had and used to drop.
                if secret {
                    tracker.forget_boundary();
                } else {
                    let run = tracking::ran(&text, mode.name(), &res, elapsed);
                    tracker.boundary(&before, &after, run);
                }

                match res {
                    Ok(status) => last_status = status,
                    Err(ShellError::Exit(code)) => {
                        // The amortised trim lets the table run over between sweeps, so the bound
                        // is enforced on the way out or a short session never enforces it at all.
                        settle_stores(&settings);
                        fire_exit(&lua, code);
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

    settle_stores(&settings);
    // End of input (Ctrl-D) is the other way a REPL ends, and POSIX makes no distinction: the
    // EXIT trap fires on both.
    fire_exit(&lua, last_status);
    let mut env_guard = env_struct.lock().unwrap();
    let last_status = run_exit_trap(&mut env_guard, last_status);
    drop(env_guard);
    std::process::exit(last_status);
}

/// Put the terminal's size in `$COLUMNS` and `$LINES`, **exported**, before every prompt.
///
/// # Why a shell has to do this
///
/// A program the shell runs cannot ask how wide the terminal is unless it *has* the terminal —
/// and a prompt renderer is usually reading a pipe, because the shell is capturing what it says.
/// `TIOCGWINSZ` on a pipe fails, so every such tool falls back to `$COLUMNS`, and to 80 when that
/// is missing too. bash maintains both under `checkwinsize`; zsh keeps them as special variables.
///
/// oslo did neither, and the effect was not subtle: `hexe shp prompt` laid every prompt out as
/// though the terminal were 80 columns wide however wide it really was, so segments were dropped
/// as "not fitting" on a 230-column screen. The same is true of starship, of `$(tput cols)` inside
/// a substitution, and of anything else asked to render into a pipe.
///
/// Refreshed per prompt rather than from a `SIGWINCH` handler, which is what `checkwinsize` does
/// and is enough: nothing between two prompts can observe the value except a command, and a
/// command that ran before the resize could not have seen it anyway. It also keeps the signal
/// handler free of anything that allocates.
///
/// **Exported**, unlike bash's, because the whole point is that a *child* reads it — a shell
/// variable a child cannot see would fix nothing here.
fn publish_terminal_size(env: &Arc<Mutex<Environment>>) {
    let cols = oslo::interactive::dropdown::terminal_cols();
    let rows = oslo::interactive::dropdown::width::terminal_rows();
    let mut guard = env.lock().unwrap();
    guard.set_var("COLUMNS", &cols.to_string(), true);
    guard.set_var("LINES", &rows.to_string(), true);
}

/// `on-exit`, from both ways a REPL ends — `exit` and end of input.
///
/// **Before the EXIT trap, not after.** The trap may itself call `exit`, and a hook that ran after
/// it would be skipped exactly when the session ended in the way most worth reporting. Both call
/// sites are needed because POSIX makes no distinction between the two endings and neither does
/// anything else here.
fn fire_exit(lua: &LuaEngine, status: i32) {
    lua.fire_at(
        hooks::at::ON_EXIT,
        vec![LuaEngine::hook_fields(&[(
            "status",
            oslo::lua::eval::value::Value::int(status as i64),
        )])],
    );
}

/// Leave the history in the state a shell that is not running should leave it.
///
/// The trim puts the history table back inside `$HISTSIZE`. It has to happen here as well as on the
/// amortised counter, or a session shorter than the counter's period never enforces the bound at
/// all. Best effort, and it does not block: another terminal holding the file means it does not
/// happen this time.
///
/// Neither store has a checkpoint any more, because neither has a log. Both are one file that is
/// consistent at every commit, and the tracker's own bound is the daily sweep in `track::prune`
/// rather than anything the way out of the loop can do. See `history_db`'s note and that module's.
fn settle_stores(settings: &history::Settings) {
    if let Some(db) = oslo::track::store() {
        db.trim(settings.max_size.max(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

#[path = "repl/aside.rs"]
mod aside;
use aside::{announce, current_directory, note_command_duration, run_lua_line, title_for_command};
// Read from `startup::prompt` and `startup::read`, which asked `repl` for them before the split
// and should not have to learn where they moved to.
pub(crate) use aside::{cwd, ignore_eof_limit, last_command_duration};
