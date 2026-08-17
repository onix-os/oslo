//! The argv call model: `oslo.run{…}` and the `sh` sugar over it.
//!
//! ```lua
//! local r = oslo.run{"grep", "-n", pattern, file, capture = true}
//! if r.ok then print(r.out) end
//!
//! sh.grep("-n", pattern, file)     -- same call, spelled for the common case
//! ```
//!
//! **Argv end to end.** The list the caller writes is the list the command receives. There is no
//! quoting step, so there is no quoting bug: `oslo.run{"rm", name}` is safe for a `name` holding a
//! space, a `*` or a `;`, where `oslo.proc.exec("rm " .. name)` is one careless value away from running
//! something else entirely. That hole is the standard criticism of shelling out from a scripting
//! language, and closing it is why this exists.
//!
//! **Nothing raises.** A command that fails is not an exceptional event in a shell — it is Tuesday.
//! `r.ok` is `r.status == 0`, and a script that wants an error writes `assert(r.ok, r.err)`.
//!
//! **`sh` is sugar, not a second model.** `sh.grep(a, b)` builds the same argv and calls the same
//! function. It is a global because `sh.ls()` is what people will type all day, and it is a table
//! rather than bare names because `ls()` resolving to a command would mean your own Lua function
//! stops being called the day someone installs a binary with its name.

use super::util::native;
use crate::lua::engine::borrow_env;
use oslo_base::value::{LuaError, LuaResult};
use oslo_base::value::{Table, Value};
use oslo_luavm::Host;
use oslo_shell::env::Environment;
use oslo_shell::exec::argv::{Capture, Outcome};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Install `oslo.run`, `oslo.pipe` and the `sh` global.
pub fn install(host: &dyn Host, oslo: &mut Table, env: &Arc<Mutex<Environment>>) {
    let env_run = Arc::clone(env);
    oslo.set(
        Value::str("run"),
        native("oslo.run", move |_, args| {
            let request = Request::from_lua(args.first(), "run")?;
            let mut guard = borrow_env(&env_run)?;
            let outcome = oslo_shell::exec::argv::run(&mut guard, &request.argv, request.capture)
                .map_err(|e| LuaError::new(format!("oslo.run: {e}")))?;
            Ok(vec![result_table(&outcome)])
        }),
    );

    let env_pipe = Arc::clone(env);
    oslo.set(
        Value::str("pipe"),
        native("oslo.pipe", move |_, args| {
            // Each argument is one stage, in the order the data flows. A method chain would read
            // better and would need the sugar to return a lazy object rather than a result — two
            // command models to keep in sync, for notation.
            let mut stages = Vec::new();
            for (i, stage) in args.iter().enumerate() {
                stages.push(Request::from_lua(Some(stage), "pipe")?.argv);
                if stages[i].is_empty() {
                    return Err(LuaError::new(format!(
                        "oslo.pipe: stage #{} is empty",
                        i + 1
                    )));
                }
            }
            // The trailing stage decides capture, exactly as `a | b > file` does.
            let capture = args
                .last()
                .map(|s| Request::from_lua(Some(s), "pipe"))
                .transpose()?
                .map(|r| r.capture)
                .unwrap_or_default();
            let mut guard = borrow_env(&env_pipe)?;
            let outcome = oslo_shell::exec::argv::pipe(&mut guard, &stages, capture)
                .map_err(|e| LuaError::new(format!("oslo.pipe: {e}")))?;
            Ok(vec![result_table(&outcome)])
        }),
    );

    // oslo.lines{...} -> an iterator over the command's output, a line at a time
    //
    // The other half of the streaming decision. `capture = true` holds the whole output in
    // memory, which is right for `uname -r` and wrong for `cargo build` — and impossible for
    // `journalctl -f`, which never ends and therefore never answers.
    let env_lines = Arc::clone(env);
    oslo.set(
        Value::str("lines"),
        native("oslo.lines", move |_, args| {
            let request = Request::from_lua(args.first(), "lines")?;
            let mut guard = borrow_env(&env_lines)?;
            let (child, reader) = oslo_shell::exec::argv::spawn_reading(&mut guard, &request.argv)
                .map_err(|e| LuaError::new(format!("oslo.lines: {e}")))?;
            drop(guard);
            Ok(vec![line_reader(child, reader)])
        }),
    );

    host.set_global("sh", sugar(env));
}

/// A command being read a line at a time, and the child behind it.
struct Reading {
    child: nix::unistd::Pid,
    /// Emptied once the output runs out or the reader is let go.
    source: std::cell::RefCell<Option<std::io::BufReader<std::fs::File>>>,
    reaped: std::cell::Cell<bool>,
}

impl Reading {
    /// Let the pipe go and collect the child. Safe to call more than once.
    ///
    /// **The reader is dropped before the wait.** Waiting first on a child that is still writing
    /// into a pipe nobody drains is a deadlock; closing the read end sends it `SIGPIPE` instead,
    /// which is what a shell's `head` does to the command feeding it.
    fn finish(&self) {
        self.source.borrow_mut().take();
        if !self.reaped.replace(true) {
            oslo_shell::exec::argv::reap(self.child);
        }
    }

    /// The next line, or `nil` once there are none.
    fn line(&self) -> LuaResult<Vec<Value>> {
        use std::io::BufRead;
        let mut slot = self.source.borrow_mut();
        let Some(buffered) = slot.as_mut() else {
            return Ok(vec![Value::Nil]);
        };
        let mut line = String::new();
        match buffered.read_line(&mut line) {
            Ok(0) => {
                drop(slot);
                self.finish();
                Ok(vec![Value::Nil])
            }
            Ok(_) => {
                line.truncate(line.trim_end_matches('\n').len());
                Ok(vec![Value::str(line)])
            }
            Err(e) => {
                drop(slot);
                self.finish();
                Err(LuaError::new(format!("oslo.lines: {e}")))
            }
        }
    }
}

impl Drop for Reading {
    fn drop(&mut self) {
        self.finish();
    }
}

/// What `oslo.lines` answers: an iterator that is also a handle.
///
/// ```lua
/// for line in oslo.lines{"cargo", "build"} do oslo.ui.log(line) end
///
/// local out <close> = oslo.lines{"journalctl", "-f"}   -- reaped at the end of the block
/// for line in out do if line:find("error") then break end end
/// ```
///
/// **One value, because a `for` and a `<close>` want different things of it.** A generic `for` needs
/// something callable and a scope-bound release needs a metatable, so the handle is callable through
/// `__call` — see [`super::handle::Handle::calls`]. The alternative was two returns the caller had
/// to remember to keep together, and the one they would forget is the one that reaps.
///
/// A loop that runs out reaps the child on its own. A loop that `break`s does not — luna does not
/// close a `for`'s closing value — so `<close>` or `out:close()` is how an abandoned read is
/// collected, and `Drop` is what catches the rest at teardown.
fn line_reader(child: nix::unistd::Pid, reader: std::os::fd::OwnedFd) -> Value {
    let reading = Rc::new(Reading {
        child,
        source: std::cell::RefCell::new(Some(std::io::BufReader::new(std::fs::File::from(reader)))),
        reaped: std::cell::Cell::new(false),
    });

    let mut handle = super::handle::Handle::new("oslo.lines");

    let it = Rc::clone(&reading);
    handle.calls("oslo.lines", move |_, _| it.line());

    handle.on_close("oslo.lines.close", move || reading.finish());

    handle.build()
}

/// The `sh` table: any name on it becomes that command.
///
/// `__index` rather than a fixed set of entries, because the set is "every program on this
/// machine" and it changes while the shell is running. A generated table would be a snapshot of
/// `$PATH` at startup, and `sh.the_thing_i_just_installed` would be nil.
fn sugar(env: &Arc<Mutex<Environment>>) -> Value {
    let env_index = Arc::clone(env);
    let mut meta = Table::new();
    meta.set(
        Value::str("__index"),
        native("sh.__index", move |_, args| {
            let Some(Value::Str(name)) = args.get(1) else {
                return Ok(vec![Value::Nil]);
            };
            let command = name.to_string();
            let env_call = Arc::clone(&env_index);
            // A command oslo can answer in rows does so. `sh.df()` gives a table with named
            // fields rather than text nobody can parse safely — see `docs/features/structured-pipelines.md`.
            // Everything else falls through to running the external program, so a script keeps
            // working on a machine where oslo is not the shell.
            if oslo_shell::data::rows::answers_in_rows(&command) {
                let tool = command.clone();
                let env_rows = Arc::clone(&env_call);
                return Ok(vec![native("sh tool", move |_, args| {
                    // The words the tool was called with, taken directly — no shell parse, so an
                    // argument holding a space arrives as one argument.
                    let mut words = Vec::new();
                    for (i, arg) in args.iter().enumerate() {
                        words.push(word(arg).ok_or_else(|| {
                            LuaError::new(format!(
                                "sh.{tool}: argument #{} is a {}, which is not a word",
                                i + 1,
                                arg.type_name()
                            ))
                        })?);
                    }
                    Ok(vec![
                        oslo_shell::data::rows::row_answer(&tool, &env_rows, &words)
                            .unwrap_or(Value::Nil),
                    ])
                })]);
            }
            Ok(vec![native("sh command", move |_, args| {
                let mut argv = vec![command.clone()];
                for (i, arg) in args.iter().enumerate() {
                    argv.push(word(arg).ok_or_else(|| {
                        LuaError::new(format!(
                            "sh.{command}: argument #{} is a {}, which is not a word",
                            i + 1,
                            arg.type_name()
                        ))
                    })?);
                }
                // **This is where `sh` stops being pure sugar for `oslo.run`, and it is the one
                // difference between them.** `sh.rm("*.txt")` is spelled to read like the command
                // line it stands in for, and a command line expands its patterns; a `sh.` call
                // that did not was the reason `oslo.glob` had to be reached for at all.
                //
                // `oslo.run{…}` remains literal, and is now what a caller reaches for to *stop*
                // this happening — the argument that must not be read as a pattern, whatever it
                // holds. The two forms exist for two intentions and this is what tells them apart.
                let argv = expand_globs(&argv);
                let mut guard = borrow_env(&env_call)?;
                let outcome = oslo_shell::exec::argv::run(&mut guard, &argv, Capture::default())
                    .map_err(|e| LuaError::new(format!("sh.{command}: {e}")))?;
                Ok(vec![result_table(&outcome)])
            })])
        }),
    );

    let table = Rc::new(std::cell::RefCell::new(Table::new()));
    table.borrow_mut().metatable = Some(Rc::new(std::cell::RefCell::new(meta)));
    Value::Table(table)
}

/// One `oslo.run{…}` request: the argv, and what to capture.
struct Request {
    argv: Vec<String>,
    capture: Capture,
}

impl Request {
    /// Read the table form: positional entries are the argv, named ones are options.
    fn from_lua(value: Option<&Value>, function: &str) -> LuaResult<Self> {
        let Some(Value::Table(t)) = value else {
            return Err(LuaError::new(format!(
                "oslo.{function}: expected a table of arguments, got {}",
                value.map_or("no value", Value::type_name)
            )));
        };
        let table = t.borrow();

        let mut argv = Vec::new();
        for (i, entry) in table.sequence().iter().enumerate() {
            argv.push(word(entry).ok_or_else(|| {
                LuaError::new(format!(
                    "oslo.{function}: argument #{} is a {}, which is not a word",
                    i + 1,
                    entry.type_name()
                ))
            })?);
        }

        // `glob = true` opts this call into pathname expansion. **Off by default and staying that
        // way**: the safety this module is built on is that the list written is the list run, so
        // `oslo.run{"rm", name}` cannot turn one file into forty because `name` happened to hold a
        // `*`. A script that wants the pattern read says so, once, at the call site.
        if table.get_str("glob").truthy() {
            argv = expand_globs(&argv);
        }

        // `capture = true` is the short form for both streams, which is what a caller who just
        // wants the output means. The two are separable for the caller who does not.
        let both = table.get_str("capture").truthy();
        Ok(Request {
            argv,
            capture: Capture {
                stdout: both || table.get_str("capture_out").truthy(),
                stderr: both || table.get_str("capture_err").truthy(),
            },
        })
    }
}

/// Pathname expansion over an argv, exactly as a command line gets it.
///
/// # Why this is here at all
///
/// `sh.rm("*.txt")` reported `cannot remove '*.txt': No such file or directory`. There is no shell
/// between Lua and the command — that is the point of this module — so nothing had ever expanded
/// the pattern, and the only way to write it was `table.unpack(oslo.glob("*.txt"))`, which is both
/// obscure and a trap: a call anywhere but last in a table constructor is truncated to one value.
///
/// # The rules are the shell's, deliberately
///
/// **The command word is never expanded**, only its arguments — the same rule
/// `exec::simple::run_simple` follows, and for the same reason: a command whose *name* came out of
/// a glob is never what was meant, and one that expanded to nothing would leave the arguments
/// looking like a command.
///
/// **A pattern that matches nothing stays as it is**, which is POSIX and what every shell without
/// `nullglob` does. It is also what keeps this safe on the arguments that are not patterns at all:
/// `sh.printf("[%s]", x)` has glob metacharacters in it, matches no file, and arrives unchanged.
/// (`oslo.glob` answers an empty table in that case instead — there the caller asked a question
/// about the filesystem, and the pattern back would be a lie.)
fn expand_globs(argv: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(argv.len());
    for (i, word) in argv.iter().enumerate() {
        if i == 0 {
            out.push(word.clone());
            continue;
        }
        // One unquoted run: the string came from Lua, where there is no quoting to have stripped,
        // so every metacharacter in it is live.
        let field = [oslo_shell::expand::Run::new(
            word.clone(),
            oslo_shell::expand::Origin::Literal,
        )];
        out.extend(oslo_shell::expand::glob::expand_glob(&field));
    }
    out
}

/// A Lua value as a command-line word.
///
/// Numbers are words — `sh.head("-n", 5)` is what anyone would write — but nothing else is. A
/// table or a function reaching argv as `table: 0x55f…` is never what was meant, and refusing is
/// how the mistake gets found at the call site instead of in the command's error message.
fn word(value: &Value) -> Option<String> {
    match value {
        Value::Str(s) => Some(s.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The result every command call answers with.
fn result_table(outcome: &Outcome) -> Value {
    let mut table = Table::new();
    table.set_str("status", Value::int(outcome.status as i64));
    table.set_str("ok", Value::Bool(outcome.status == 0));
    // Absent rather than empty when the stream was not captured: `r.out == nil` means nobody
    // listened, `r.out == ""` means the command printed nothing, and a script that cannot tell
    // them apart will eventually treat one as the other.
    if let Some(out) = &outcome.out {
        table.set_str("out", Value::str(out));
    }
    if let Some(err) = &outcome.err {
        table.set_str("err", Value::str(err));
    }
    if let Some(signal) = outcome.signal {
        table.set_str("signal", Value::int(signal as i64));
    }
    Value::table(table)
}
