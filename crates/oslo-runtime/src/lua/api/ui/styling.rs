//! `oslo.ui.style` — text with a border, padding and every attribute a style carries.
//!
//! Split out of [`super::prompt`], which had grown past the 600-line limit: the styling widget is
//! the one thing in that file that draws nothing and asks nobody anything, so it is the seam.

use super::super::util::{ok, put};
use super::prompt::{field, flag, maybe, size, spec};
use oslo_base::value::{Table, Value};
use oslo_ui::ask::{Border, Styling, style};
use oslo_ui::theme;

pub fn install(ui: &mut Table) {
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
                // **Every attribute the style has, not three of them.** `bold` was read and the
                // other six were dropped in silence, so `oslo.ui.style(x, { underline = true })`
                // answered unstyled text and there was nothing to say which layer had eaten it.
                settings.style.bold = flag(&t, "bold");
                settings.style.dim = flag(&t, "dim");
                settings.style.italic = flag(&t, "italic");
                settings.style.underline = flag(&t, "underline");
                settings.style.reverse = flag(&t, "reverse");
                settings.style.blink = flag(&t, "blink");
                settings.style.hidden = flag(&t, "hidden");
                settings.style.strike = flag(&t, "strike");
                settings.padding_x = size(&t, "padding_x", 0);
                settings.padding_y = size(&t, "padding_y", 0);
                settings.width = match t.get_str("width") {
                    // The same ceiling as every other width: see `util::width_of`.
                    Value::Number(n) => n
                        .as_int()
                        .map(|i| (i.max(0) as usize).min(crate::lua::api::util::WIDEST)),
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
