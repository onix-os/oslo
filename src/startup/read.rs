//! Reading one command from the line editor.
//!
//! Split from [`super::repl`] because reading and running are separate concerns and only one of
//! them is hard. Running a command is a call; *reading* one means a continuation prompt when the
//! line is unfinished, a here-document body that must go in exactly as typed, two languages to
//! choose between, and a key that hands control back by accepting the line.

use crate::expand_history;
use crate::startup::mode::{self, Line, Mode, ToggleRequest};
use crate::startup::repl::Repl;
use crate::startup::{history, prompt, rc};
use oslo::Environment;
use oslo::LuaEngine;
use oslo::interactive::InputStatus;
use rustyline::error::ReadlineError;
use std::sync::{Arc, Mutex};

/// One trip round the prompt.
pub(super) enum Input {
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

/// Read one complete command, continuing onto further lines while the parser wants more.
///
/// This is where `PS2` earns its keep: a command that is not finished gets the continuation
/// prompt, instead of the hard syntax error `for i in 1 2 3` used to produce the moment you
/// pressed Enter.
pub(super) fn read_command(
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
    // Where the cursor goes when `typed` is put back. Only a `bind -x` command moves it away from
    // the end — `$READLINE_POINT` is how a plugin says "leave the cursor here", and a picker that
    // inserts a word mid-line needs it.
    let mut typed_point = 0usize;

    loop {
        // Every line starts in insert mode as far as the editor is concerned, so the mode this
        // module remembers has to start there too. Before the prompt is *rendered*, not after:
        // leaving a line in normal mode otherwise drew the next prompt saying `N` while the editor
        // was already back in insert, and it stayed wrong until the first keystroke.
        oslo::interactive::vi::reset();
        // A new line is a new walk through the history: whatever the last one had scrolled back
        // to belongs to the line that is now finished.
        super::keybind::reset_history_walk();
        // The shape too: the terminal is still drawing whatever the last line ended in, and a
        // block cursor over a line you are typing into says normal mode when it is insert.
        // Only to a terminal: a cursor-shape escape written down a pipe is not a cursor shape,
        // it is two stray bytes in somebody's output.
        let settings = oslo::interactive::settings::current();
        if settings.vi.enabled && std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            print!("{}", settings.vi.cursors.insert.escape());
        }

        let prompt = if buffer.is_empty() {
            prompt::primary_prompt(env_struct, lua, last_status, *current)
        } else {
            lua.render_with("prompt.continuation", &{
                let mut ctx = prompt::segment_context(last_status, *current, None);
                // The one fact that is different here, and the reason the field exists.
                ctx.continuation = true;
                ctx
            })
            .unwrap_or_else(|| rc::ps2(&mut env_struct.lock().unwrap()))
        };

        // The right prompt is handed to the helper rather than concatenated: it is drawn from
        // the highlighter, which is the only place a cursor move does not confuse rustyline.
        if let Some(helper) = rl.helper() {
            // A config's own right prompt wins; otherwise oslo draws one. There used to be none at
            // all unless a config asked, which meant the machinery existed and nobody saw it.
            // The same order the left prompt resolves in: a Lua prompt is an explicit choice and
            // outranks the shell variable, which outranks oslo's own.
            //
            // `$RPS1` is here so that a *shell* integration can own both sides. hexe and starship
            // rebuild the prompt from `PROMPT_COMMAND`, and until now the only thing they could
            // set was `PS1` — the right column was oslo's whatever they did, so half the prompt
            // came from the integration and half from the shell arguing with it.
            // **The same facts the left prompt gets.** These two used to be rendered with
            // `render`, which is `render_with(Context::default())` — so a right-prompt segment saw
            // status 0, an empty cwd, no branch and no duration, whatever had actually happened.
            // A right prompt exists to show the status and the duration; handing it zeros made the
            // whole feature draw the wrong thing rather than nothing, which is worse.
            let facts = prompt::segment_context(last_status, reading, None);
            let right = lua
                .render_with("prompt.right", &facts)
                .or_else(|| rc::rps1(&mut env_struct.lock().unwrap()))
                .or_else(|| {
                    Some(oslo::interactive::prompt::render_default_right_prompt(
                        last_status,
                        super::repl::last_command_duration(),
                    ))
                });
            helper.set_right_prompt(right, oslo::interactive::prompt::printed_width(&prompt));
            // What this prompt is for, so a vi-mode repaint rebuilds the same one rather than
            // guessing at the language or the status.
            helper.set_prompt_context(reading.name(), last_status);
        }

        // What this row *is*, recorded before the editor is entered rather than from inside the
        // highlighter. The highlighter only reaches its own `note_row` on some paths, so anything
        // that redraws the prompt — the vi mode letter, the language toggle — found nothing
        // recorded and silently did nothing at all. Here it is unconditional: a prompt is about to
        // be drawn, and this is what it says.
        {
            // Whether what is about to be drawn is oslo's own prompt. A `$PS1`, a Lua prompt or
            // the continuation prompt is not, and must not be redrawn as one.
            let builtin = prompt
                == oslo::interactive::prompt::render_default_left_prompt(
                    last_status,
                    reading.name(),
                );
            oslo::interactive::prompt::note_row(
                reading.name(),
                last_status,
                oslo::interactive::prompt::printed_width(&prompt),
                builtin,
            );
            // What this prompt looks like in each vi mode, so a mode change mid-line can redraw
            // it. Only the mode changes while a line is being typed; everything else a prompt
            // shows is fixed for the line, so a handful of variants covers it.
            //
            // Only when the width does not move: the editor measures the prompt once and lays the
            // row out against that number for the life of the line, so a variant of a different
            // size would put the text a cell away from where the editor believes it is.
            if !builtin && oslo::interactive::vi::enabled() {
                let width = oslo::interactive::prompt::printed_width(&prompt);
                let variants = ["I", "N", "R"]
                    .into_iter()
                    .filter_map(|mode| {
                        let ctx = prompt::segment_context(last_status, reading, Some(mode));
                        let text = lua.render_with("prompt.left", &ctx)?;
                        (oslo::interactive::prompt::printed_width(&text) == width)
                            .then(|| (mode.to_string(), text))
                    })
                    .collect();
                oslo::interactive::row::set_variants(variants);
            }
        }

        // Written before the prompt rather than inside it, for the same reason the right prompt is
        // not concatenated: the line editor measures the prompt string to know where the line
        // starts, and an OSC in there is counted as visible width.
        // The title goes back to the directory now that nothing is running, and the working
        // directory is (re)announced — the first prompt of a session is the only chance to tell
        // the terminal where it started.
        print!(
            "{}{}{}",
            oslo::interactive::marks::working_directory(&crate::startup::repl::cwd()),
            oslo::interactive::marks::title(&oslo::interactive::prompt::tilde(
                &crate::startup::repl::cwd()
            )),
            oslo::interactive::marks::prompt_start()
        );
        let _ = std::io::Write::flush(&mut std::io::stdout());

        let split = typed_point.min(typed.len());
        let raw = match rl.readline_with_initial(&prompt, (&typed[..split], &typed[split..])) {
            Ok(raw) => raw,
            Err(ReadlineError::Interrupted) => {
                // A finder choice deliberately interrupts this editor instance instead of using
                // rustyline's `Replace`: replacement text is inserted at the right place but its
                // logical cursor is not advanced. Reopening with the whole choice on the *left*
                // side of `readline_with_initial` puts both the visible and logical cursor at the
                // end, so the next typed character really appends there.
                if let Some(choice) = super::keybind::take_finder_choice() {
                    typed = choice.line;
                    typed_point = typed.len();
                    print!(
                        "{}",
                        oslo::interactive::row::rewind_after_readline(&choice.previous)
                    );
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                    continue;
                }
                // Ctrl-C abandons the *line*, not the language. If the user switched to Lua and
                // then thought better of the command, the prompt they were looking at said `lua`
                // and the next one must too — and the flag has to be consumed either way, or it
                // fires on the next line and switches the language nobody asked to switch.
                if toggle.take()
                    && let Some(language) = oslo::interactive::prompt::language()
                {
                    *current = if language == "lua" {
                        Mode::Lua
                    } else {
                        Mode::Shell
                    };
                }
                // No `^C` echoed. The keystroke was the user's instruction to abandon the line,
                // and they know they pressed it — printing it back spends a row saying so, which
                // is a row the next prompt could have had. The editor has already moved off the
                // abandoned line, so there is nothing to write here at all.
                return Input::Interrupted;
            }
            Err(ReadlineError::Eof) => {
                // Same for end of input: leaving the flag set would apply the switch to whatever
                // line came next if `$IGNOREEOF` keeps the shell alive.
                let _ = toggle.take();
                return Input::Eof;
            }
            Err(err) => {
                eprintln!("oslo: {}", err);
                return Input::Fatal;
            }
        };
        typed.clear();
        typed_point = 0;

        // The line has been accepted, so the prompt above it has done its job. If a config asked
        // for a shorter one to stand in its place, put that there now — before the command runs,
        // so what scrolls past is one line of prompt per command rather than three.
        //
        // Only to a terminal, and only from the row the editor actually drew: `rewind_after_readline`
        // accounts for the wrap, which is why this is not `ESC [ 1 A`.
        if std::io::IsTerminal::is_terminal(&std::io::stdout())
            && let Some(short) = lua.render_with(
                "prompt.transient",
                &prompt::segment_context(last_status, reading, None),
            )
        {
            print!(
                "{}{}{}\r\n",
                oslo::interactive::row::rewind_after_readline(&raw),
                short,
                raw
            );
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }

        // The toggle key repaints the prompt in place rather than submitting, so by the time a
        // line comes back the prompt may be showing the other language. That is the answer for
        // *this* line and for the ones after it: what you see above the cursor is what runs.
        // The toggle no longer submits anything — it repaints the prompt where it stands — so
        // this is only catching up with a switch that already happened on screen. It must not put
        // the line back into the editor: by the time this runs the user has pressed Enter, and
        // re-entering here would swallow the command instead of running it.
        if toggle.take()
            && let Some(language) = oslo::interactive::prompt::language()
        {
            let switched = if language == "lua" {
                Mode::Lua
            } else {
                Mode::Shell
            };
            *current = switched;
            reading = switched;
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
            // **Only this language's lines.** The editor's history holds both, and `!!` expanding
            // to a Lua line at a shell prompt produces something that cannot run — the same
            // crossing the ghost suggestion and the arrow keys were fixed for.
            let previous: Vec<String> = oslo::interactive::recall::for_language(reading.name());
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
pub(super) struct HeredocTracker(bool);

impl HeredocTracker {
    /// Whether history expansion may rewrite the next line.
    pub(super) fn expands_history(&self) -> bool {
        !self.0
    }

    /// Take account of a line that has been accepted into the buffer.
    ///
    /// Only ever turns the bit on. A document whose delimiter arrives part-way through an
    /// unfinished command leaves the rest of that command unexpanded too — the safe direction:
    /// the cost is a `!!` that has to be typed out in full, against a `!` inside a heredoc
    /// quietly becoming somebody else's command.
    pub(super) fn observe(&mut self, line: &str) {
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
pub(super) fn is_complete(source: &str, mode: Mode) -> bool {
    match mode {
        Mode::Lua => oslo::lua::eval::is_complete(source),
        Mode::Shell => !matches!(
            oslo::interactive::syntax::classify(source),
            InputStatus::Incomplete
        ),
    }
}
