//! `oslo.prompt`, and the helpers a prompt function needs.
//!
//! A prompt is a Lua function, not a format string. `PS1`'s escapes are a small language that has
//! to grow a new letter for every new thing anyone wants in a prompt; a function needs nothing
//! added to it to call `oslo.git.branch()` or to count something itself.
//!
//! ```lua
//! oslo.prompt.left = function()
//!   return oslo.style(oslo.path.shorten(oslo.fs.cwd()), "blue") .. " ❯ "
//! end
//! oslo.prompt.right = function() return oslo.style(os.date("%H:%M"), "brightblack") end
//! ```
//!
//! A string works too, for a prompt that never changes.

use super::util::{ok, put, text};
use crate::interactive::theme::{self, Color, Style};
use crate::lua::engine::Registry;
use crate::lua::eval::LuaError;
use crate::lua::eval::value::{Table, Value};
use std::rc::Rc;

/// Registry keys the prompt lives under.
pub(crate) const LEFT: &str = "prompt.left";
pub(crate) const RIGHT: &str = "prompt.right";
pub(crate) const CONTINUATION: &str = "prompt.continuation";

/// Add `oslo.prompt`, `oslo.style`, `oslo.git` and `oslo.path.shorten`.
pub fn install(oslo: &mut Table, registry: &Registry) {
    oslo.set(Value::str("prompt"), build(registry));
    put(oslo, "style", |_, args| {
        let body = text(&args, 1, "oslo.style")?;
        ok(Value::str(
            style_from(args.get(1)).paint(&body, theme::depth()),
        ))
    });
    oslo.set(Value::str("git"), git());
}

/// The `oslo.prompt` table.
///
/// Deliberately **empty**, with both `__index` and `__newindex`. Lua only consults `__newindex`
/// for a key the table does not already have — so pre-filling `left` with a setter function, which
/// was the first shape this took, meant `oslo.prompt.left = f` quietly replaced that field and
/// never reached the registry at all. The prompt simply never changed, and nothing said why.
fn build(registry: &Registry) -> Value {
    let table = Rc::new(std::cell::RefCell::new(Table::new()));
    let mut meta = Table::new();

    let for_read = Rc::clone(registry);
    meta.set(
        Value::str("__index"),
        super::util::native("oslo.prompt.__index", move |_, args| {
            let Some(Value::Str(field)) = args.get(1) else {
                return ok(Value::Nil);
            };
            match key_for(field) {
                Some(key) => ok(for_read.borrow().get(key).cloned().unwrap_or(Value::Nil)),
                None => ok(Value::Nil),
            }
        }),
    );

    let for_write = Rc::clone(registry);
    meta.set(
        Value::str("__newindex"),
        super::util::native("oslo.prompt.__newindex", move |_, args| {
            let Some(Value::Str(field)) = args.get(1) else {
                return Err(LuaError::new("oslo.prompt: the field must be a name"));
            };
            let Some(key) = key_for(field) else {
                return Err(LuaError::new(format!(
                    "oslo.prompt.{field} is not a prompt; the prompts are left, right and \
                     continuation"
                )));
            };
            match args.get(2) {
                Some(value) if !matches!(value, Value::Nil) => {
                    for_write
                        .borrow_mut()
                        .insert(key.to_string(), value.clone());
                }
                // Assigning nil removes it, and the shell goes back to its own prompt.
                _ => {
                    for_write.borrow_mut().remove(key);
                }
            }
            ok(Value::Bool(true))
        }),
    );

    table.borrow_mut().metatable = Some(Rc::new(std::cell::RefCell::new(meta)));
    Value::Table(table)
}

/// The registry key a `oslo.prompt` field maps to.
fn key_for(field: &str) -> Option<&'static str> {
    match field {
        "left" => Some(LEFT),
        "right" => Some(RIGHT),
        "continuation" => Some(CONTINUATION),
        _ => None,
    }
}

/// A style written as `oslo.style(text, "green")` or `oslo.style(text, {fg = …, bold = true})`.
fn style_from(value: Option<&Value>) -> Style {
    match value {
        Some(Value::Str(name)) => Color::parse(name).map(Style::fg).unwrap_or_default(),
        Some(Value::Table(table)) => {
            let table = table.borrow();
            let colour = |key: &str| match table.get(&Value::str(key)) {
                Value::Str(name) => Color::parse(&name),
                _ => None,
            };
            Style {
                fg: colour("fg"),
                bg: colour("bg"),
                bold: table.get(&Value::str("bold")).truthy(),
                dim: table.get(&Value::str("dim")).truthy(),
                italic: table.get(&Value::str("italic")).truthy(),
                underline: table.get(&Value::str("underline")).truthy(),
                reverse: table.get(&Value::str("reverse")).truthy(),
            }
        }
        _ => Style::default(),
    }
}

/// `oslo.git` — what a prompt asks about a repository.
fn git() -> Value {
    let mut git = Table::new();

    // oslo.git.branch() -> "main", a short hash when detached, or nil outside a repository.
    put(&mut git, "branch", |_, _| {
        ok(match crate::interactive::prompt::git_branch() {
            Some(branch) => Value::str(branch),
            None => Value::Nil,
        })
    });

    // oslo.git.root() -> the working tree's top directory, or nil.
    put(&mut git, "root", |_, _| {
        ok(match crate::interactive::prompt::git_root() {
            Some(root) => Value::str(root.display().to_string()),
            None => Value::Nil,
        })
    });

    Value::table(git)
}

/// `oslo.path.shorten` — added to the existing `oslo.path` table.
pub fn shorten(table: &mut Table) {
    // oslo.path.shorten(path, keep) -> "~/d/o/t/rush"
    //
    // Home becomes `~` and every component but the last `keep` is cut to its first character,
    // which is the abbreviation every prompt eventually grows because a deep path pushes the
    // place you type off the right of the screen.
    put(table, "shorten", |_, args| {
        let path = text(&args, 1, "oslo.path.shorten")?;
        let keep = args
            .get(1)
            .and_then(Value::as_number)
            .and_then(|n| n.as_int())
            .unwrap_or(1)
            .max(0) as usize;
        ok(Value::str(crate::interactive::prompt::shorten(&path, keep)))
    });

    // oslo.path.home(path) -> the same path with $HOME written as `~`.
    put(table, "home", |_, args| {
        let path = text(&args, 1, "oslo.path.home")?;
        ok(Value::str(crate::interactive::prompt::tilde(&path)))
    });
}
