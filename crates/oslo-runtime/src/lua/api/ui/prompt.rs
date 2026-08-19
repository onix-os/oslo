//! `oslo.ui.input`, `.confirm`, `.choose`, `.filter`, `.style` — the raw-mode widgets, from Lua.
//!
//! The same code the `ui` builtin runs, so a prompt looks and behaves identically whether the
//! script asking is shell or Lua. [`oslo_ui::ask`] is where they live; this is the
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
use oslo_base::value::{Table, Value};
use oslo_ui::ask::chrome::{Chrome, Fit, Place};
use oslo_ui::ask::{
    Align, Answer, As, Border, Browse, Choice, Confirm, Entry, Input, Level, Pager, Spin, Styling,
    Table as Rows, Want, Write, choose, confirm, file, filter, format, horizontal, input, line,
    pager, parse_table, spin, style, table, vertical, write,
};
use oslo_ui::theme;
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
pub(super) fn maybe(table: &Table, name: &str) -> Option<String> {
    match table.get(&Value::str(name)) {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

pub(super) fn flag(table: &Table, name: &str) -> bool {
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
pub(super) fn size(table: &Table, name: &str, fallback: usize) -> usize {
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

/// The named entries of a table, as `(name, text)` pairs.
///
/// **Numbers and booleans count.** This used to take only `Value::Str`, so
/// `oslo.ui.log{fields = {count = 12}}` dropped the field on the floor — and a count, a status or a
/// duration is exactly what anyone logs first. A value that cannot be a word at all (a table, a
/// function) is still skipped, because `fields = {x = {}}` is a mistake rather than a value.
fn named_values(table: &Table) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (key, value) in table.pairs() {
        let Value::Str(name) = &key else { continue };
        let text = match &value {
            Value::Str(s) => s.to_string(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => continue,
        };
        out.push((name.to_string(), text));
    }
    out
}

/// The chrome fields every widget accepts: `legend`, `border`, `fit`, `fullscreen`, `align_x`,
/// `align_y`.
///
/// Read from the same options table the widget's own fields come from, because they *are* the
/// widget's fields as far as a caller is concerned — nobody wants to pass two tables to ask a
/// question. A name that is not a placement is refused rather than defaulted: `align_x = "centre"`
/// works, `align_x = "centred"` says so.
fn chrome_of(t: &Table) -> Result<Chrome, oslo_base::value::LuaError> {
    let mut chrome = Chrome::default();
    // Absent leaves the default on, so `legend = false` is the only spelling that turns it off and
    // `legend = nil` cannot mean "off" by accident.
    if let Value::Bool(shown) = t.get_str("legend") {
        chrome.legend = shown;
    }
    chrome.fullscreen = flag(t, "fullscreen") || flag(t, "alt");
    // Absent keeps the default rather than zeroing it: a caller who set only `border` still wants
    // the cell of padding that makes a box readable. `size` clamps at zero rather than one, which
    // is the whole reason it exists — `padding_x = 0` must mean none.
    chrome.padding_x = size(t, "padding_x", chrome.padding_x);
    chrome.padding_y = size(t, "padding_y", chrome.padding_y);
    chrome.legend_gap = size(t, "legend_gap", chrome.legend_gap);
    if let Some(name) = maybe(t, "border") {
        chrome.border = Border::parse(&name)
            .ok_or_else(|| oslo_base::value::LuaError::new(format!("{name}: not a border")))?;
    }
    if let Some(colour) = maybe(t, "border_fg") {
        chrome.border_style =
            theme::Style::fg(theme::Color::parse(&colour).ok_or_else(|| {
                oslo_base::value::LuaError::new(format!("{colour}: not a colour"))
            })?);
    }
    for (field, slot) in [("fit", 0), ("border_fit", 0)] {
        let _ = slot;
        if let Some(name) = maybe(t, field) {
            chrome.fit = Fit::parse(&name).ok_or_else(|| {
                oslo_base::value::LuaError::new(format!("{name}: fit is \"content\" or \"full\""))
            })?;
        }
    }
    for (field, axis) in [("align_x", true), ("align_y", false)] {
        if let Some(name) = maybe(t, field) {
            let place = Place::parse(&name)
                .ok_or_else(|| oslo_base::value::LuaError::new(format!("{name}: not a {field}")))?;
            if axis {
                chrome.align_x = place;
            } else {
                chrome.align_y = place;
            }
        }
    }
    // `align = "center"` sets both, which is what anyone centring a full-screen widget means.
    if let Some(name) = maybe(t, "align") {
        let place = Place::parse(&name)
            .ok_or_else(|| oslo_base::value::LuaError::new(format!("{name}: not an alignment")))?;
        chrome.align_x = place;
        chrome.align_y = place;
    }
    Ok(chrome)
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
            chrome: chrome_of(&t)?,
            look: super::look::look_of(&t)?,
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
                // **Parsed before anything is asked.** A bad `align` must be a diagnostic, not a
                // question answered and then thrown away — and this is the one widget that would
                // otherwise ask first, because it falls back to a line without a terminal.
                settings.chrome = chrome_of(&t)?;
            }
            _ => {}
        }
        // **Without a terminal, ask on a line rather than answering nothing.** The raw-mode widget
        // needs a tty it can take; `super::ask` needs only stdin, works down a pipe and over a
        // serial console, and leaves the question in the transcript. One name, both behaviours —
        // and it is what `ui/mod.rs` says the pair is for.
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            return ok(Value::Bool(super::ask::on_a_line(
                &settings.question,
                settings.default,
            )));
        }
        ok(match confirm(&settings) {
            Answer::Given(yes) => Value::Bool(yes),
            _ => Value::Nil,
        })
    });

    // oslo.ui.choose{items=, header=, multi=, height=} -> string, list, or nil
    put(ui, "choose", |_, args| {
        list_widget(&args, false).map(|v| vec![v])
    });
    // The same, narrowed as you type.
    put(ui, "filter", |_, args| {
        list_widget(&args, true).map(|v| vec![v])
    });

    // oslo.ui.write{header=, placeholder=, default=} -> string or nil
    put(ui, "write", |_, args| {
        let t = spec(&args);
        let t = t.borrow();
        ok(
            match write(&Write {
                header: field(&t, "header"),
                placeholder: field(&t, "placeholder"),
                default: maybe(&t, "default").or_else(|| maybe(&t, "value")),
                chrome: chrome_of(&t)?,
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
                fuzzy: oslo_ui::settings::current().completion.fuzzy,
                chrome: chrome_of(&t)?,
                look: super::look::look_of(&t)?,
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
                fuzzy: oslo_ui::settings::current().completion.fuzzy,
                chrome: chrome_of(&t)?,
                look: super::look::look_of(&t)?,
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
                chrome: chrome_of(&t)?,
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
                if let Value::Table(pairs) = t.get_str("fields") {
                    fields.extend(named_values(&pairs.borrow()));
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
            return Err(oslo_base::value::LuaError::new("ui.log: fatal"));
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
                if let Value::Table(pairs) = t.get_str("fields") {
                    values.extend(named_values(&pairs.borrow()));
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

    // oslo.ui.style(text | {text=, …}) or oslo.ui.style(text, {…})
    //
    // **Both call shapes, and the second one is a bug fix.** `oslo.ui.style("hi", {fg="green"})`
    // used to take the string, drop the spec on the floor and hand back unpainted text — every
    // option silently ignored. It is the shape anyone writes first, and it is the shape the other
    // `oslo.ui.style` (in `api::prompt`, which this one shadows) accepted.
    put(ui, "style", |_, args| {
        let mut settings = Styling::default();
        // The spec is argument two when the text came first, and argument one otherwise.
        let leading_text = matches!(args.first(), Some(Value::Str(_)));
        let args: Vec<Value> = match (leading_text, args.get(1)) {
            (true, Some(Value::Table(spec))) => {
                // The caller's spec, with the text folded in, so everything below reads one table
                // whichever shape was written.
                let mut merged = Table::new();
                for (key, value) in spec.borrow().pairs() {
                    merged.set(key, value);
                }
                merged.set_str("text", args[0].clone());
                vec![Value::table(merged)]
            }
            _ => args,
        };
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
                settings.width = match t.get_str("width") {
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
///
/// The two parsers are asked with `?` rather than `unwrap_or_default`: a misspelt colour or a
/// preset that does not exist has to be an error a script can see. Defaulting quietly is how
/// `look = "histry"` would draw the plain list and leave nothing at all to explain why.
fn list_widget(args: &[Value], filtering: bool) -> Result<Value, oslo_base::value::LuaError> {
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
        fuzzy: oslo_ui::settings::current().completion.fuzzy,
        chrome: chrome_of(&t)?,
        look: super::look::look_of(&t)?,
        // `ui choose` and `ui filter` pick from what they were given; offering to invent a row
        // would be answering a question the caller did not ask.
        create: None,
    };
    let answer = if filtering {
        filter(&settings)
    } else {
        choose(&settings)
    };
    Ok(match answer {
        // A single pick answers with the string; `multi` answers with a list, even of one. A
        // caller that asked for many is written to loop, and handing it a bare string on the day
        // one thing was checked would break that.
        Answer::Given(picked) if multi => {
            crate::lua::api::util::list(picked.iter().map(Value::str).collect::<Vec<_>>())
        }
        Answer::Given(picked) => picked.first().map(Value::str).unwrap_or(Value::Nil),
        _ => Value::Nil,
    })
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
