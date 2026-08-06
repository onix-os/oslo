//! `oslo.ui.block` — a headline and a rail of labelled rows, for a config or a plugin.
//!
//! ```lua
//! local b = oslo.ui.block("direnv loaded")
//! b:row("PATH", "/nix/store/…:/home/…/target/debug", { overflow = "ellipsis" })
//! b:row("aliases", "_b _c _r _t _v")
//! b:done()
//! ```
//!
//! **The same drawing the shell uses for its own reports.** [`crate::interactive::block`] is the
//! implementation, and `startup::environments::report` is a caller exactly as this is — a Lua API
//! that reimplemented the built-in renderer would drift from it, and a config that drew a block
//! looking almost-but-not-quite like the shell's is worse than one that could not draw one at all.
//!
//! The handle is a table of closures over one shared `Block`, which is what makes `b:row(…)` chain
//! and `b:done()` emit once. A method call in Lua passes the table as the first argument, so every
//! closure here ignores its first and reads from the capture.

use super::super::util::{ok, put, text};
use crate::interactive::block::{Block, Overflow};
use crate::interactive::theme::Style;
use crate::lua::eval::LuaError;
use crate::lua::eval::value::{Table, Value};
use std::cell::RefCell;
use std::io::IsTerminal;
use std::rc::Rc;

pub fn install(ui: &mut Table) {
    // oslo.ui.block(head) -> a handle you add rows to and then finish
    put(ui, "block", |_, args| {
        let head = match args.first() {
            None | Some(Value::Nil) => String::new(),
            _ => text(&args, 1, "oslo.ui.block")?,
        };
        // Undecorated when nobody is looking: a rail glyph and an escape sequence in a pipe are
        // not decoration, they are damage to whatever is reading.
        let mut block = Block::new(head);
        if !std::io::stdout().is_terminal() {
            block = block.plain();
        }
        ok(handle(Rc::new(RefCell::new(block))))
    });
}

/// The methods, each closing over the one block they build.
fn handle(block: Rc<RefCell<Block>>) -> Value {
    let mut methods = Table::new();

    let for_row = Rc::clone(&block);
    put(&mut methods, "row", move |_, args| {
        // `b:row(label, text)` — Lua hands the table itself as argument one, so the label is two.
        let label = text(&args, 2, "block:row")?;
        let body = match args.get(2) {
            None | Some(Value::Nil) => String::new(),
            _ => text(&args, 3, "block:row")?,
        };
        let overflow = overflow_of(args.get(3))?;
        let style = style_of(args.get(3));
        let mut block = for_row.borrow_mut();
        block.styled_row(label, Style::default(), body, style);
        if let Some(how) = overflow {
            block.overflow(how);
        }
        ok(Value::Bool(true))
    });

    let for_note = Rc::clone(&block);
    put(&mut methods, "note", move |_, args| {
        let body = text(&args, 2, "block:note")?;
        let overflow = overflow_of(args.get(2))?;
        let mut block = for_note.borrow_mut();
        block.note(body);
        if let Some(how) = overflow {
            block.overflow(how);
        }
        ok(Value::Bool(true))
    });

    let for_lines = Rc::clone(&block);
    put(&mut methods, "lines", move |_, _| {
        // The rendered rows, for a caller that wants to put them somewhere other than the screen.
        let mut list = Table::new();
        for (i, line) in for_lines.borrow().lines().into_iter().enumerate() {
            list.set(Value::int(i as i64 + 1), Value::str(&line));
        }
        ok(Value::table(list))
    });

    let for_done = Rc::clone(&block);
    put(&mut methods, "done", move |_, _| {
        // **One write, not one per row.** A block assembled across several statements must not
        // interleave with anything else printing, and a half-drawn block is worse than a late one.
        let lines = for_done.borrow().lines();
        if !lines.is_empty() {
            println!("{}", lines.join("\n"));
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
        ok(Value::Bool(true))
    });

    Value::table(methods)
}

/// The `overflow = "…"` field, refused rather than defaulted when it is not a policy.
///
/// A typo picking `count` silently is how a config ends up cutting a value it meant to wrap, with
/// nothing to say why.
fn overflow_of(options: Option<&Value>) -> Result<Option<Overflow>, LuaError> {
    let Some(Value::Table(table)) = options else {
        return Ok(None);
    };
    let named = table.borrow().get(&Value::str("overflow"));
    match named {
        Value::Nil => Ok(None),
        Value::Str(name) => Overflow::named(name.as_ref()).map(Some).ok_or_else(|| {
            LuaError::new(format!(
                "block: overflow must be \"count\", \"ellipsis\" or \"wrap\", got {name:?}"
            ))
        }),
        other => Err(LuaError::new(format!(
            "block: overflow must be a string, got {}",
            other.type_name()
        ))),
    }
}

/// The `style = …` field, as `oslo.ui.style` produced it.
///
/// Absent is the default style rather than an error: most rows do not want one, and a caller who
/// passed something unusable gets an unstyled row instead of a failed block.
fn style_of(options: Option<&Value>) -> Style {
    let Some(Value::Table(table)) = options else {
        return Style::default();
    };
    match table.borrow().get(&Value::str("style")) {
        // The same vocabulary `oslo.ui.style` and the prompt take: a theme slot name like
        // `prompt.git`, or a colour like `cyan` or `#8be9fd`. One parser, so a plugin's rows and
        // the prompt agree about what a name means.
        Value::Str(spec) => super::super::prompt::style_named(spec.as_ref()),
        _ => Style::default(),
    }
}
