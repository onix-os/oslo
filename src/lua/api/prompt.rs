//! `oslo.prompt`, and the helpers a prompt function needs.
//!
//! A prompt is a Lua function, not a format string. `PS1`'s escapes are a small language that has
//! to grow a new letter for every new thing anyone wants in a prompt; a function needs nothing
//! added to it to call `oslo.git.branch()` or to count something itself.
//!
//! ```lua
//! oslo.prompt.left = function()
//!   return oslo.ui.style(oslo.path.shorten(oslo.fs.cwd()), "blue") .. " ❯ "
//! end
//! oslo.prompt.right = function() return oslo.ui.style(os.date("%H:%M"), "brightblack") end
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
/// What the prompt is redrawn as once its line has been accepted.
///
/// A "transient" prompt: the full prompt while you are typing, something short above the output
/// afterwards, so a screen of history is not three-quarters prompt. zsh users build this by
/// aliasing ZLE widgets — oh-my-posh wraps `zle-line-init` and calls `.recursive-edit` itself,
/// powerlevel10k wraps `zle-line-finish` — several hundred lines of editor surgery between them.
///
/// It is a render key here because the shell owns its own line editor and can simply redraw. Unset
/// means "leave the prompt as it was", which is what every shell does today.
pub(crate) const TRANSIENT: &str = "prompt.transient";

/// Add `oslo.prompt`, `oslo.ui.style`, `oslo.git` and `oslo.path.shorten`.
pub fn install(oslo: &mut Table, ui: &mut Table, registry: &Registry) {
    oslo.set(Value::str("prompt"), build(registry));
    // Painting text is `oslo.ui`; the prompt's own configuration stays `oslo.prompt`, which is a
    // table a config assigns into rather than a function it calls.
    put(ui, "style", |_, args| {
        let body = text(&args, 1, "oslo.ui.style")?;
        ok(Value::str(
            style_from(args.get(1)).paint(&body, theme::depth()),
        ))
    });
    oslo.set(Value::str("git"), git());
    // The fine-grained shape: a prompt as a list of named, prioritised pieces rather than one
    // opaque string. See `super::segment`.
    oslo.set(Value::str("segment"), super::segment::constructor());
    oslo.set(Value::str("theme"), theme_table());
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
                     continuation, transient"
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
        "transient" => Some(TRANSIENT),
        _ => None,
    }
}

/// A style written as `oslo.ui.style(text, "green")` or `oslo.ui.style(text, {fg = …, bold = true})`.
/// The `oslo.theme` table, whose `styles` field names colours.
///
/// `__newindex` again, for the reason `oslo.prompt` needs it: a plain table would accept the
/// assignment into itself and never reach the style registry, so the config would look right and
/// do nothing.
fn theme_table() -> Value {
    let styles = Rc::new(std::cell::RefCell::new(Table::new()));
    let mut meta = Table::new();
    meta.set(
        Value::str("__newindex"),
        super::util::native("oslo.theme.styles.__newindex", move |_, args| {
            let (Some(Value::Str(name)), Some(Value::Str(spec))) = (args.get(1), args.get(2))
            else {
                return Err(LuaError::new(
                    "oslo.theme.styles: a name maps to a style string, as in \
                     styles['git.branch'] = 'fg:cyan bold'"
                        .to_string(),
                ));
            };
            theme::styles::define(name, spec);
            ok(Value::Bool(true))
        }),
    );
    styles.borrow_mut().metatable = Some(Rc::new(std::cell::RefCell::new(meta)));

    let mut theme_table = Table::new();
    theme_table.set(Value::str("styles"), Value::Table(styles));
    Value::Table(Rc::new(std::cell::RefCell::new(theme_table)))
}

/// A span's `style`, which is a *name*.
///
/// Two vocabularies, because both are worth having. A dotted name — `prompt.user`, `prompt.git` —
/// is a **theme slot**: it follows whatever colour scheme is loaded, which is the whole point of
/// naming styles rather than writing colours into the prompt. Anything else is parsed as a colour,
/// so `"cyan"` or `"#8be9fd"` works without the config having to define a theme first.
pub fn style_named(name: &str) -> Style {
    // A config's own definition wins. That is what naming styles is for: `oslo.theme.styles` is
    // the one place a colour scheme lives, and a name defined there must outrank oslo's idea of
    // what, say, `prompt.git` should look like.
    if let Some(defined) = theme::styles::lookup(name) {
        return defined;
    }
    let theme = theme::current();
    match name {
        "prompt.cwd" => theme.prompt.cwd,
        "prompt.host" => theme.prompt.host,
        "prompt.user" => theme.prompt.user,
        "prompt.git" => theme.prompt.git,
        "prompt.ok" => theme.prompt.ok,
        "prompt.failed" => theme.prompt.failed,
        "prompt.aside" => theme.prompt.aside,
        // An unknown dotted name is a typo, not a colour: `prompt.usr` should not silently paint
        // nothing *and* not be mistaken for a colour called "prompt.usr".
        other => Color::parse(other).map(Style::fg).unwrap_or_default(),
    }
}

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
