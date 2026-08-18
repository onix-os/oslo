//! `oslo.completion.columns` — letting a config decide what the dropdown's info columns say.
//!
//! The built-in columns answer for the kinds oslo knows about: a file's size, a directory's entry
//! count, what an alias expands to. This is the seam for everything else. The function is handed
//! one candidate, already `stat`ed, and returns the list of columns to draw:
//!
//! ```lua
//! oslo.completion.columns = function(c)
//!   if c.kind == 'file' then
//!     return { c.description, c.size_human, c.age, c.mode }
//!   end
//! end
//! ```
//!
//! Returning nothing falls back to the built-in columns for that candidate, so a config can answer
//! for the one kind it cares about without having to reimplement the rest.
//!
//! The function runs **once per visible row per frame** — fifteen calls while an arrow key is held,
//! not once per candidate. See [`oslo_ui::dropdown`]'s `columns` module for why that
//! distinction is what makes a `stat` per row affordable at all.

use oslo_base::value::{Table, Value};
use oslo_luavm::{Engine, Host};
use oslo_ui::dropdown::{CompletionCandidate, Facts, human_age, human_mode, human_size};
use std::cell::RefCell;
use std::rc::Rc;

/// The candidate as Lua sees it: what the completer knew, plus what one `stat` found out.
///
/// Both a raw and a rendered form of each measurement, because a config that wants to sort or
/// compare needs the number and a config that wants to draw wants the string — and making it
/// derive `4.2K` from `4300` itself would mean every config reimplementing [`human_size`].
fn candidate_table(cand: &CompletionCandidate, facts: &Facts) -> Value {
    let mut t = Table::new();
    let mut set = |key: &str, value: Value| t.set(Value::str(key), value);

    set("display", Value::str(&cand.display));
    set("replacement", Value::str(&cand.replacement));
    set("kind", opt_str(cand.kind.as_deref()));
    set("description", opt_str(cand.description.as_deref()));
    set("path", opt_str(cand.path.as_deref()));
    // What the completer already knew and the renderer could not work out: an alias's expansion.
    set("detail", opt_str(cand.detail.as_deref()));

    set("is_dir", Value::Bool(facts.is_dir));
    set("is_symlink", Value::Bool(facts.is_symlink));
    set(
        "size",
        facts.size.map_or(Value::Nil, |n| Value::int(n as i64)),
    );
    set("size_human", opt_string(facts.size.map(human_size)));
    set(
        "entries",
        facts.entries.map_or(Value::Nil, |n| Value::int(n as i64)),
    );
    // Whether the count reached the end. A config that prints `c.entries` unqualified on a huge
    // directory would otherwise report a cap as a fact.
    set("entries_capped", Value::Bool(facts.entries_capped));
    set(
        "mtime",
        facts.mtime.map_or(Value::Nil, |t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .map_or(Value::Nil, |d| Value::int(d.as_secs() as i64))
        }),
    );
    set("age", opt_string(facts.mtime.map(human_age)));
    set(
        "mode",
        facts.mode.map_or(Value::Nil, |m| Value::int(m as i64)),
    );
    set("mode_human", opt_string(facts.mode.map(human_mode)));

    Value::Table(Rc::new(RefCell::new(t)))
}

fn opt_str(text: Option<&str>) -> Value {
    text.map_or(Value::Nil, Value::str)
}

fn opt_string(text: Option<String>) -> Value {
    text.map_or(Value::Nil, |t| Value::str(&t))
}

/// How many columns a config may ask for. Well past anything drawable; it exists only to bound
/// the index scan in [`to_columns`].
const MAX_COLUMNS: i64 = 16;

/// What a returned value means as a column list.
///
/// **Read by index rather than as a sequence, and that is the whole point of this function.**
/// `{ c.description, c.size_human, c.age }` has a nil in the middle for any candidate whose size
/// is unknown, and a Lua table constructor puts nothing at that index — so the sequence stops at
/// one element and `age` disappears. That row would then be a column shorter than every other one
/// and rag against them. Scanning the indices instead keeps a hole as a hole.
fn to_columns(values: &[Value]) -> Option<Vec<String>> {
    match values.first() {
        // No return value at all, or an explicit nil: defer to the built-in columns.
        None | Some(Value::Nil) => None,
        Some(Value::Table(table)) => {
            let table = table.borrow();
            let mut columns: Vec<String> = (1..=MAX_COLUMNS)
                .map(|i| cell_text(&table.get(&Value::int(i))))
                .collect();
            // Only what the config actually filled; the scan's tail is not a row of blank columns.
            while columns.last().is_some_and(String::is_empty) {
                columns.pop();
            }
            Some(columns)
        }
        // A bare string is the one-column case, which is what a `return c.size_human` looks like.
        Some(Value::Str(s)) => Some(vec![s.to_string()]),
        Some(Value::Number(n)) => Some(vec![n.to_string()]),
        _ => None,
    }
}

fn cell_text(value: &Value) -> String {
    match value {
        Value::Nil => String::new(),
        Value::Str(s) => s.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        // A table or a function in a column is a mistake worth seeing as a gap rather than a
        // pointer address worth drawing.
        _ => String::new(),
    }
}

/// Install `oslo.completion.columns` as the dropdown's column provider, if the config set one.
///
/// Removes any previously installed provider when it did not, so reloading a config that has
/// dropped the function goes back to the built-in columns rather than keeping the old one.
pub fn install(interp: &Rc<Engine>) {
    let Value::Table(oslo) = interp.global("oslo") else {
        oslo_ui::dropdown::set_provider(None);
        return;
    };
    let function = match oslo.borrow().get_str("completion") {
        Value::Table(completion) => completion.borrow().get_str("columns"),
        _ => Value::Nil,
    };
    if !matches!(function, Value::Function(_)) {
        oslo_ui::dropdown::set_provider(None);
        return;
    }

    let interp = Rc::clone(interp);
    // Errors are reported once and the hook then stays quiet: it runs on every frame, and a
    // function that raises would otherwise scroll the same message past the prompt sixty times a
    // second while an arrow key is held.
    let complained = std::cell::Cell::new(false);
    oslo_ui::dropdown::set_provider(Some(Rc::new(
        move |cand: &CompletionCandidate, facts: &Facts| {
            let arg = candidate_table(cand, facts);
            match interp.call_function(&function, vec![arg]) {
                Ok(values) => to_columns(&values),
                Err(e) => {
                    if !complained.replace(true) {
                        eprintln!("oslo: completion.columns: {e}");
                    }
                    None
                }
            }
        },
    )));
}

/// Install `oslo.completion.for_command` as the per-command completion hook.
///
/// One table of `name -> function`, rather than a call that registers each: a config is read once
/// and the table is what it leaves behind, which is the same shape `oslo.keys` uses.
///
/// ```lua
/// oslo.completion.for_command = {
///   git = function(argv, current)
///     if #argv == 1 then return { "add", "commit", "push", "status" } end
///   end,
/// }
/// ```
pub fn install_command_completer(interp: &Rc<Engine>) {
    let Value::Table(oslo) = interp.global("oslo") else {
        oslo_ui::completion::set_command_completer(None);
        return;
    };
    let table = match oslo.borrow().get_str("completion") {
        Value::Table(completion) => completion.borrow().get_str("for_command"),
        _ => Value::Nil,
    };
    let Value::Table(table) = table else {
        oslo_ui::completion::set_command_completer(None);
        return;
    };

    let interp = Rc::clone(interp);
    let complained = std::cell::Cell::new(false);
    oslo_ui::completion::set_command_completer(Some(Rc::new(
        move |command: &str, prior: &[&str], current: &str| {
            let function = table.borrow().get(&Value::str(command));
            if !matches!(function, Value::Function(_)) {
                return None;
            }
            let argv = Table::new();
            let argv = Rc::new(RefCell::new(argv));
            for (i, word) in prior.iter().enumerate() {
                argv.borrow_mut()
                    .set(Value::int(i as i64 + 1), Value::str(word));
            }
            let args = vec![Value::Table(argv), Value::str(current)];
            match interp.call_function(&function, args) {
                Ok(values) => to_candidates(&values),
                Err(e) => {
                    // Reported once: this runs on every Tab, and a raising hook would otherwise
                    // scroll the same message past the prompt.
                    if !complained.replace(true) {
                        eprintln!("oslo: completion.for_command.{command}: {e}");
                    }
                    None
                }
            }
        },
    )));
}

/// A returned list of candidates: either bare strings, or `{name, description}` pairs.
fn to_candidates(values: &[Value]) -> Option<Vec<(String, Option<String>)>> {
    let Some(Value::Table(table)) = values.first() else {
        return None;
    };
    let mut out = Vec::new();
    for value in table.borrow().sequence() {
        match value {
            Value::Str(s) => out.push((s.to_string(), None)),
            Value::Table(pair) => {
                let pair = pair.borrow();
                let name = match pair.get(&Value::int(1)) {
                    Value::Str(s) => s.to_string(),
                    _ => continue,
                };
                let description = match pair.get(&Value::int(2)) {
                    Value::Str(s) => Some(s.to_string()),
                    _ => None,
                };
                out.push((name, description));
            }
            _ => {}
        }
    }
    Some(out)
}

/// Let the Lua prompt complete against the names that actually exist in this session.
///
/// The editor knows what is being *typed* — where the name starts, whether a dot or a colon comes
/// before it, whether the cursor is inside a string. Only the interpreter knows what a name *is*.
/// This hands over the second half, and nothing but names crosses: see
/// [`oslo_ui::completion::lua`] for the split and [`Engine::names_at`] for why the values stay in
/// the VM.
pub fn install_lua_completer(interp: &Rc<Engine>) {
    let interp = Rc::clone(interp);
    oslo_ui::completion::lua::set_name_source(Some(Rc::new(move |path: &[String]| {
        interp.names_at(path)
    })));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a config and ask the installed provider what it says about `cand`.
    fn columns_of(source: &str, cand: &CompletionCandidate, facts: &Facts) -> Option<Vec<String>> {
        let interp = Rc::new(Engine::new());
        interp
            .eval(source, "columns test")
            .expect("the test chunk must run");
        install(&interp);
        let out = oslo_ui::dropdown::columns_for(cand, facts);
        let builtin = oslo_ui::dropdown::builtin_columns(cand, facts);
        oslo_ui::dropdown::set_provider(None);
        (out != builtin).then_some(out)
    }

    fn file() -> CompletionCandidate {
        let mut c = CompletionCandidate::new("Cargo.toml".into(), "Cargo.toml".into(), None);
        c.kind = Some("file".to_string());
        c
    }

    #[test]
    fn a_config_function_decides_the_columns() {
        let facts = Facts {
            size: Some(4300),
            ..Facts::default()
        };
        let out = columns_of(
            "oslo = { completion = { columns = function(c) \
               return { c.kind, c.size_human } end } }",
            &file(),
            &facts,
        );
        assert_eq!(
            out,
            Some(vec!["file".to_string(), "4.2K".to_string()]),
            "the config's columns must reach the dropdown"
        );
    }

    /// Answering for one kind and returning nothing for the rest must leave the rest alone —
    /// otherwise every config would have to reimplement the built-in columns to change one.
    #[test]
    fn returning_nothing_falls_back_to_the_built_in_columns() {
        let facts = Facts::default();
        let out = columns_of(
            "oslo = { completion = { columns = function(c) \
               if c.kind == 'dir' then return { 'D' } end end } }",
            &file(),
            &facts,
        );
        assert_eq!(out, None, "a file must keep its built-in columns");
    }

    /// A nil in the middle is an empty column, not the end of the list. This is not hypothetical:
    /// `{ c.description, c.size_human, c.age }` has a nil in the middle for any candidate whose
    /// size is unknown, and reading the table as a *sequence* stops there and loses `age` — that
    /// row would come out a column short and rag against every other one.
    #[test]
    fn a_nil_column_is_blank_rather_than_the_end_of_the_row() {
        let interp = Rc::new(Engine::new());
        interp
            .eval("t = { 'a', nil, 'c' }", "columns test")
            .expect("runs");
        // The evaluator's constructor really does stop the sequence at the hole, which is what
        // makes the index scan necessary rather than merely tidy.
        let Value::Table(raw) = interp.global("t") else {
            panic!("t is a table")
        };
        assert_eq!(raw.borrow().sequence().len(), 1);

        let columns = to_columns(&[interp.global("t")]).expect("a table is a column list");
        assert_eq!(
            columns,
            vec!["a".to_string(), String::new(), "c".to_string()]
        );
    }

    /// The scan's tail is not a row of sixteen blank columns.
    #[test]
    fn only_the_columns_the_config_filled_are_drawn() {
        let interp = Rc::new(Engine::new());
        interp
            .eval("t = { 'a', 'b' }", "columns test")
            .expect("runs");
        let columns = to_columns(&[interp.global("t")]).expect("a table is a column list");
        assert_eq!(columns, vec!["a".to_string(), "b".to_string()]);
    }

    /// A function that raises must not print on every frame — it runs while an arrow key is held.
    #[test]
    fn a_raising_function_falls_back_rather_than_taking_the_dropdown_down() {
        let facts = Facts::default();
        let out = columns_of(
            "oslo = { completion = { columns = function(c) error('nope') end } }",
            &file(),
            &facts,
        );
        assert_eq!(out, None, "a broken hook must leave the built-ins standing");
    }

    /// Every measurement is offered raw *and* rendered, so a config need not reimplement the
    /// formatting to draw it or parse the string back to compare it.
    #[test]
    fn a_candidate_carries_both_the_number_and_the_string() {
        let facts = Facts {
            size: Some(4300),
            mode: Some(0o755),
            entries: Some(12),
            ..Facts::default()
        };
        let Value::Table(t) = candidate_table(&file(), &facts) else {
            panic!("a candidate is a table");
        };
        let t = t.borrow();
        let int = |k: &str| t.get(&Value::str(k)).as_number().and_then(|n| n.as_int());
        let text = |k: &str| match t.get(&Value::str(k)) {
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        };
        assert_eq!(int("size"), Some(4300));
        assert_eq!(text("size_human").as_deref(), Some("4.2K"));
        assert_eq!(int("mode"), Some(0o755));
        assert_eq!(text("mode_human").as_deref(), Some("rwxr-xr-x"));
        assert_eq!(int("entries"), Some(12));
        // Absent facts are nil rather than zero: a file whose `stat` failed has no size, and a
        // config testing `if c.size then` must be able to tell that from an empty file.
        assert!(matches!(t.get_str("mtime"), Value::Nil));
    }
}
