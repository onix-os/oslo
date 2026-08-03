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
    Answer, Border, Choice, Confirm, Input, Styling, choose, confirm, filter, input, style,
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

fn count(table: &Table, name: &str, fallback: usize) -> usize {
    match table.get(&Value::str(name)) {
        Value::Number(n) => n.as_int().map(|i| i.max(1) as usize).unwrap_or(fallback),
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
                settings.padding_x = count(&t, "padding_x", 0);
                settings.padding_y = count(&t, "padding_y", 0);
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
        chosen = items(&t, "");
        if chosen.is_empty() {
            let mut index = 1i64;
            while let Value::Str(s) = t.get(&Value::int(index)) {
                chosen.push(s.to_string());
                index += 1;
            }
        }
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
