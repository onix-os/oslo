//! Applying `oslo.keys` to the line editor.
//!
//! Split from `repl` because it is the one place the editor's key map is touched, and because the
//! language toggle needs a different kind of binding from everything else: rustyline has no
//! command that hands control back to the caller, so the toggle is a conditional handler that sets
//! a flag the loop reads.

use super::mode::{self, ToggleRequest};
use super::repl::Repl;
use oslo::Environment;
use oslo::interactive::vi;
use rustyline::{Cmd, ConditionalEventHandler, Event, EventContext, RepeatCount};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Watches every keystroke to keep the cursor shape agreeing with the vi mode.
///
/// A wildcard handler that **never handles anything**: it reads the mode rustyline is now in,
/// writes a cursor escape if that changed, and returns `None` so the real binding for the key runs
/// exactly as it would have. This is the only place rustyline exposes the vi mode at all —
/// `EventContext::input_mode` — and it is why fish's "the cursor tells you the mode" is possible
/// here without patching the editor.
struct ViCursor {
    cursors: vi::Cursors,
}

impl ConditionalEventHandler for ViCursor {
    fn handle(&self, event: &Event, _: RepeatCount, _: bool, ctx: &EventContext) -> Option<Cmd> {
        let now = match ctx.input_mode() {
            rustyline::InputMode::Insert => vi::Mode::Insert,
            rustyline::InputMode::Command => vi::Mode::Normal,
            rustyline::InputMode::Replace => vi::Mode::Replace,
        };
        // **The key has not been applied yet**, so `input_mode` is the mode this keystroke is
        // about to leave. Reporting it as-is drew the cursor one key behind: Esc changed nothing
        // until you pressed something else, which is exactly when a mode indicator is useless.
        //
        // So the keys that *enter* a mode are predicted. This is a short list, not the vi keymap:
        // everything else leaves the mode alone, and a key that is mispredicted corrects itself on
        // the very next keystroke because the real mode is read again then.
        let mode = vi::after_key(now, key_char(event));
        // The mode changed, so the prompt is now wrong. rustyline will not redraw it and cannot be
        // asked to, so oslo draws the row again itself from what the highlighter recorded — see
        // `prompt::repaint`. Without this the letter sat there saying `I` while the cursor had
        // already become a block, which is worse than showing nothing.
        if let Some(escape) = vi::observe(mode, &self.cursors) {
            // Where the cursor *actually* is, not where the line ends: the editor hands over the
            // line and the byte position, so the column is exact. Using the end of the line
            // instead dragged the cursor right on every mode change.
            let cursor = oslo::interactive::prompt::printed_width(&ctx.line()[..ctx.pos()]);
            let mut out = std::io::stdout();
            let _ = out.write_all(escape.as_bytes());
            // The row after the cursor shape, so the prompt and the cursor agree in one frame
            // rather than flickering between two.
            let _ =
                out.write_all(oslo::interactive::prompt::repaint(ctx.line(), cursor).as_bytes());
            let _ = out.flush();
        }
        // Declined on purpose: this handler observes, it does not bind.
        None
    }
}

/// Walks oslo's own history instead of the editor's, so recall follows the language.
///
/// The editor holds one history and it is refilled only when the read loop regains control — that
/// is, after Enter. The language can change in the middle of a line, so until the line ends the
/// editor is still holding the other language's history and Up recalls a shell command at a Lua
/// prompt. It cannot be fixed by swapping that history: the toggle runs from inside `readline`,
/// which already holds the only mutable borrow of the editor.
///
/// So Up and Down are answered here, from the same set the suggestion reads, filtered by the
/// language the prompt is showing *now*.
struct HistoryWalk {
    back: bool,
}

impl ConditionalEventHandler for HistoryWalk {
    fn handle(&self, _: &Event, _: RepeatCount, _: bool, ctx: &EventContext) -> Option<Cmd> {
        let language = oslo::interactive::prompt::language()?;
        let lines = oslo::interactive::recall::for_language(&language);
        if lines.is_empty() {
            // Nothing remembered at all — no database, first ever session — so let the editor
            // answer as it always did. But if other languages have history and this one does not,
            // answering with theirs is the very thing this handler exists to prevent.
            return if oslo::interactive::recall::is_empty() {
                None
            } else {
                Some(Cmd::Noop)
            };
        }
        // **What the walk is anchored to.** A walk that has not started yet takes the line as it
        // stands now: that is both the prefix to filter by and the text to give back if the user
        // walks all the way forward again. Recomputing it mid-walk would anchor to whatever the
        // history just put on the line, so it is captured once and kept until the walk ends.
        let mut walk = WALK.lock().ok()?;
        // A walk belongs to the line it started on. If what is on screen is not what this walk
        // last put there, the user has typed, cleared or edited since — so the old prefix is no
        // longer what they are looking for, and the walk starts again from where they now are.
        let fresh = !matches!(
            walk.as_ref(),
            Some((walked, _, _, shown)) if walked == &language && shown == ctx.line()
        );
        let anchor = if fresh {
            ctx.line().to_string()
        } else {
            walk.as_ref()
                .map(|(_, _, a, _)| a.clone())
                .unwrap_or_default()
        };
        // Only the entries that continue what is already typed. An empty anchor matches
        // everything, so an empty line still walks the whole history the way it always did.
        let lines: Vec<String> = lines
            .into_iter()
            .filter(|l| l.starts_with(&anchor))
            .collect();
        if lines.is_empty() {
            // Nothing continues this prefix. Doing nothing is the honest answer — clearing the
            // line or beeping would both lose what the user is part-way through typing.
            return Some(Cmd::Noop);
        }
        // **A walk belongs to one language.** Switching in the middle of one starts a new walk
        // rather than carrying the old position across: the two histories have nothing to do with
        // each other, and a position three back in Lua means nothing in shell. Without this,
        // toggling mid-walk left a stale depth behind and the next Up recalled the wrong entry or
        // nothing at all.
        let depth = if fresh {
            0
        } else {
            walk.as_ref().map(|(_, d, _, _)| *d).unwrap_or(0)
        };
        // A walk that has not started begins just past the newest entry, so the first Up lands on
        // it. The depth counts back from the end: 1 is the newest.
        let depth = if self.back {
            (depth + 1).min(lines.len())
        } else {
            depth.saturating_sub(1)
        };
        // What this walk is about to put on screen, so the next press can tell an untouched line
        // from an edited one.
        let showing = if depth == 0 {
            anchor.clone()
        } else {
            lines.get(lines.len() - depth).cloned().unwrap_or_default()
        };
        *walk = Some((language.clone(), depth, anchor.clone(), showing));
        // Walked forward past the newest entry: back to the line the user was composing when the
        // walk started. It used to come back *empty* — the editor keeps no copy, so this module
        // keeps one. Pressing Up and then Down again deleting what you had typed is a small piece
        // of data loss that happens several times a day.
        if depth == 0 {
            // `Some(...)`, never `None`. A command returned from a binding is treated as
            // repeatable and rewritten before it runs: `Replace(mvt, None)` has the *last inserted
            // text* substituted for the `None`, so clearing the line pasted the last thing typed
            // back into it. Naming the replacement leaves nothing to substitute.
            return Some(Cmd::Replace(rustyline::Movement::WholeLine, Some(anchor)));
        }
        let line = lines.get(lines.len() - depth)?.clone();
        // Already showing it — a second Down at the newest entry should not flicker.
        if line == ctx.line() {
            return Some(Cmd::Noop);
        }
        Some(Cmd::Replace(rustyline::Movement::WholeLine, Some(line)))
    }
}

/// Forget where a history walk had got to. Called when a new line starts.
pub fn reset_history_walk() {
    if let Ok(mut walk) = WALK.lock() {
        *walk = None;
    }
}

/// Shared so Up and Down walk the same position: the language it belongs to, how far back it has
/// gone, the line the walk started from — which is both the prefix being matched and the text
/// handed back when the walk returns to the present — and what it last put on screen, so an edited
/// line can be told from an untouched one.
static WALK: Mutex<Option<(String, usize, String, String)>> = Mutex::new(None);

/// Expands an abbreviation when the space that ends its word is typed.
///
/// The space is part of the expansion, not a separate keystroke: `gco ` becomes `git checkout `
/// in one step, so what you see after pressing space is a finished command rather than a word
/// waiting to be finished.
struct Abbreviations;

impl ConditionalEventHandler for Abbreviations {
    fn handle(&self, _: &Event, _: RepeatCount, _: bool, ctx: &EventContext) -> Option<Cmd> {
        // Only at the end of a word being typed. Expanding while the cursor is in the middle of
        // the line would rewrite text the user is editing, not text they are writing.
        let (text, cursor) = oslo::interactive::abbr::expand(ctx.line(), ctx.pos())?;
        // The space the user pressed, added here rather than left to rustyline: this handler
        // consumes the keystroke, so it owes the line that character.
        let mut text = text;
        text.insert(cursor, ' ');
        let _ = cursor;
        Some(Cmd::Replace(rustyline::Movement::WholeLine, Some(text)))
    }
}

/// Runs a Lua handler for one key, and applies whatever it says the line should become.
///
/// The handler is given a description of the line and answers with a description of the line it
/// wants — see [`oslo::interactive::editor`]. Nothing is mutated while Lua runs, which matters
/// because a handler may do anything at all, including opening another prompt.
struct LuaKey {
    key: String,
}

impl ConditionalEventHandler for LuaKey {
    fn handle(&self, _: &Event, _: RepeatCount, _: bool, ctx: &EventContext) -> Option<Cmd> {
        let handler = oslo::interactive::editor::handler(&self.key)?;
        let line = oslo::interactive::editor::line_table(ctx.line(), ctx.pos());
        let answer = match oslo::lua::engine::call_here(&handler, vec![line]) {
            Ok(values) => values.into_iter().next().unwrap_or_default(),
            Err(e) => {
                // Reported above the prompt rather than swallowed: a binding that silently does
                // nothing is indistinguishable from one that was never installed.
                eprintln!("oslo: keys['{}']: {e}", self.key);
                return Some(Cmd::Noop);
            }
        };
        let Some((text, cursor)) = oslo::interactive::editor::line_from(&answer) else {
            // The handler looked but did not ask for a change.
            return Some(Cmd::Noop);
        };
        // Replace the whole line, then put the cursor where the handler asked. `Some(text)` and
        // never `None`: a `Replace` with no text has the *last inserted text* substituted into it
        // when the editor replays the command.
        let _ = cursor;
        Some(Cmd::Replace(rustyline::Movement::WholeLine, Some(text)))
    }
}

/// The character a key event carries, if it is a plain one.
///
/// Esc arrives as its own key code rather than as a character, so it is reported as `\x1b` — which
/// is what it is, and what [`vi::after_key`] matches on.
fn key_char(event: &Event) -> Option<char> {
    let Event::KeySeq(keys) = event else {
        return None;
    };
    let key = keys.first()?;
    // Only an unmodified key. Ctrl-R arrives as `Char('r')` with a modifier, and reading it as a
    // bare `r` would have it answered by whatever `r` means — a real confusion now that the
    // insert-starting keys are matched on the character alone.
    if key.1 != rustyline::Modifiers::NONE {
        return None;
    }
    match key.0 {
        rustyline::KeyCode::Char(c) => Some(c),
        rustyline::KeyCode::Esc => Some('\x1b'),
        _ => None,
    }
}

/// Apply `oslo.keys`, plus the language toggle.
///
/// The toggle is bound last so a config that puts something else on the same key wins: the config
/// is a later, more specific statement than the default.
pub fn apply(rl: &mut Repl, env_struct: &Arc<Mutex<Environment>>, toggle: &ToggleRequest) {
    let settings = oslo::interactive::settings::current();

    // Bound before `oslo.keys`, because a wildcard that declines every event must not shadow a
    // real binding — and rustyline consults the more specific one first regardless.
    vi::set_enabled(settings.vi.enabled);
    if settings.vi.enabled {
        rl.bind_sequence(
            Event::Any,
            rustyline::EventHandler::Conditional(Box::new(ViCursor {
                cursors: settings.vi.cursors,
            })),
        );
        // **Tab completes in normal mode too.** rustyline's vi keymap leaves Tab unbound there, so
        // pressing it did nothing whatsoever — and since nothing happens, the natural response is
        // to press it again, which is how "the dropdown needs five presses" was really "you were
        // in normal mode and the first four did nothing".
        //
        // Safe to bind: Tab is not a vi command. Nothing is being taken away.
        rl.bind_sequence(
            Event::KeySeq(vec![rustyline::KeyEvent(
                rustyline::KeyCode::Tab,
                rustyline::Modifiers::NONE,
            )]),
            rustyline::EventHandler::Simple(Cmd::Complete),
        );

        // The prompt is drawn before any key is pressed, so the starting shape has to be written
        // by hand — otherwise the first line of the session has whatever cursor the terminal had.
        let mut out = std::io::stdout();
        if std::io::IsTerminal::is_terminal(&out) {
            let _ = out.write_all(settings.vi.cursors.insert.escape().as_bytes());
            let _ = out.flush();
        }
    }

    // Up and Down before `oslo.keys`, so a config can still take them.
    for (code, back) in [
        (rustyline::KeyCode::Up, true),
        (rustyline::KeyCode::Down, false),
        (rustyline::KeyCode::Char('p'), true),
        (rustyline::KeyCode::Char('n'), false),
    ] {
        let modifiers = if matches!(code, rustyline::KeyCode::Char(_)) {
            rustyline::Modifiers::CTRL
        } else {
            rustyline::Modifiers::NONE
        };
        rl.bind_sequence(
            Event::KeySeq(vec![rustyline::KeyEvent(code, modifiers)]),
            rustyline::EventHandler::Conditional(Box::new(HistoryWalk { back })),
        );
    }

    // Space expands an abbreviation, if the word just typed is one. Bound unconditionally but
    // cheap: with no abbreviations defined the handler is one hash lookup that answers `None`, and
    // declining leaves rustyline to insert the space exactly as it would have.
    rl.bind_sequence(
        Event::KeySeq(vec![rustyline::KeyEvent(
            rustyline::KeyCode::Char(' '),
            rustyline::Modifiers::NONE,
        )]),
        rustyline::EventHandler::Conditional(Box::new(Abbreviations)),
    );

    let (bindings, problems) = oslo::interactive::keys::resolve(&settings.keys);
    for problem in problems {
        eprintln!("oslo: {problem}");
    }

    // `oslo.suggest.accept` / `.accept_word` name the same actions `oslo.keys` can bind, under the
    // names the suggestion settings use. Applied first so an explicit `oslo.keys` entry on the same
    // key still wins: a later, more specific statement beats a general one.
    for (key, action) in [
        (settings.suggest.accept.as_deref(), "accept-suggestion"),
        (
            settings.suggest.accept_word.as_deref(),
            "accept-suggestion-word",
        ),
    ] {
        let Some(key) = key else { continue };
        match oslo::interactive::keys::parse_key(key) {
            Some(event) => {
                if let Some(command) =
                    oslo::interactive::keys::action(action).and_then(|a| a.command())
                {
                    rl.bind_sequence(event, rustyline::EventHandler::Simple(command));
                }
            }
            None => eprintln!("oslo: oslo.suggest: '{key}' is not a key name"),
        }
    }

    let mut toggle_bound = false;
    for (event, action) in bindings {
        // A binding the config wrote itself. Bound before the fixed actions are consulted, so a
        // key that has a handler runs the handler.
        if action == oslo::interactive::keys::Action::LuaHandler
            && let Some(key) = oslo::interactive::keys::name_of(&event)
        {
            rl.bind_sequence(
                event,
                rustyline::EventHandler::Conditional(Box::new(LuaKey { key })),
            );
            continue;
        }
        match action.command() {
            Some(command) => {
                rl.bind_sequence(event, rustyline::EventHandler::Simple(command));
            }
            // The toggle hands control back to this loop, which no editor command can do.
            None => {
                rl.bind_sequence(
                    event,
                    rustyline::EventHandler::Conditional(Box::new(toggle.clone())),
                );
                toggle_bound = true;
            }
        }
    }

    if !toggle_bound && let Some(key) = mode::toggle_key(&env_struct.lock().unwrap()) {
        rl.bind_sequence(
            key,
            rustyline::EventHandler::Conditional(Box::new(toggle.clone())),
        );
    }
}

/// A key `bind` gave shell commands to run.
///
/// It records the request and answers `AcceptLine`, which is the only way out of rustyline that
/// gives the caller both the buffer and control. The read loop recognises the request, so the
/// line is *not* run as a command — see [`crate::startup::integration`] for what happens next and
/// why it cannot happen here.
///
/// The commands are worked out **when the key is pressed**, not when it is bound: a macro names
/// key sequences, and what those are bound to can change between one prompt and the next. atuin
/// rebinds its own widgets as the keymap changes, and resolving early would have run whatever the
/// chain meant at startup.
struct BindCommand {
    /// The key sequence this binding stands for. One event for a `bind -x` key; the macro's
    /// expansion for a macro.
    keys: Vec<rustyline::KeyEvent>,
}

impl ConditionalEventHandler for BindCommand {
    fn handle(&self, _: &Event, _: RepeatCount, _: bool, ctx: &EventContext) -> Option<Cmd> {
        let commands = oslo::interactive::readline::expand(&self.keys);
        if commands.is_empty() {
            // Nothing to run, so nothing should happen — least of all ending the line.
            return Some(Cmd::Noop);
        }
        oslo::interactive::readline::request(commands, ctx.line(), ctx.pos());
        Some(Cmd::AcceptLine)
    }
}

/// Whether `bind` has changed anything since the bindings were last applied.
///
/// One atomic load in the common case, which is what makes it safe to ask before every prompt.
pub fn bindings_changed() -> bool {
    oslo::interactive::readline::generation() != APPLIED.load(Ordering::SeqCst)
}

/// The generation the editor's bindings were last built from.
static APPLIED: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Apply everything `bind` has recorded, and report the generation it was applied at.
///
/// Called from the read loop rather than once at startup, because `bind` is shell code: it runs
/// from an rc file, from a function, or from an `eval` typed into a shell that is already up.
/// Binding only at startup meant `eval "$(atuin init bash)"` at the prompt did nothing until the
/// next restart, which is the kind of half-working that costs an evening to diagnose.
pub fn apply_bindings(rl: &mut Repl) -> usize {
    use oslo::interactive::readline::{self, Bound};
    let generation = readline::generation();
    APPLIED.store(generation, Ordering::SeqCst);
    for entry in readline::entries() {
        // Only the keymap in force. A vi-command binding installed while you are typing is not a
        // shortcut, it is a character that no longer types itself — atuin binds `/` and `k` there,
        // and applying them made `ls /tmp` open a history search mid-word.
        if !entry.keymap.is_active() {
            continue;
        }
        let event = Event::KeySeq(entry.keys.clone());
        match &entry.bound {
            // Bound to its *own* keys, not to the command: `expand` looks the sequence up again
            // when the key is pressed, so one path covers both a direct command and a macro.
            Bound::Command(_) => rl.bind_sequence(
                event,
                rustyline::EventHandler::Conditional(Box::new(BindCommand {
                    keys: entry.keys.clone(),
                })),
            ),
            Bound::Macro { keys, .. } => rl.bind_sequence(
                event,
                rustyline::EventHandler::Conditional(Box::new(BindCommand { keys: keys.clone() })),
            ),
            // A readline *function* name. oslo maps the few that name something it has and leaves
            // the rest alone rather than binding a key to nothing — `bind -P` will still list it,
            // so a user can see what was asked for and what happened to it.
            Bound::Function(name) => match readline_function(name) {
                Some(command) => rl.bind_sequence(event, rustyline::EventHandler::Simple(command)),
                None => continue,
            },
        };
    }
    generation
}

/// readline function names oslo has an equivalent for.
///
/// Deliberately short. These are the ones an init script binds in passing, and inventing a mapping
/// for the rest of readline's ~150 functions would be guessing at behaviour oslo does not have.
fn readline_function(name: &str) -> Option<Cmd> {
    Some(match name {
        "accept-line" => Cmd::AcceptLine,
        "beginning-of-line" => Cmd::Move(rustyline::Movement::BeginningOfLine),
        "end-of-line" => Cmd::Move(rustyline::Movement::EndOfLine),
        "backward-char" => Cmd::Move(rustyline::Movement::BackwardChar(1)),
        "forward-char" => Cmd::Move(rustyline::Movement::ForwardChar(1)),
        "backward-word" => Cmd::Move(rustyline::Movement::BackwardWord(1, rustyline::Word::Emacs)),
        "forward-word" => Cmd::Move(rustyline::Movement::ForwardWord(
            1,
            rustyline::At::AfterEnd,
            rustyline::Word::Emacs,
        )),
        "clear-screen" => Cmd::ClearScreen,
        "complete" => Cmd::Complete,
        "kill-line" => Cmd::Kill(rustyline::Movement::EndOfLine),
        "unix-line-discard" => Cmd::Kill(rustyline::Movement::BeginningOfLine),
        "previous-history" => Cmd::PreviousHistory,
        "next-history" => Cmd::NextHistory,
        _ => return None,
    })
}
