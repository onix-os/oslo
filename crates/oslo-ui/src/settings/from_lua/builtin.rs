//! `oslo.builtin.*` — the settings that belong to a builtin rather than to the editor.
//!
//! Split from [`super`] when that file crossed the 600-line limit, and along the seam it
//! already had: everything else there describes the line editor, the prompt or the completion
//! dropdown, and this describes `rm` and `nav`. It is also the one group nested a level deeper —
//! `builtin` is a namespace *per builtin*, not a group of settings — so it reads differently from
//! everything around it.

use super::super::Settings;
use super::read::{flag, number};
use oslo_base::value::Table;
use oslo_base::value::Value;

/// Read `oslo.builtin.*` into `settings`, adding to `problems` for anything unusable.
pub(super) fn read(oslo: &Table, settings: &mut Settings, problems: &mut Vec<String>) {
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
        // The browser to run instead of oslo's own. Empty stays empty, which is the built-in one.
        if let Value::Table(argv) = nav.get_str("command") {
            settings.command = argv
                .borrow()
                .sequence()
                .iter()
                .filter_map(|word| match word {
                    Value::Str(word) => Some(word.to_string()),
                    _ => None,
                })
                .collect();
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
}
