//! Turning the `oslo` table a config leaves behind into [`super::Settings`].
//!
//! Split from the definitions next door for the reason the 600-line limit exists to force: what a
//! setting *is* and how it is *read* are two different subjects, and the reading is the half that
//! grows every time a knob is added. `super::theme` is laid out the same way.
//!
//! Every knob merges over the default rather than replacing the struct, so naming one field cannot
//! blank the rest — and every value nothing answers to is reported rather than ignored, because a
//! typo that silently leaves a feature off looks exactly like the feature being broken.

use super::{Settings, Sort, Source};
use crate::interactive::matching::Fuzzy;
use crate::lua::eval::Value;

pub fn read_lua_settings(oslo: &Value) -> (Settings, Vec<String>) {
    let mut settings = Settings::default();
    let mut problems = Vec::new();
    let Value::Table(oslo) = oslo else {
        return (settings, problems);
    };
    let oslo = oslo.borrow();

    if let Value::Table(table) = oslo.get(&Value::str("dirs")) {
        for (key, value) in table.borrow().pairs() {
            match (&key, &value) {
                (Value::Str(name), Value::Str(path)) => {
                    settings.dirs.push((name.to_string(), path.to_string()));
                }
                _ => problems
                    .push("oslo.dirs: every entry must be a name mapped to a path".to_string()),
            }
        }
        settings.dirs.sort();
    }

    if let Value::Table(table) = oslo.get(&Value::str("notify")) {
        let table = table.borrow();
        if let Some(n) = number(&table, "after") {
            settings.notify.after = n.max(0) as u64;
        }
        if let Value::Str(title) = table.get(&Value::str("title")) {
            settings.notify_text.title = Some(title.to_string());
        }
        if let Value::Str(command) = table.get(&Value::str("command")) {
            settings.notify_text.command = Some(command.to_string());
        }
    }

    if let Value::Table(table) = oslo.get(&Value::str("vi")) {
        let table = table.borrow();
        flag(&table, "enabled", &mut settings.vi.enabled);
        let c = &mut settings.vi.cursors;
        cursor(&table, "cursor_insert", &mut c.insert, &mut problems);
        cursor(&table, "cursor_normal", &mut c.normal, &mut problems);
        cursor(&table, "cursor_replace", &mut c.replace, &mut problems);
    }

    if let Value::Table(table) = oslo.get(&Value::str("completion")) {
        let table = table.borrow();
        if let Some(n) = number(&table, "max_rows") {
            // One row is still a dropdown; zero would be a dropdown that never appears, which is
            // what turning completion off looks like and is not what a row count means.
            settings.completion.max_rows =
                (n.max(1) as usize).min(crate::interactive::dropdown::CEILING_ROWS);
        }
        flag(
            &table,
            "descriptions",
            &mut settings.completion.descriptions,
        );
        flag(&table, "show_kind", &mut settings.completion.show_kind);
        flag(
            &table,
            "case_sensitive",
            &mut settings.completion.case_sensitive,
        );
        if let Value::Table(list) = table.get(&Value::str("sources")) {
            let mut kinds = Vec::new();
            for value in list.borrow().sequence() {
                if let Value::Str(name) = value {
                    let name = match name.as_ref() {
                        "directory" => "dir",
                        "function" | "func" => "function",
                        other => other,
                    };
                    if !kinds.iter().any(|k: &String| k == name) {
                        kinds.push(name.to_string());
                    }
                }
            }
            settings.completion.sources = Some(kinds);
        }
        if let Value::Str(name) = table.get(&Value::str("sort")) {
            match name.as_ref() {
                "frecency" | "frequency" => settings.completion.sort = Sort::Frecency,
                "alpha" | "alphabetical" | "name" => settings.completion.sort = Sort::Alpha,
                other => problems.push(format!(
                    "oslo.completion.sort: '{other}' is not an order; use 'frecency' or 'alpha'"
                )),
            }
        }
        fuzzy(
            &table,
            "oslo.completion.fuzzy",
            &mut settings.completion.fuzzy,
            &mut problems,
        );
    }

    if let Value::Table(table) = oslo.get(&Value::str("suggest")) {
        let table = table.borrow();
        if let Value::Table(list) = table.get(&Value::str("sources")) {
            let mut sources = Vec::new();
            for value in list.borrow().sequence() {
                let Value::Str(name) = value else { continue };
                match Source::parse(name) {
                    Some(source) if !sources.contains(&source) => sources.push(source),
                    // A duplicate is harmless and silently ignored; a name nothing answers to is
                    // a typo, and a typo that turns a source off without saying so is exactly the
                    // kind of thing that gets blamed on the shell.
                    Some(_) => {}
                    None => problems.push(format!(
                        "oslo.suggest.sources: '{name}' is not a source; \
                         the sources are history, completion and path"
                    )),
                }
            }
            settings.suggest.sources = sources;
        }
        if let Value::Str(key) = table.get(&Value::str("accept")) {
            settings.suggest.accept = Some(key.to_string());
        }
        if let Value::Str(key) = table.get(&Value::str("accept_word")) {
            settings.suggest.accept_word = Some(key.to_string());
        }
    }

    if let Value::Table(table) = oslo.get(&Value::str("misc")) {
        let table = table.borrow();
        if let Value::Bool(on) = table.get(&Value::str("welcome")) {
            settings.misc.welcome = on;
        }
        if let Value::Bool(on) = table.get(&Value::str("warnings")) {
            settings.misc.warnings = on;
        }
        if let Value::Str(greeting) = table.get(&Value::str("greeting")) {
            settings.misc.greeting = Some(greeting.to_string());
        }
        if let Some(ms) = number(&table, "escape_delay") {
            // Clamped rather than refused: a zero would make every arrow key over ssh read as Esc,
            // and a delay longer than a second is a shell that appears to have stopped responding
            // to Esc at all.
            settings.misc.escape_delay = ms.clamp(1, 2000) as u64;
        }
        if let Value::Str(depth) = table.get(&Value::str("color_depth")) {
            match crate::interactive::theme::Depth::named(&depth) {
                Some(_) => settings.misc.color_depth = Some(depth.to_string()),
                None => problems.push(format!(
                    "oslo.misc.color_depth: {depth:?} is not a depth; \
                     it is truecolor, 256, 16 or none"
                )),
            }
        }
    }

    if let Value::Table(table) = oslo.get(&Value::str("finder")) {
        let table = table.borrow();
        if let Value::Bool(on) = table.get(&Value::str("enabled")) {
            settings.finder.enabled = on;
        }
        if let Value::Str(key) = table.get(&Value::str("key")) {
            // Checked here rather than at bind time, so a typo is reported next to the line that
            // wrote it instead of silently leaving the finder unreachable.
            if crate::interactive::keys::is_key_name(&key) {
                settings.finder.key = key.to_string();
            } else {
                problems.push(format!("oslo.finder.key: '{key}' is not a key name"));
            }
        }
        if let Some(n) = number(&table, "limit") {
            settings.finder.limit = n.max(1) as usize;
        }
        flag(
            &table,
            "confirm_delete",
            &mut settings.finder.confirm_delete,
        );
    }

    if let Value::Table(table) = oslo.get(&Value::str("keys")) {
        for (key, action) in table.borrow().pairs() {
            match (&key, &action) {
                (Value::Str(key), Value::Str(action)) => {
                    settings.keys.push((key.to_string(), action.to_string()));
                }
                // A function is a binding the config wrote itself. Kept aside rather than named,
                // because a `Value` cannot live in `Settings` — the settings are plain data,
                // readable without an interpreter, and a Lua function is neither.
                (Value::Str(key), Value::Function(_)) => {
                    crate::interactive::editor::register(key, action.clone());
                    settings.keys.push((
                        key.to_string(),
                        crate::interactive::editor::ACTION.to_string(),
                    ));
                }
                _ => problems.push(
                    "oslo.keys: an entry is a key name mapped to an action name or to a function"
                        .to_string(),
                ),
            }
        }
        // Table iteration has no order, and a binding that depends on which of two entries was
        // applied last is one that behaves differently between runs.
        settings.keys.sort();
    }

    if let Value::Table(table) = oslo.get(&Value::str("abbr")) {
        // Sorted by name, for the reason `oslo.keys` is: Lua table iteration has no order, and two
        // runs of the same config must install the same thing in the same sequence.
        let mut defined: Vec<(String, String, bool)> = Vec::new();
        for (key, value) in table.borrow().pairs() {
            let Value::Str(name) = key else {
                problems
                    .push("oslo.abbr: every key must be the word being abbreviated".to_string());
                continue;
            };
            match value {
                // The short form, which is what almost every entry is: `gco = "git checkout"`.
                Value::Str(expansion) => {
                    defined.push((name.to_string(), expansion.to_string(), false))
                }
                // The long form, for the one in ten that needs to fire outside command position.
                Value::Table(spec) => {
                    let spec = spec.borrow();
                    let expansion = match spec.get(&Value::str("expansion")) {
                        Value::Str(text) => text.to_string(),
                        // `{ "git checkout", anywhere = true }` — the expansion as the first
                        // element, which is how the same table reads when written by hand.
                        _ => match spec.get(&Value::int(1)) {
                            Value::Str(text) => text.to_string(),
                            _ => {
                                problems.push(format!(
                                    "oslo.abbr.{name}: needs an expansion, \
                                     either as `expansion = ...` or as the first element"
                                ));
                                continue;
                            }
                        },
                    };
                    let anywhere = matches!(spec.get(&Value::str("anywhere")), Value::Bool(true));
                    defined.push((name.to_string(), expansion, anywhere));
                }
                _ => problems.push(format!(
                    "oslo.abbr.{name}: an abbreviation expands to a string"
                )),
            }
        }
        defined.sort();
        settings.abbr = defined;
    }

    if let Value::Table(table) = oslo.get(&Value::str("history")) {
        let table = table.borrow();
        if let Some(n) = number(&table, "size") {
            settings.history.size = Some(n.max(0) as usize);
        }
        if let Value::Str(file) = table.get(&Value::str("file")) {
            settings.history.file = Some(file.to_string());
        }
        flag(&table, "ignore_space", &mut settings.history.ignore_space);
        flag(&table, "ignore_dups", &mut settings.history.ignore_dups);
        if let Value::Table(list) = table.get(&Value::str("ignore")) {
            for value in list.borrow().sequence() {
                match value {
                    Value::Str(pattern) => settings.history.ignore.push(pattern.to_string()),
                    _ => problems
                        .push("oslo.history.ignore: every entry must be a pattern".to_string()),
                }
            }
        }
    }

    (settings, problems)
}

fn number(table: &crate::lua::eval::Table, name: &str) -> Option<i64> {
    table.get(&Value::str(name)).as_number()?.as_int()
}

/// A cursor-shape field, left alone when the config does not mention it.
///
/// A name nothing answers to is reported rather than silently ignored: a cursor that quietly keeps
/// its default looks exactly like oslo not reading the config at all.
fn cursor(
    table: &crate::lua::eval::Table,
    name: &str,
    slot: &mut crate::interactive::vi::Cursor,
    problems: &mut Vec<String>,
) {
    let Value::Str(text) = table.get(&Value::str(name)) else {
        return;
    };
    match crate::interactive::vi::Cursor::parse(&text) {
        Some(cursor) => *slot = cursor,
        None => problems.push(format!(
            "oslo.vi.{name}: '{text}' is not a cursor; \
             use block, line or underscore, optionally followed by blink"
        )),
    }
}

/// A boolean field, left alone when the config does not mention it.
///
/// `false` and "absent" have to be told apart, or `descriptions = false` would be indistinguishable
/// from not setting it and could never turn anything off.
fn flag(table: &crate::lua::eval::Table, name: &str, slot: &mut bool) {
    match table.get(&Value::str(name)) {
        Value::Nil => {}
        value => *slot = value.truthy(),
    }
}

/// Read a `fuzzy` knob, which takes either a boolean or a preset name.
///
/// Both spellings are accepted because both are the obvious thing to write: `fuzzy = true` is what
/// you reach for first, and `fuzzy = "loose"` is what you reach for once you want to tune it. A
/// name nothing answers to is reported rather than ignored — a typo that silently leaves fuzzy
/// matching off looks exactly like the feature not working.
fn fuzzy(
    table: &crate::lua::eval::Table,
    path: &str,
    slot: &mut Fuzzy,
    problems: &mut Vec<String>,
) {
    match table.get(&Value::str("fuzzy")) {
        Value::Nil => {}
        Value::Str(name) => match Fuzzy::parse(name.as_ref()) {
            Some(chosen) => *slot = chosen,
            None => problems.push(format!(
                "{path}: '{name}' is not a preset; use off, tight, smart or loose"
            )),
        },
        value => {
            *slot = if value.truthy() {
                Fuzzy::Smart
            } else {
                Fuzzy::Off
            }
        }
    }
}
