//! Reading a [`Look`] out of a Lua table.
//!
//! The field names are the shell flags with the dashes taken out, which is the rule the whole
//! `oslo.ui` surface follows: `--filter-at bottom` is `filter_at = "bottom"`. One shape to learn,
//! and a prompt looks the same whichever language asked for it.
//!
//! ```lua
//! oslo.ui.filter{
//!   items = history,
//!   look = "history",              -- the whole combination under one name
//!   slot_right = " {n}/{total} ",  -- and then whatever you want changed
//!   stripe = 236,
//! }
//! ```

use crate::interactive::ask::look::{Look, Preset, Where, Width};
use crate::interactive::scanner::Scanner;
use crate::interactive::theme::{Color, Style};
use crate::lua::eval::LuaError;
use crate::lua::eval::value::{Table, Value};

use super::prompt::{flag, maybe, size};

/// The look a table asks for, starting from whatever preset it named.
pub(crate) fn look_of(t: &Table) -> Result<Look, LuaError> {
    // The preset is applied first and whole, so `look = "history"` plus one field is "that look,
    // with this changed" rather than a pile of fields that have to agree with each other.
    let mut look = match maybe(t, "look").or_else(|| maybe(t, "preset")) {
        Some(name) => Preset::parse(&name)
            .ok_or_else(|| LuaError::new(format!("{name}: look is plain, history or menu")))?
            .look(),
        None => Look::default(),
    };

    if let Some(name) = maybe(t, "filter_at") {
        look.filter_at = Where::parse(&name)
            .ok_or_else(|| LuaError::new(format!("{name}: filter_at is top or bottom")))?;
    }
    if let Some(name) = maybe(t, "list_width") {
        look.width = Width::parse(&name)
            .ok_or_else(|| LuaError::new(format!("{name}: list_width is content or full")))?;
    }
    if let Value::Bool(reverse) = t.get(&Value::str("reverse")) {
        look.reverse = reverse;
    } else if flag(t, "reverse") {
        look.reverse = true;
    }

    for (name, slot) in [
        ("slot_left", &mut look.left),
        ("slot_right", &mut look.right),
        ("prompt", &mut look.prompt),
        ("placeholder", &mut look.placeholder),
        ("marker", &mut look.marker),
    ] {
        if let Some(text) = maybe(t, name) {
            *slot = text;
        }
    }

    // Absent keeps whatever the preset set rather than zeroing it, which is what makes
    // `{look = "history", list_pad = 2}` mean the history look with wider margins.
    look.surface_rows = size(t, "surface_rows", look.surface_rows).max(1);
    look.gap = size(t, "list_gap", look.gap);
    look.pad = size(t, "list_pad", look.pad);

    if let Some(c) = colour(t, "surface")? {
        look.surface = Some(c);
    }
    // A stripe is a background, and `stripe = false` is how a preset's stripe is turned off —
    // there is no colour that means "none".
    if let Value::Bool(false) = t.get(&Value::str("stripe")) {
        look.stripe = None;
    } else if let Some(c) = colour(t, "stripe")? {
        look.stripe = Some(Style {
            bg: Some(c),
            ..Style::default()
        });
    }
    for (name, foreground, slot) in [
        ("sel_fg", true, 0usize),
        ("sel_bg", false, 0),
        ("row_fg", true, 1),
        ("row_bg", false, 1),
        ("hit_fg", true, 2),
        ("hit_bg", false, 2),
    ] {
        let Some(c) = colour(t, name)? else { continue };
        let style = match slot {
            0 => &mut look.selected,
            1 => &mut look.row,
            _ => &mut look.hit,
        };
        match foreground {
            true => style.fg = Some(c),
            false => style.bg = Some(c),
        }
    }
    if let Some(c) = colour(t, "accent")? {
        look.accent = Style::fg(c);
    }
    if let Some(c) = colour(t, "meta_fg")? {
        look.meta_style = Style::fg(c);
    }
    look.meta_columns = size(t, "meta_columns", look.meta_columns);

    // `scanner = false` turns a preset's sweep off; a number is its width; `true` is the default
    // one. Three spellings because a bare `scanner = true` is what anyone writes first and a width
    // is the only thing anyone changes.
    match t.get(&Value::str("scanner")) {
        Value::Bool(false) => look.scanner = None,
        Value::Bool(true) => look.scanner = Some(Scanner::default()),
        Value::Number(n) => {
            look.scanner = Some(Scanner {
                width: n.as_int().unwrap_or(9).clamp(2, 32) as u8,
                ..look.scanner.unwrap_or_default()
            })
        }
        _ => {}
    }
    if let Some(text) = maybe(t, "badge") {
        look.badge = text;
    }
    if let Some(c) = colour(t, "badge_fg")? {
        look.badge_style.fg = Some(c);
    }
    if let Some(c) = colour(t, "badge_bg")? {
        look.badge_style.bg = Some(c);
    }
    Ok(look)
}

/// A colour field, refused by name rather than ignored — a misspelt colour that silently did
/// nothing is the failure this whole module is arranged to avoid.
fn colour(t: &Table, name: &str) -> Result<Option<Color>, LuaError> {
    let Some(text) = maybe(t, name) else {
        return Ok(None);
    };
    Color::parse(&text)
        .map(Some)
        .ok_or_else(|| LuaError::new(format!("{text}: not a colour")))
}
