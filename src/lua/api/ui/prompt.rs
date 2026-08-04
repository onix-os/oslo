//! `oslo.ui.input`, `.confirm`, `.choose`, `.filter`, `.style` — the raw-mode widgets, from Lua.
//!
//! The same code the `ui` builtin runs, so a prompt looks and behaves identically whether the
//! script asking is shell or Lua. [`crate::interactive::ask`] is where they live; this is the
//! binding.
//!
//! # These are not [`super::ask`]
//!
//! That module asks with ordinary `read`-a-line prompts: it works down a pipe, over a serial
//! console, and inside a script whose output is being logged, and what you were asked stays in the
//! transcript. It is still the right thing for a config file or a non-interactive script.
//!
//! These take the terminal for a moment. They need one — with no tty they answer `nil` rather than
//! blocking — and they are what you want when a person is definitely there and the question
//! deserves arrow keys.
//!
//! # Every argument is a table field
//!
//! `oslo.ui.input{placeholder = "name", password = true}` rather than five positional arguments
//! nobody can order from memory. A widget has too many knobs for positions to stay readable, and a
//! table is also how the caller writes only the two they care about.

use super::super::util::{ok, put};
use crate::interactive::ask::{
    Align, Answer, As, Border, Browse, Choice, Confirm, Entry, Input, Level, Pager, Spin, Styling,
    Table as Rows, Want, Write, choose, confirm, file, filter, format, horizontal, input, line,
    pager, parse_table, spin, style, table, vertical, write,
};
use crate::interactive::theme;
use crate::lua::eval::value::{Table, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// A string field, or the empty string.
fn field(table: &Table, name: &str) -> String {
    match table.get(&Value::str(name)) {
        Value::Str(s) => s.to_string(),
        _ => String::new(),
    }
}

/// An optional string field.
fn maybe(table: &Table, name: &str) -> Option<String> {
    match table.get(&Value::str(name)) {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

fn flag(table: &Table, name: &str) -> bool {
    matches!(table.get(&Value::str(name)), Value::Bool(true))
}

/// A count that must be at least one — a height, a number of rows.
fn count(table: &Table, name: &str, fallback: usize) -> usize {
    match table.get(&Value::str(name)) {
        Value::Number(n) => n.as_int().map(|i| i.max(1) as usize).unwrap_or(fallback),
        _ => fallback,
    }
}

/// A count that may be zero — a padding, a width.
///
/// Separate from [`count`] because clamping these to one is a bug you cannot see until you compare
/// the two languages: `oslo.ui.style{padding_x = 0}` drew a column of padding where the shell's
/// `ui style --padding "0 0"` drew none. Same widget, same theme, different box.
fn size(table: &Table, name: &str, fallback: usize) -> usize {
    match table.get(&Value::str(name)) {
        Value::Number(n) => n.as_int().map(|i| i.max(0) as usize).unwrap_or(fallback),
        _ => fallback,
    }
}

/// The argument as a table, so `oslo.ui.input()` and `oslo.ui.input{…}` both work.
///
/// An empty table stands in for a missing argument, which is what makes every field optional
/// without a branch at each one.
fn spec(args: &[Value]) -> Rc<RefCell<Table>> {
    match args.first() {
        Some(Value::Table(t)) => Rc::clone(t),
        _ => Rc::new(RefCell::new(Table::new())),
    }
}

/// A list field: `{items = {"a", "b"}}`.
fn items(table: &Table, name: &str) -> Vec<String> {
    let Value::Table(list) = table.get(&Value::str(name)) else {
        return Vec::new();
    };
    let list = list.borrow();
    let mut out = Vec::new();
    let mut index = 1i64;
    while let Value::Str(s) = list.get(&Value::int(index)) {
        out.push(s.to_string());
        index += 1;
    }
    out
}

pub fn install(ui: &mut Table) {
    // oslo.ui.input{prompt=, placeholder=, default=, password=, required=} -> string or nil
    put(ui, "input", |_, args| {
        let t = spec(&args);
        let t = t.borrow();
        let answer = input(&Input {
            prompt: field(&t, "prompt"),
            placeholder: field(&t, "placeholder"),
            default: maybe(&t, "default").or_else(|| maybe(&t, "value")),
            password: flag(&t, "password"),
            required: flag(&t, "required"),
        });
        // `nil` for both cancelled and no-terminal. Lua has one absent value and a script says
        // `if not answer then return end`; splitting the two would mean every caller checking a
        // second thing they do not care about.
        ok(match answer {
            Answer::Given(line) => Value::str(&line),
            _ => Value::Nil,
        })
    });

    // oslo.ui.confirm("question" | {question=, yes=, no=, default=}) -> boolean or nil
    put(ui, "confirm", |_, args| {
        let mut settings = Confirm::default();
        match args.first() {
            Some(Value::Str(question)) => settings.question = question.to_string(),
            Some(Value::Table(_)) => {
                let t = spec(&args);
                let t = t.borrow();
                if let Some(q) = maybe(&t, "question") {
                    settings.question = q;
                }
                if let Some(y) = maybe(&t, "yes") {
                    settings.yes = y;
                }
                if let Some(n) = maybe(&t, "no") {
                    settings.no = n;
                }
                settings.default = flag(&t, "default");
            }
            _ => {}
        }
        ok(match confirm(&settings) {
            Answer::Given(yes) => Value::Bool(yes),
            _ => Value::Nil,
        })
    });

    // oslo.ui.choose{items=, header=, multi=, height=} -> string, list, or nil
    put(ui, "choose", |_, args| ok(list_widget(&args, false)));
    // The same, narrowed as you type.
    put(ui, "filter", |_, args| ok(list_widget(&args, true)));

    // oslo.ui.write{header=, placeholder=, default=} -> string or nil
    put(ui, "write", |_, args| {
        let t = spec(&args);
        let t = t.borrow();
        ok(
            match write(&Write {
                header: field(&t, "header"),
                placeholder: field(&t, "placeholder"),
                default: maybe(&t, "default").or_else(|| maybe(&t, "value")),
            }) {
                Answer::Given(text) => Value::str(&text),
                _ => Value::Nil,
            },
        )
    });

    // oslo.ui.file{start=, directories=, hidden=, height=} -> path or nil
    put(ui, "file", |_, args| {
        let t = spec(&args);
        let t = t.borrow();
        let want = if flag(&t, "directories") {
            Want::Directories
        } else if flag(&t, "both") {
            Want::Both
        } else {
            Want::Files
        };
        ok(
            match file(&Browse {
                start: maybe(&t, "start")
                    .or_else(|| maybe(&t, "path"))
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::path::PathBuf::from(".")),
                want,
                hidden: flag(&t, "hidden"),
                height: count(&t, "height", 12),
                fuzzy: crate::interactive::settings::current().completion.fuzzy,
            }) {
                Answer::Given(path) => Value::str(&path),
                _ => Value::Nil,
            },
        )
    });

    // oslo.ui.table{rows=, headers=, separator=, height=} -> the chosen row, or nil
    put(ui, "table", |_, args| {
        let t = spec(&args);
        let t = t.borrow();
        let separator = maybe(&t, "separator")
            .and_then(|s| s.chars().next())
            .unwrap_or(',');
        let text = items(&t, "rows").join("\n");
        let (rows, raw) = parse_table(&text, separator);
        ok(
            match table(&Rows {
                headers: items(&t, "headers"),
                rows,
                raw,
                height: count(&t, "height", 10),
                filter: !flag(&t, "no_filter"),
                fuzzy: crate::interactive::settings::current().completion.fuzzy,
            }) {
                Answer::Given(row) => Value::str(&row),
                _ => Value::Nil,
            },
        )
    });

    // oslo.ui.pager{text=, title=, wrap=} -> true when it was shown
    put(ui, "pager", |_, args| {
        let t = spec(&args);
        let t = t.borrow();
        ok(Value::Bool(matches!(
            pager(&Pager {
                title: field(&t, "title"),
                text: field(&t, "text"),
                wrap: flag(&t, "wrap"),
            }),
            Answer::Given(())
        )))
    });

    // oslo.ui.spin{title=, command={"sleep","1"}, quiet=} -> the command's exit status
    put(ui, "spin", |_, args| {
        let t = spec(&args);
        let t = t.borrow();
        ok(Value::int(spin(&Spin {
            title: maybe(&t, "title").unwrap_or_else(|| "working".to_string()),
            command: items(&t, "command"),
            quiet: flag(&t, "quiet"),
        }) as i64))
    });

    // oslo.ui.log{message=, level=, time=, fields=} -> nothing; writes to stderr
    put(ui, "log", |_, args| {
        let mut level = Level::Info;
        let mut message = String::new();
        let mut time = None;
        let mut fields = Vec::new();
        match args.first() {
            Some(Value::Str(text)) => message = text.to_string(),
            Some(Value::Table(_)) => {
                let t = spec(&args);
                let t = t.borrow();
                message = field(&t, "message");
                if let Some(name) = maybe(&t, "level") {
                    match Level::parse(&name) {
                        Some(parsed) => level = parsed,
                        None => {
                            return crate::lua::api::util::failed(
                                "oslo.ui.log",
                                format!("{name}: not a level"),
                            );
                        }
                    }
                }
                time = maybe(&t, "time");
                if let Value::Table(pairs) = t.get(&Value::str("fields")) {
                    for (key, value) in pairs.borrow().pairs() {
                        if let (Value::Str(k), Value::Str(v)) = (&key, &value) {
                            fields.push((k.to_string(), v.to_string()));
                        }
                    }
                    // Table iteration has no order, and a log line whose fields moved between runs
                    // is one nobody can diff.
                    fields.sort();
                }
            }
            _ => {}
        }
        eprintln!(
            "{}",
            line(&Entry {
                level,
                message,
                time,
                fields
            })
        );
        // `fatal` is `error` plus stopping, which is the only thing that distinguishes the two —
        // printing it and carrying on would make the level a lie in one of the two languages.
        //
        // A Lua error rather than an exit: it unwinds the chunk, which is what `fatal` means
        // *inside* a script, and it leaves a caller in `pcall` able to decide otherwise. The
        // shell-side `ui log --level fatal` exits non-zero instead, because there is no chunk
        // there to unwind.
        if level == Level::Fatal {
            return Err(crate::lua::eval::LuaError::new("ui.log: fatal"));
        }
        ok(Value::Nil)
    });

    // oslo.ui.format(text | {text=, type=, fields=}) -> string
    put(ui, "format", |_, args| {
        let mut kind = As::Markdown;
        let mut text = String::new();
        let mut values = Vec::new();
        match args.first() {
            Some(Value::Str(body)) => text = body.to_string(),
            Some(Value::Table(_)) => {
                let t = spec(&args);
                let t = t.borrow();
                text = field(&t, "text");
                if let Some(name) = maybe(&t, "type") {
                    match As::parse(&name) {
                        Some(parsed) => kind = parsed,
                        None => {
                            return crate::lua::api::util::failed(
                                "oslo.ui.format",
                                format!("{name}: not a type"),
                            );
                        }
                    }
                }
                if let Value::Table(pairs) = t.get(&Value::str("fields")) {
                    for (key, value) in pairs.borrow().pairs() {
                        if let (Value::Str(k), Value::Str(v)) = (&key, &value) {
                            values.push((k.to_string(), v.to_string()));
                        }
                    }
                    // Sorted for the same reason `log`'s fields are: table iteration has no order,
                    // and `template` applies replacements in sequence — so overlapping keys would
                    // render differently between two runs of the same script.
                    values.sort();
                }
            }
            _ => {}
        }
        ok(Value::str(format(&text, kind, &values).as_str()))
    });

    // oslo.ui.join{blocks=, vertical=, align=} -> string
    put(ui, "join", |_, args| {
        let t = spec(&args);
        let t = t.borrow();
        let mut blocks = items(&t, "blocks");
        if blocks.is_empty() {
            let mut index = 1i64;
            while let Value::Str(s) = t.get(&Value::int(index)) {
                blocks.push(s.to_string());
                index += 1;
            }
        }
        let align = match maybe(&t, "align") {
            Some(name) => match Align::parse(&name) {
                Some(parsed) => parsed,
                None => {
                    return crate::lua::api::util::failed(
                        "oslo.ui.join",
                        format!("{name}: not an alignment"),
                    );
                }
            },
            None => Align::Start,
        };
        ok(Value::str(
            if flag(&t, "vertical") {
                vertical(&blocks, align)
            } else {
                horizontal(&blocks, align)
            }
            .as_str(),
        ))
    });

    // oslo.ui.style(text | {text=, border=, fg=, bg=, bold=, padding_x=, padding_y=, width=})
    put(ui, "style", |_, args| {
        let mut settings = Styling::default();
        match args.first() {
            Some(Value::Str(text)) => settings.text = text.to_string(),
            Some(Value::Table(_)) => {
                let t = spec(&args);
                let t = t.borrow();
                settings.text = field(&t, "text");
                if let Some(name) = maybe(&t, "border") {
                    match Border::parse(&name) {
                        Some(border) => settings.border = border,
                        None => {
                            return crate::lua::api::util::failed(
                                "oslo.ui.style",
                                format!("{name}: not a border"),
                            );
                        }
                    }
                }
                settings.style.fg = maybe(&t, "fg").and_then(|c| theme::Color::parse(&c));
                settings.style.bg = maybe(&t, "bg").and_then(|c| theme::Color::parse(&c));
                if let Some(c) = maybe(&t, "border_fg").and_then(|c| theme::Color::parse(&c)) {
                    settings.border_style.fg = Some(c);
                }
                settings.style.bold = flag(&t, "bold");
                settings.padding_x = size(&t, "padding_x", 0);
                settings.padding_y = size(&t, "padding_y", 0);
                settings.width = match t.get(&Value::str("width")) {
                    Value::Number(n) => n.as_int().map(|i| i.max(0) as usize),
                    _ => None,
                };
            }
            _ => {}
        }
        // Returned rather than printed: a caller may want to put it in a variable, and
        // `print(oslo.ui.style{…})` is the other half of that in one more character.
        ok(Value::str(style(&settings).as_str()))
    });
}

/// `choose` and `filter`, which differ only in whether typing narrows the list.
fn list_widget(args: &[Value], filtering: bool) -> Value {
    let t = spec(args);
    let t = t.borrow();
    let mut chosen = items(&t, "items");
    if chosen.is_empty() {
        // `oslo.ui.choose{"a", "b"}` — the list itself as the argument, which is what a Lua
        // caller writes first and is worth accepting.
        chosen = positional(&t);
    }
    let multi = flag(&t, "multi");
    let settings = Choice {
        header: field(&t, "header"),
        items: chosen,
        multi,
        filter: filtering,
        height: count(&t, "height", 10),
        fuzzy: crate::interactive::settings::current().completion.fuzzy,
    };
    let answer = if filtering {
        filter(&settings)
    } else {
        choose(&settings)
    };
    match answer {
        // A single pick answers with the string; `multi` answers with a list, even of one. A
        // caller that asked for many is written to loop, and handing it a bare string on the day
        // one thing was checked would break that.
        Answer::Given(picked) if multi => {
            crate::lua::api::util::list(picked.iter().map(Value::str).collect::<Vec<_>>())
        }
        Answer::Given(picked) => picked.first().map(Value::str).unwrap_or(Value::Nil),
        _ => Value::Nil,
    }
}

/// The positional entries of a table: `{"a", "b", "c"}`.
///
/// Every widget that takes a list accepts one this way as well as under its named field, because
/// `oslo.ui.choose{"a", "b"}` is what a Lua caller writes before reading any documentation.
fn positional(table: &Table) -> Vec<String> {
    let mut out = Vec::new();
    let mut index = 1i64;
    while let Value::Str(s) = table.get(&Value::int(index)) {
        out.push(s.to_string());
        index += 1;
    }
    out
}
