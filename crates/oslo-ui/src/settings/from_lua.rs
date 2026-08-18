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
use oslo_base::value::Value;

pub fn read_lua_settings(oslo: &Value) -> (Settings, Vec<String>) {
    let mut settings = Settings::default();
    let mut problems = Vec::new();
    let Value::Table(oslo) = oslo else {
        return (settings, problems);
    };
    let oslo = oslo.borrow();

    if let Value::Table(table) = oslo.get_str("dirs") {
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

    if let Value::Table(table) = oslo.get_str("notify") {
        let table = table.borrow();
        if let Some(n) = number(&table, "after") {
            settings.notify.after = n.max(0) as u64;
        }
        if let Value::Str(title) = table.get_str("title") {
            settings.notify_text.title = Some(title.to_string());
        }
        if let Value::Str(command) = table.get_str("command") {
            settings.notify_text.command = Some(command.to_string());
        }
    }

    if let Value::Table(table) = oslo.get_str("vi") {
        let table = table.borrow();
        flag(&table, "enabled", &mut settings.vi.enabled);
        let c = &mut settings.vi.cursors;
        cursor(&table, "cursor_insert", &mut c.insert, &mut problems);
        cursor(&table, "cursor_normal", &mut c.normal, &mut problems);
        cursor(&table, "cursor_replace", &mut c.replace, &mut problems);
    }

    if let Value::Table(table) = oslo.get_str("completion") {
        let table = table.borrow();
        if let Some(n) = number(&table, "max_rows") {
            // One row is still a dropdown; zero would be a dropdown that never appears, which is
            // what turning completion off looks like and is not what a row count means.
            settings.completion.max_rows = (n.max(1) as usize).min(crate::dropdown::CEILING_ROWS);
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
        // One kind filter per prompt: the two share no kind of candidate, so a single list could
        // only ever be right for one of them. See `settings::Completion::lua_sources`.
        for (field, slot) in [
            ("sh_sources", &mut settings.completion.sh_sources),
            ("lua_sources", &mut settings.completion.lua_sources),
        ] {
            let Value::Table(list) = table.get(&Value::str(field)) else {
                continue;
            };
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
            *slot = Some(kinds);
        }
        if let Value::Str(name) = table.get_str("sort") {
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

    if let Value::Table(table) = oslo.get_str("suggest") {
        let table = table.borrow();
        // Two lists, read the same way: `sources` is the shell prompt's and `lua_sources` is the
        // Lua prompt's. Separate because most of the sources answer with shell — see
        // `settings::Suggest::lua_sources`.
        for (field, slot) in [
            ("sh_sources", &mut settings.suggest.sh_sources),
            ("lua_sources", &mut settings.suggest.lua_sources),
        ] {
            let Value::Table(list) = table.get(&Value::str(field)) else {
                continue;
            };
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
                        "oslo.suggest.{field}: '{name}' is not a source; \
                         the sources are history, completion, names, path, predict and provider"
                    )),
                }
            }
            *slot = sources;
        }
        // Canonical, and checked, like every other key a config names: these are compared against
        // the editor's own name for the key that was pressed, so `"Ctrl-F"` or `"return"` was
        // accepted and then matched nothing for the rest of the session.
        for (field, slot) in [
            ("accept", &mut settings.suggest.accept),
            ("accept_word", &mut settings.suggest.accept_word),
        ] {
            if let Value::Str(key) = table.get(&Value::str(field)) {
                match crate::keys::canonical(&key) {
                    Some(name) => *slot = Some(name),
                    None => {
                        problems.push(format!("oslo.suggest.{field}: '{key}' is not a key name"))
                    }
                }
            }
        }
        if let Value::Table(list) = table.get_str("skip_history") {
            settings.suggest.skip_history = list
                .borrow()
                .sequence()
                .iter()
                .filter_map(|value| match value {
                    Value::Str(name) => Some(name.to_string()),
                    _ => None,
                })
                .collect();
        }
    }

    if let Value::Table(table) = oslo.get_str("misc") {
        let table = table.borrow();
        if let Value::Bool(on) = table.get_str("welcome") {
            settings.misc.welcome = on;
        }
        if let Value::Bool(on) = table.get_str("warnings") {
            settings.misc.warnings = on;
        }
        if let Value::Bool(on) = table.get_str("nested_ask") {
            settings.misc.nested_ask = on;
        }
        if let Value::Str(greeting) = table.get_str("greeting") {
            settings.misc.greeting = Some(greeting.to_string());
        }
        // `interrupt_escape` takes a bare count or a table, and the count is what almost everybody
        // writes — so the short form has to work rather than being a documented alias nobody uses.
        if let Some(count) = number(&table, "interrupt_escape") {
            // A negative count is nonsense rather than "off"; clamping says so without raising.
            settings.misc.interrupt_escape.after = count.max(0) as u64;
        } else if let Value::Table(escape) = table.get_str("interrupt_escape") {
            let escape = escape.borrow();
            if let Some(count) = number(&escape, "after") {
                settings.misc.interrupt_escape.after = count.max(0) as u64;
            }
            flag(
                &escape,
                "notify",
                &mut settings.misc.interrupt_escape.notify,
            );
            if let Value::Str(name) = escape.get_str("action") {
                match super::EscapeAction::from_name(&name) {
                    Some(action) => settings.misc.interrupt_escape.action = action,
                    // Named and wrong is a typo worth reporting: silently keeping `stop` would
                    // look identical to the config having been read.
                    None => problems.push(format!(
                        "oslo.misc.interrupt_escape.action: {name:?} is not an action; they are {}",
                        super::EscapeAction::NAMES.join(", ")
                    )),
                }
            }
        }
        if let Some(seconds) = number(&table, "idle_timeout") {
            settings.misc.idle_timeout = seconds.max(0) as u64;
        }
        if let Some(ms) = number(&table, "escape_delay") {
            // Clamped rather than refused: a zero would make every arrow key over ssh read as Esc,
            // and a delay longer than a second is a shell that appears to have stopped responding
            // to Esc at all.
            settings.misc.escape_delay = ms.clamp(1, 2000) as u64;
        }
        if let Value::Str(depth) = table.get_str("color_depth") {
            match crate::theme::Depth::named(&depth) {
                Some(_) => settings.misc.color_depth = Some(depth.to_string()),
                None => problems.push(format!(
                    "oslo.misc.color_depth: {depth:?} is not a depth; \
                     it is truecolor, 256, 16 or none"
                )),
            }
        }
    }

    if let Value::Table(table) = oslo.get_str("scratch") {
        let table = table.borrow();
        if let Value::Str(key) = table.get_str("key") {
            // Checked where it is written rather than where it is used, so a typo is reported
            // against the line that made it instead of leaving a tab nobody can get out of.
            if let Some(name) = crate::keys::canonical(&key) {
                settings.scratch.key = name;
            } else {
                problems.push(format!("oslo.scratch.key: '{key}' is not a key name"));
            }
        }
        if let Value::Bool(on) = table.get_str("daemon") {
            settings.scratch.daemon = on;
        }
        if let Value::Str(dir) = table.get_str("dir") {
            settings.scratch.dir = dir.to_string();
        }
        if let Some(n) = number(&table, "log_bytes")
            && n >= 0
        {
            settings.scratch.log_bytes = n as u64;
        }
    }

    if let Value::Table(table) = oslo.get_str("macros") {
        let table = table.borrow();
        if let Value::Str(key) = table.get_str("key") {
            if let Some(name) = crate::keys::canonical(&key) {
                settings.macros.key = name;
            } else {
                problems.push(format!("oslo.macros.key: '{key}' is not a key name"));
            }
        }
    }

    if let Value::Table(table) = oslo.get_str("finder") {
        let table = table.borrow();
        if let Value::Bool(on) = table.get_str("enabled") {
            settings.finder.enabled = on;
        }
        if let Value::Str(key) = table.get_str("key") {
            // Checked here rather than at bind time, so a typo is reported next to the line that
            // wrote it instead of silently leaving the finder unreachable.
            if let Some(name) = crate::keys::canonical(&key) {
                settings.finder.key = name;
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

    if let Value::Table(table) = oslo.get_str("keys") {
        for (key, action) in table.borrow().pairs() {
            // Checked here, like every other key in this file: `oslo.keys` was the one table that
            // took a name without looking at it, so a misspelled key or action was stored, bound to
            // nothing and never mentioned again — the failure mode `keys.rs` opens by promising not
            // to have.
            //
            // The canonical spelling is what gets stored, because a binding is found by comparing
            // it against the editor's own name for the key — see [`crate::keys::canonical`].
            let canon = match &key {
                Value::Str(name) => match crate::keys::canonical(name) {
                    Some(canon) => canon,
                    None => {
                        problems.push(format!("oslo.keys: '{name}' is not a key name"));
                        continue;
                    }
                },
                _ => String::new(),
            };
            match (&key, &action) {
                (Value::Str(key), Value::Str(action)) => {
                    if crate::keys::action(action).is_none() {
                        problems.push(format!("oslo.keys.{key}: '{action}' is not an action"));
                        continue;
                    }
                    settings.keys.push((canon, action.to_string()));
                }
                // A function is a binding the config wrote itself. Kept aside rather than named,
                // because a `Value` cannot live in `Settings` — the settings are plain data,
                // readable without an interpreter, and a Lua function is neither.
                (Value::Str(_), Value::Function(_)) => {
                    crate::editor::register(&canon, action.clone());
                    settings
                        .keys
                        .push((canon, crate::editor::ACTION.to_string()));
                }
                // **The table form: a binding that can say what it does.**
                //
                //     oslo.keys["alt-n"] = { run = f, desc = "the next note" }
                //
                // Without this a binding is a key and a closure, and nothing can list what a
                // session has bound — which is the gap neovim closed by putting `desc` on
                // `vim.keymap.set`, and what every which-key display is built on.
                (Value::Str(key), Value::Table(spec)) => {
                    let spec = spec.borrow();
                    let desc = match spec.get_str("desc") {
                        Value::Str(text) => Some(text.to_string()),
                        _ => None,
                    };
                    match spec.get_str("run") {
                        run @ Value::Function(_) => {
                            crate::editor::register(&canon, run);
                            settings
                                .keys
                                .push((canon.clone(), crate::editor::ACTION.to_string()));
                        }
                        Value::Str(named) => {
                            if crate::keys::action(&named).is_none() {
                                problems
                                    .push(format!("oslo.keys.{key}: '{named}' is not an action"));
                                continue;
                            }
                            settings.keys.push((canon.clone(), named.to_string()));
                        }
                        _ => {
                            problems.push(format!(
                                "oslo.keys.{key}: `run` must be a function or an action name"
                            ));
                            continue;
                        }
                    }
                    if let Some(desc) = desc {
                        settings.key_descriptions.push((canon, desc));
                    }
                }
                _ => problems.push(
                    "oslo.keys: an entry is a key name mapped to an action name, a function, \
                     or a table with `run` and `desc`"
                        .to_string(),
                ),
            }
        }
        // Table iteration has no order, and a binding that depends on which of two entries was
        // applied last is one that behaves differently between runs.
        settings.keys.sort();
        settings.key_descriptions.sort();
    }

    if let Value::Table(table) = oslo.get_str("abbr") {
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
                    let expansion = match spec.get_str("expansion") {
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
                    let anywhere = matches!(spec.get_str("anywhere"), Value::Bool(true));
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

    // `oslo.builtin.rm`. Nested one deeper than the rest because `builtin` is a namespace per
    // builtin, not a group of settings — the next one to want a knob adds a sibling table.
    if let Value::Table(table) = oslo.get_str("builtin")
        && let Value::Table(rm) = table.borrow().get_str("rm")
    {
        let rm = rm.borrow();
        flag(&rm, "to_tmp", &mut settings.builtin.rm.to_tmp);
        if let Some(mb) = number(&rm, "max_to_tmp") {
            settings.builtin.rm.max_to_tmp = mb.max(0) as u64;
        }
        match rm.get_str("trash") {
            Value::Str(dir) if !dir.trim().is_empty() => {
                settings.builtin.rm.trash = dir.to_string();
            }
            Value::Nil => {}
            // Reported rather than ignored: a trash directory that silently stayed `/tmp` would
            // send files somewhere the config plainly said it did not want them.
            _ => problems.push("oslo.builtin.rm.trash: must be a directory path".to_string()),
        }
    }

    if let Value::Table(table) = oslo.get_str("builtin")
        && let Value::Table(nav) = table.borrow().get_str("nav")
    {
        let nav = nav.borrow();
        let settings = &mut settings.builtin.nav;
        flag(&nav, "fullscreen", &mut settings.fullscreen);
        flag(&nav, "legend", &mut settings.legend);
        flag(&nav, "hidden", &mut settings.hidden);
        flag(&nav, "reverse", &mut settings.reverse);
        flag(&nav, "scanner", &mut settings.scanner);
        if let Some(n) = number(&nav, "height") {
            settings.height = n.max(0) as usize;
        }
        if let Some(n) = number(&nav, "width") {
            settings.width = n.max(0) as usize;
        }
        if let Some(n) = number(&nav, "legend_gap") {
            settings.legend_gap = n.max(0) as usize;
        }
        if let Some(n) = number(&nav, "padding_x") {
            settings.padding_x = n.max(0) as usize;
        }
        if let Some(n) = number(&nav, "padding_y") {
            settings.padding_y = n.max(0) as usize;
        }
        if let Value::Table(walk) = nav.get_str("type_nav") {
            let walk = walk.borrow();
            flag(&walk, "enabled", &mut settings.type_nav.enabled);
            if let Some(ms) = number(&walk, "settle_ms") {
                settings.type_nav.settle = std::time::Duration::from_millis(ms.max(0) as u64);
            }
        }
        if let Value::Table(icons) = nav.get_str("icons") {
            let icons = icons.borrow();
            if let Value::Str(mark) = icons.get_str("dir") {
                settings.icons.directory = mark.to_string();
            }
            if let Value::Str(mark) = icons.get_str("file") {
                settings.icons.file = mark.to_string();
            }
            if let Value::Table(by_extension) = icons.get_str("ext") {
                for (key, value) in by_extension.borrow().pairs() {
                    match (&key, &value) {
                        // Lowercased once here rather than at every row: an extension is matched
                        // case-insensitively, and `README.MD` should read like `readme.md`.
                        (Value::Str(extension), Value::Str(mark)) => settings
                            .icons
                            .by_extension
                            .push((extension.to_ascii_lowercase(), mark.to_string())),
                        _ => problems.push(
                            "oslo.builtin.nav.icons.ext: every entry must be an extension \
                             mapped to what to draw"
                                .to_string(),
                        ),
                    }
                }
            }
        }
        if let Value::Str(name) = nav.get_str("position") {
            settings.position = match name.as_ref() {
                "top" | "start" => crate::ask::chrome::Place::Start,
                "center" | "centre" | "middle" => crate::ask::chrome::Place::Center,
                "bottom" | "end" => crate::ask::chrome::Place::End,
                other => {
                    problems.push(format!(
                        "oslo.builtin.nav.position: '{other}' is not a position; use top, center or bottom"
                    ));
                    settings.position
                }
            };
        }
        if let Value::Str(name) = nav.get_str("border") {
            match crate::ask::Border::parse(&name) {
                Some(border) => settings.border = border,
                None => problems.push(format!(
                    "oslo.builtin.nav.border: '{name}' is not a border; use none, rounded, square, double or thick"
                )),
            }
        }
        if let Value::Str(name) = nav.get_str("border_fit") {
            match crate::ask::chrome::Fit::parse(&name) {
                Some(fit) => settings.border_fit = fit,
                None => problems.push(format!(
                    "oslo.builtin.nav.border_fit: '{name}' is not a fit; use content or full"
                )),
            }
        }
        if let Value::Str(name) = nav.get_str("border_fg") {
            match crate::theme::Color::parse(&name) {
                Some(colour) => settings.border_fg = Some(colour),
                None => problems.push(format!(
                    "oslo.builtin.nav.border_fg: '{name}' is not a colour"
                )),
            }
        }
        if let Value::Str(name) = nav.get_str("filter_at") {
            match crate::ask::Where::parse(&name) {
                Some(place) => settings.filter_at = place,
                None => problems.push(format!(
                    "oslo.builtin.nav.filter_at: '{name}' is not a placement; use top or bottom"
                )),
            }
        }
    }

    if let Value::Table(table) = oslo.get_str("history") {
        let table = table.borrow();
        if let Some(n) = number(&table, "size") {
            settings.history.size = Some(n.max(0) as usize);
        }
        if let Value::Str(file) = table.get_str("file") {
            settings.history.file = Some(file.to_string());
        }
        flag(&table, "ignore_space", &mut settings.history.ignore_space);
        flag(&table, "ignore_dups", &mut settings.history.ignore_dups);
        if let Value::Table(list) = table.get_str("ignore") {
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

mod read;
use read::{cursor, flag, fuzzy, number};
