//! The interactive loop.
//!
//! It is kept out of `main` because it is where the three startup concerns meet: [`super::rc`]
//! decides what the prompt says and what was sourced before the first one, [`super::history`]
//! decides what is remembered, and [`super::lua_init`] adds the optional Lua layer on top.

use crate::absorb_loop_control;
use crate::lua::LuaEngine;
use crate::lua::api::hooks;
use crate::startup::integration;
use crate::startup::mode::Mode;
use crate::startup::read::{Input, read_command};
use crate::startup::recall::{remember_history, seed_from_store};
use crate::startup::{arrival, config, history, lua_init, mode, plugins, prompt};
use crate::startup::{rc, timers, timing, tracking};
use oslo_base::error::ShellError;
use oslo_shell::Environment;
use oslo_shell::env::builtins::run_exit_trap;
use oslo_shell::env::options::ShellOption;
use oslo_shell::exec::{JobManager, eval_command_list};
use oslo_shell::syntax::parse_with_aliases;
use oslo_ui::OsloHelper;
use std::sync::{Arc, Mutex};

// A sibling file rather than an entry in `startup::mod`, and a child module rather than a peer,
// because it is the loop's editor and nothing else's: `run_repl` is its only caller. Declared the
// way `main.rs` declares `history_expand`, for the same reason — the narrowest name a thing can
// have is the one that keeps it from being reached by accident.
#[path = "editor.rs"]
mod editor;
#[path = "notify.rs"]
mod notify;
// The completion provider for a script that declares its own arguments.
#[cfg(feature = "argc")]
#[path = "repl/argc.rs"]
mod argc;
mod precmd;

use super::history::store::History;
use editor::{publish_history, remember};

pub fn run_repl(login: bool) -> ! {
    // Everything downstream that behaves differently for a person than for a script — the job
    // notice, whether a background job keeps the terminal's stdin — reads this.
    // (Addressed by path rather than a re-export: `exec::mod` is being edited elsewhere.)
    oslo_shell::exec::pipeline::set_interactive(true);
    super::terminal::initialize();

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
    // **A shell inside a shell says so before it becomes one.** After the config, so a setting can
    // turn the question off; before everything below, so answering "no" pays for none of it.
    super::nested::ask_before_nesting();
    // **After the config, so the database wins.** The ordinary shell rule — the last definition of a
    // name is the one that counts — applied to sources rather than to lines. See `startup::stored`.
    let mut macros_held = super::stored::install(&env_struct);
    plugins::start(&env_struct);
    // A script that declares its arguments completes them. Registered after the config so
    // `oslo.completion.sources` can name it and a provider of the same name can replace it.
    #[cfg(feature = "argc")]
    argc::register();

    let settings = history::settings(&env_struct.lock().unwrap());
    // Start walking `$PATH` now, in the background. Whatever is left to do here — opening the
    // history database, building the editor, reading the config — is time the scan gets for free,
    // and it is the difference between the first Tab being instant and it being the one keystroke
    // that visibly stalls.
    if let Some(path) = env_struct.lock().unwrap().get_var("PATH") {
        oslo_ui::command_index::warm(path.to_string());
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
    oslo_shell::env::builtins::remember_directory(&here);
    let mut tracker = tracking::Tracker::start(&here, &settings);
    // The directory environment for wherever the shell was started, which is a directory the user
    // walked into as much as any other — `cd` is not the only way to arrive somewhere.
    arrival::start();
    arrival::arrive(&env_struct, &lua, std::path::Path::new(&here));
    // The mode the prompt is reading. It lives for the whole session: switching language is a
    // property of the session, not of one line.
    let mut current = mode::starting_mode(&env_struct.lock().unwrap());

    let helper = OsloHelper::new(Arc::clone(&env_struct));
    let mut history = History::open(settings.file.clone(), settings.max_size);
    seed_from_store(&mut history, settings.max_size);
    publish_history(&history);

    let mut jobs = JobManager::new();
    jobs.setup_signals();

    // `oslo.misc.welcome = false` takes these two rows back. Printed here rather than earlier
    // because the config has run by now and can have turned them off — a banner that appeared
    // before the setting was read could not be suppressed by it.
    // A greeting of your own replaces the banner outright, fish's `fish_greeting`. It is a
    // separate setting from `welcome` rather than an empty-string special case, because "say
    // nothing" and "say this" are different intentions and one of them should not be spelled `""`.
    let misc = oslo_ui::settings::current().misc.clone();
    match &misc.greeting {
        Some(greeting) => println!("{greeting}"),
        None if misc.welcome => {
            println!(
                "oslo {} - POSIX Compatible Shell with Lua & Fish-style Features",
                oslo_base::version::current()
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
    let mut universal_stamp = oslo_shell::env::universal::changed_at();
    oslo_shell::env::universal::apply(&mut env_struct.lock().unwrap());

    // **What an idle prompt does when the background moves.** Installed here rather than in
    // `terminal::initialize` because it needs the environment a universal refresh writes into, and
    // the REPL is what owns it. Runs on the shell thread with no editor borrow held — see
    // `oslo_base::background`.
    let serviced = Arc::clone(&env_struct);
    oslo_base::background::install(move || {
        oslo_shell::exec::job::reap_background_jobs();
        let changed = match serviced.lock() {
            Ok(mut env) => oslo_shell::env::universal::apply_if_changed(&mut env),
            Err(_) => false,
        };
        // **A changed variable is invisible until the prompt is rebuilt.** `PS1` is expanded when
        // the prompt is rendered, not on every repaint — so a theme another terminal just set would
        // otherwise sit in the environment, correct and unseen, until the next command. This is the
        // same door an asynchronous prompt already comes through.
        if changed {
            oslo_ui::prompt::invalidate();
        }
        changed
    });
    // Armed *after* the servicer, and only here: both checks want an interactive shell that has
    // somewhere to deliver the news. A script reaps at its command boundaries and has no editor to
    // wake, so the signal would buy it nothing and cost it an interrupted `read` in every library
    // call that makes one.
    oslo_ui::term::watch_for_children();
    oslo_shell::env::universal::watch::start();

    // The directory whose `.env.lua` is loaded. Seeded from the arrival above, so the first prompt
    // does not run it a second time.
    let mut settled = here.clone();

    loop {
        timers::fire();
        // **Where the shell notices it has moved, by any route at all.**
        //
        // The directory environment used to be reconciled in one place only: after a command line
        // whose start and end directories differed. That misses every other way of moving —
        // a key binding that jumps, a Lua hook, a `cd` in a sourced file, anything that does not
        // straddle a whole command — and what is left behind is not nothing. It is the previous
        // project's `$PATH`, its exported variables and its aliases, in a shell standing somewhere
        // else. An alias like `_b` then still builds the repository you walked out of, and the
        // only cure was a `cd` round trip to force the check to run.
        //
        // Comparing against the directory last *settled* rather than against where the last
        // command happened to start makes the route irrelevant: however the shell got here, the
        // prompt before you type is the moment it has to be true. `arrive` itself is cheap when
        // nothing has changed, and this only calls it when the directory actually differs.
        // What the *previous* cycle cost, printed where the lag was felt: after the key that ended
        // the last line and before this prompt is drawn. Reporting after the read instead would
        // land the line only once the next key had already been pressed.
        timing::report();

        timing::phase("direnv", || {
            let here = current_directory();
            if here != settled {
                arrival::arrive(&env_struct, &lua, std::path::Path::new(&here));
                settled = here;
            }
        });

        // Another shell may have set a universal variable since the last prompt. A `stat` decides
        // whether to read the file, so the common case — nobody changed anything — costs one
        // syscall per prompt rather than a parse.
        universal_stamp = timing::phase("universal", || {
            oslo_shell::env::universal::refresh(&mut env_struct.lock().unwrap(), universal_stamp)
        });

        // And the same question about macros: another shell — or `oslo macros` in this one — may
        // have added, removed or turned one off since the last prompt. Two `stat`s decide whether
        // there is anything to do, which is the same bargain the line above makes.
        macros_held = timing::phase("macros", || {
            super::stored::refresh(&env_struct, macros_held)
        });

        timing::phase("size", || publish_terminal_size(&env_struct));

        // A prompt is about to be drawn. This is bash's `PROMPT_COMMAND` and zsh's `precmd`, and
        // the hook a prompt integration written in Lua needs — the shell-side one already exists
        // as `$PROMPT_COMMAND` below.
        timing::phase("pre-prompt", || {
            lua.fire_at(hooks::at::PRE_PROMPT, Vec::new())
        });

        // `$PROMPT_COMMAND` runs before every prompt. It is the other half of the DEBUG trap —
        // together they are bash's preexec/precmd pair, and every integration written for bash is
        // built on the two of them: starship redraws `PS1` here, hexe reports the command that
        // just finished. Fired before `read_command` rather than after a command, so it also runs
        // for the first prompt of the session, which is where a prompt integration draws itself.
        timing::phase("prompt-command", || {
            integration::prompt_command(&env_struct, last_status)
        });

        let mut interaction = oslo_ui::marks::Interaction::begin();
        match read_command(
            &helper,
            &history,
            &env_struct,
            &lua,
            last_status,
            &mut current,
        ) {
            Input::Nothing | Input::Interrupted => {
                print!("{}", interaction.abort());
                let _ = std::io::Write::flush(&mut std::io::stdout());
                continue;
            }
            Input::Eof => {
                print!("{}", interaction.abort());
                let _ = std::io::Write::flush(&mut std::io::stdout());
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

                // Handed the command as typed, which is what a `precmd` hook is for: logging it,
                // timing it, or setting a title from it.
                // `preexec` is the accurate name; `precmd` is what oslo called it first and still
                // answers to. Both are handed the command line as typed.
                // Again here: this shell has been blocked in its line editor since before the
                // command was typed, so the check at the top of the loop last ran a command ago.
                // Without this, `universal X=1` in one terminal and `echo $X` in another shows the
                // old value once and the new one from then on — the worst possible behaviour,
                // since it looks like it works and does not.
                universal_stamp = oslo_shell::env::universal::refresh(
                    &mut env_struct.lock().unwrap(),
                    universal_stamp,
                );
                // Read here rather than below because the hook is told it: a `preexec` handler
                // that logs where a command ran needs the directory it started in, and asking for
                // it from inside the handler would answer after any `cd` the last command did.
                let before = current_directory();
                let about = LuaEngine::command_started(&text, &before, mode.name());
                // The line as typed, kept because history records what you wrote rather than what a
                // hook rewrote it into — and the rewrite happens two lines below.
                let entered = text.clone();
                // **`pre-cmd` may answer.** A string replaces the line; `false` cancels it; a table
                // does either and may also decline to have the line written down. Only here, at a
                // prompt — a script and `sh -c` never load a config, so no hook exists to change
                // what they run.
                let answer =
                    crate::lua::engine::answer_hook_with(hooks::at::PRE_CMD, vec![about.clone()]);
                let Some(answered) = precmd::read(answer, text) else {
                    // Cancelled. 130 is the status a line abandoned at the prompt already
                    // reports, so nothing downstream needs a new case for this.
                    last_status = 130;
                    print!("{}", interaction.abort());
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                    continue;
                };
                let text = answered.text;
                // **The veto joins the leading space rather than running beside it.** Every sink
                // below already asks this one flag, so a hook declining to be recorded is the same
                // condition by a second route — and cannot reach a sink the space does not, or miss
                // one it does. See `precmd::write_down` for why the recording follows the hook.
                let secret = secret || !answered.record;
                let logged_as =
                    precmd::write_down(&mut history, &entered, mode, secret, settings.max_size);
                plugins::before(&text);
                // The title says what is running while it runs, and goes back to the directory when
                // the prompt returns. **A hidden line does not reach it either**: the title goes to
                // the terminal and the multiplexer, the same audience as the mark below.
                announce(&oslo_ui::marks::title(&if secret {
                    "private command".to_string()
                } else {
                    lua.render_with(
                        "prompt.title",
                        &prompt::title_context(last_status, current, &text),
                    )
                    .unwrap_or_else(|| title_for_command(&text))
                }));
                // Everything after this belongs to the command, not to the prompt.
                print!(
                    "{}",
                    oslo_ui::marks::output_start((!secret).then_some(text.as_str()))
                );
                let _ = std::io::Write::flush(&mut std::io::stdout());
                let started = std::time::Instant::now();

                // Held for exactly as long as the command runs. `set -x` is the one place a hidden
                // line could still print its own arguments, and it prints them from inside
                // execution rather than from here — so the decision has to travel with the command.
                let quiet = secret.then(oslo_base::quiet::Quiet::enter);

                let res = match mode {
                    // A Lua line leaves `$?` where it was unless it asked otherwise: `oslo.proc.exit`
                    // is the way to choose a status, and a chunk that merely printed something
                    // has not run a command.
                    Mode::Lua => run_lua_line(&lua, &text, last_status),
                    Mode::Shell => {
                        // Record what each link of `a && b || c` does, for the line the user typed
                        // and nothing else. Armed here rather than inside the evaluator because
                        // *this* is the only place that knows a person typed it: a script, a
                        // `-c` command and every nested chain leave it off and pay nothing.
                        oslo_shell::exec::pipeline::segments::arm();
                        let mut env_guard = env_struct.lock().unwrap();
                        let res = absorb_loop_control(
                            parse_with_aliases(&text, !env_guard.get_aliases().is_empty(), &|n| {
                                env_guard.get_alias(n).map(str::to_string)
                            })
                            .and_then(|ast| eval_command_list(&mut env_guard, &ast)),
                        );
                        drop(env_guard);
                        oslo_shell::exec::pipeline::segments::disarm();
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
                    if let Some(track) = oslo_base::track::store() {
                        track.clear();
                        track.forget_runs();
                    }
                    oslo_ui::recall::clear();
                    publish_history(&history);
                }

                // Fired before the status is acted on, so a `cd` hook sees the directory the
                // command left behind even when that command was the last one of the session.
                // A `direnv allow` or `deny` cannot reload itself: running `.env.lua` needs the
                // Lua engine, which lives here and not in a builtin's arguments. So the builtin
                // leaves a request and this carries it out — before the directory check below, so
                // that allowing and then `cd`-ing does not do the work twice.
                if arrival::reload_requested() {
                    let here = current_directory();
                    arrival::arrive(&env_struct, &lua, std::path::Path::new(&here));
                }

                // **Where a hook that could only watch becomes one that can act.** `post-change-dir`
                // and the rest of the notifying hooks fire from places that hold the shell's state
                // — `attempt_directory`, the job reaper, the timing report — so they are held until
                // here, which is the first moment in a command's life when nothing is locked. The
                // fire sites stay where they are accurate; only the handler moves.
                //
                // Beside `take_reload_request` above, and for the same reason it exists: a builtin
                // leaves something behind and this carries it out.
                timers::after_command();
                // The command is over, so `set -x` belongs to whatever runs next.
                drop(quiet);

                // Where the command left the shell, for the tracker below.
                //
                // Nothing is reconciled here any more: the directory environment and the
                // terminal's idea of where we are are both brought into line at the top of the
                // loop, before the next prompt, along with every other way of moving. Doing it in
                // two places meant they disagreed the moment one learned about a route the other
                // did not, which is exactly how this broke.
                //
                // The `post-change-dir` hook is separate and stays where it is: it fires from
                // `attempt_directory`, which every `cd`, `pushd`, `popd` and jump passes through,
                // so it catches a move made inside a function or a subshell too.
                let after = current_directory();
                let elapsed = started.elapsed();
                note_command_duration(elapsed);
                let terminal_status = match &res {
                    Ok(status) | Err(ShellError::Exit(status)) => *status,
                    Err(error) => error.failure_status(),
                };
                // Close command output before post-command reports and notifications.
                print!("{}", interaction.finish(terminal_status));
                let _ = std::io::Write::flush(&mut std::io::stdout());
                // A chain that stopped part-way says where, and what would carry on from there.
                // One line, and only when there is something to say: a chain that finished, or a
                // single command that failed, prints nothing.
                if let Some(from) = oslo_shell::exec::pipeline::segments::resumable() {
                    eprintln!("oslo: chain stopped — resume with: {from}");
                }
                // **A hidden line does not get a desktop notification either**, which would put the
                // command in a popup and, on most desktops, in a notification history the shell
                // does not own and cannot clear.
                if !secret {
                    announce(&notify::slow_command_notice(&text, elapsed, &res));
                }
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
                    // A `pre-record` rule may keep one link of a chain instead of the whole line,
                    // or refuse it. Asked once, before anything is written down, and the answer
                    // decides both what the aggregate learns and what the log ends up saying.
                    let decided =
                        tracking::ask_what_to_record(&text, &before, mode.name(), &res, elapsed);
                    let lines = tracking::lines_to_record(&decided, &text);
                    for (first, line) in lines.iter().enumerate().map(|(i, l)| (i == 0, l)) {
                        let run = tracking::ran(line, mode.name(), &res, elapsed);
                        // Only the first arrival records the *movement* and the dwell; a second
                        // one would credit this directory twice for the same command.
                        if first {
                            tracker.boundary(&before, &after, run);
                        } else {
                            tracker.also_ran(&before, run);
                        }
                    }
                    if lines.is_empty() {
                        tracker.forget_boundary();
                    }
                    // What the line did, joined to the log row it went in under. **After the
                    // boundary**, because that is what resolves the directory this then reads back
                    // — one lookup between them rather than two.
                    if let Some(id) = logged_as {
                        tracking::settle_log_row(id, &decided, &text);
                        if !lines.is_empty() {
                            tracking::record_outcome(id, &res, elapsed);
                        }
                    }
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

#[cfg(test)]
#[path = "repl/tests.rs"]
mod tests;

#[path = "repl/aside.rs"]
mod aside;
mod session;
use aside::{announce, current_directory, note_command_duration, run_lua_line, title_for_command};
use session::{fire_exit, publish_terminal_size, settle_stores};
// Read from `startup::prompt` and `startup::read`, which asked `repl` for them before the split
// and should not have to learn where they moved to.
pub(crate) use aside::{cwd, ignore_eof_limit, last_command_duration};
