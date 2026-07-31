//! Turning `oslo.theme` into a [`Theme`].
//!
//! Every field is optional and layered over the default, so a config that names one colour keeps
//! the other forty. Writing `oslo.theme = { syntax = { command = "cyan" } }` is the natural way to
//! change one thing, and it must not blank the pager.
//!
//! An entry may be a string or a table: `"green"` and `{fg = "green", bold = true}` are the same
//! kind of value, because the short form is what people write and the long form is what they need
//! the moment they want it bold.

use super::{Color, KindColors, Pager, Prompt, Style, Syntax, Theme};
use crate::lua::eval::Value;

/// Read `oslo.theme`, answering the theme and what was wrong with it.
///
/// Deliberately free of side effects — it does not install what it built. Parsing that writes to
/// a process-wide global cannot be tested in parallel: two cases racing on the same static pass
/// or fail depending on scheduling, which is a test that lies. The caller installs.
///
/// Diagnostics rather than silence: a mistyped colour name is the single most likely thing to be
/// wrong in a theme, and an element that quietly keeps its default looks like oslo ignoring the
/// config. One complaint per line, each naming its path.
pub fn read_lua_theme(value: &Value) -> (Theme, Vec<String>) {
    let mut problems = Vec::new();
    let Value::Table(table) = value else {
        if !matches!(value, Value::Nil) {
            problems.push(format!(
                "oslo.theme must be a table, not a {}",
                value.type_name()
            ));
        }
        return (Theme::default(), problems);
    };

    let mut theme = Theme::default();
    let table = table.borrow();

    if let Value::Table(syntax) = table.get(&Value::str("syntax")) {
        read_syntax(&syntax.borrow(), &mut theme.syntax, &mut problems);
    }
    if let Value::Table(pager) = table.get(&Value::str("pager")) {
        read_pager(&pager.borrow(), &mut theme.pager, &mut problems);
    }
    if let Value::Table(prompt) = table.get(&Value::str("prompt")) {
        read_prompt(&prompt.borrow(), &mut theme.prompt, &mut problems);
    }

    (theme, problems)
}

/// One style field, left alone when the config does not name it.
fn field(
    table: &crate::lua::eval::Table,
    name: &str,
    path: &str,
    slot: &mut Style,
    problems: &mut Vec<String>,
) {
    if let Some(style) = style(
        &table.get(&Value::str(name)),
        &format!("{path}.{name}"),
        problems,
    ) {
        *slot = style;
    }
}

/// A theme entry as a [`Style`]: `"green"`, or `{fg = …, bg = …, bold = …}`.
fn style(value: &Value, path: &str, problems: &mut Vec<String>) -> Option<Style> {
    match value {
        Value::Nil => None,
        Value::Str(text) => match Color::parse(text) {
            Some(colour) => Some(Style::fg(colour)),
            None => {
                problems.push(format!("{path}: '{text}' is not a colour"));
                None
            }
        },
        Value::Table(table) => {
            let table = table.borrow();
            let mut out = Style::default();
            for (key, slot) in [("fg", true), ("bg", false)] {
                if let Value::Str(text) = table.get(&Value::str(key)) {
                    match Color::parse(&text) {
                        Some(colour) if slot => out.fg = Some(colour),
                        Some(colour) => out.bg = Some(colour),
                        None => problems.push(format!("{path}.{key}: '{text}' is not a colour")),
                    }
                }
            }
            out.bold = table.get(&Value::str("bold")).truthy();
            out.dim = table.get(&Value::str("dim")).truthy();
            out.italic = table.get(&Value::str("italic")).truthy();
            out.underline = table.get(&Value::str("underline")).truthy();
            out.reverse = table.get(&Value::str("reverse")).truthy();
            Some(out)
        }
        other => {
            problems.push(format!(
                "{path}: expected a colour or a table, got a {}",
                other.type_name()
            ));
            None
        }
    }
}

fn read_syntax(table: &crate::lua::eval::Table, into: &mut Syntax, problems: &mut Vec<String>) {
    let p = "oslo.theme.syntax";
    // `command` first, because the two that fall back to it need its final value.
    field(table, "command", p, &mut into.command, problems);

    // `builtin` and `function` inherit `command` unless named. That is fish's rule, and it is what
    // keeps a two-line theme from looking half-finished: setting `command` alone recolours all
    // three kinds of "this is the thing being run".
    let inherits = !matches!(table.get(&Value::str("command")), Value::Nil);
    for (name, slot) in [("builtin", 0), ("function", 1)] {
        let path = format!("{p}.{name}");
        let chosen = style(&table.get(&Value::str(name)), &path, problems);
        let value = match chosen {
            Some(style) => style,
            None if inherits => into.command,
            None => continue,
        };
        match slot {
            0 => into.builtin = value,
            _ => into.function = value,
        }
    }

    field(table, "keyword", p, &mut into.keyword, problems);
    field(table, "error", p, &mut into.error, problems);
    field(table, "param", p, &mut into.param, problems);
    field(table, "valid_path", p, &mut into.valid_path, problems);
    field(table, "option", p, &mut into.option, problems);
    // `quote` sets both, so a config written before they were split still works and still means
    // what it meant. Naming either one on its own then overrides that half.
    // A sentinel no config can write, so "was `quote` named at all?" is answerable — comparing
    // against the current value cannot tell a config that set it to the default from one that did
    // not set it, and that is exactly the case the back-compat needs to get right.
    let sentinel = Style {
        bold: true,
        dim: true,
        italic: true,
        underline: true,
        reverse: true,
        ..Style::default()
    };
    let mut both = sentinel;
    field(table, "quote", p, &mut both, problems);
    if both != sentinel {
        into.single_quote = both;
        into.double_quote = both;
    }
    field(table, "single_quote", p, &mut into.single_quote, problems);
    field(table, "double_quote", p, &mut into.double_quote, problems);
    field(table, "escape", p, &mut into.escape, problems);
    field(table, "operator", p, &mut into.operator, problems);
    field(table, "redirection", p, &mut into.redirection, problems);
    field(table, "end", p, &mut into.end, problems);
    field(table, "comment", p, &mut into.comment, problems);
    field(table, "variable", p, &mut into.variable, problems);
    field(
        table,
        "autosuggestion",
        p,
        &mut into.autosuggestion,
        problems,
    );
    field(table, "match_bracket", p, &mut into.match_bracket, problems);
}

fn read_pager(table: &crate::lua::eval::Table, into: &mut Pager, problems: &mut Vec<String>) {
    let p = "oslo.theme.pager";
    field(table, "text", p, &mut into.text, problems);
    field(table, "text_sel", p, &mut into.text_sel, problems);
    field(table, "match", p, &mut into.match_, problems);
    field(table, "desc", p, &mut into.desc, problems);
    field(table, "desc_sel", p, &mut into.desc_sel, problems);
    field(table, "scroll", p, &mut into.scroll, problems);

    // `bg` and `sel_bg` are bare colours rather than styles: they are the row's background and
    // nothing else.
    for (name, slot) in [("bg", 0), ("sel_bg", 1), ("kind_sel", 2)] {
        if let Value::Str(text) = table.get(&Value::str(name)) {
            match (Color::parse(&text), slot) {
                (Some(colour), 0) => into.bg = Some(colour),
                (Some(colour), 1) => into.sel_bg = Some(colour),
                (Some(colour), _) => into.kind_sel = Some(colour),
                (None, _) => problems.push(format!("{p}.{name}: '{text}' is not a colour")),
            }
        }
    }

    if let Value::Table(kinds) = table.get(&Value::str("kind")) {
        read_kinds(&kinds.borrow(), &mut into.kind, problems);
    }
}

fn read_kinds(table: &crate::lua::eval::Table, into: &mut KindColors, problems: &mut Vec<String>) {
    let p = "oslo.theme.pager.kind";
    field(table, "command", p, &mut into.command, problems);
    field(table, "builtin", p, &mut into.builtin, problems);
    field(table, "file", p, &mut into.file, problems);
    field(table, "dir", p, &mut into.dir, problems);
    field(table, "variable", p, &mut into.variable, problems);
    field(table, "history", p, &mut into.history, problems);
    field(table, "alias", p, &mut into.alias, problems);
    field(table, "other", p, &mut into.other, problems);
}

fn read_prompt(table: &crate::lua::eval::Table, into: &mut Prompt, problems: &mut Vec<String>) {
    let p = "oslo.theme.prompt";
    field(table, "cwd", p, &mut into.cwd, problems);
    field(table, "git", p, &mut into.git, problems);
    field(table, "ok", p, &mut into.ok, problems);
    field(table, "failed", p, &mut into.failed, problems);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::eval;

    /// Build `oslo.theme` by running a chunk, which is how a real config produces one.
    fn theme_from(source: &str) -> (Theme, Vec<String>) {
        let interp = eval::Interp::new("theme test");
        let ast = eval::parse(source).expect("the test chunk must parse");
        interp.run_ast(&ast).expect("the test chunk must run");
        read_lua_theme(&interp.global("theme"))
    }

    /// Naming one colour must not blank the other forty.
    #[test]
    fn a_partial_theme_keeps_every_default_it_did_not_name() {
        let (theme, problems) = theme_from("theme = { syntax = { command = 'cyan' } }");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            theme.syntax.command,
            Style::fg(Color::Basic {
                index: 6,
                bright: false
            })
        );
        // Untouched, and still the default rather than blank.
        assert_eq!(theme.pager.bg, Pager::default().bg);
        assert_eq!(theme.syntax.single_quote, Syntax::default().single_quote);
    }

    #[test]
    fn a_style_may_be_a_string_or_a_table() {
        let (theme, problems) = theme_from(
            "theme = { syntax = { quote = 'yellow',
                                  error = { fg = '#ff0000', bold = true, underline = true } } }",
        );
        assert!(problems.is_empty(), "{problems:?}");
        // `quote` still sets both halves, so a config written before they were split keeps
        // meaning what it meant.
        let yellow = Style::fg(Color::Basic {
            index: 3,
            bright: false,
        });
        assert_eq!(theme.syntax.single_quote, yellow);
        assert_eq!(theme.syntax.double_quote, yellow);
        assert_eq!(theme.syntax.error.fg, Some(Color::Rgb(0xff, 0, 0)));
        assert!(theme.syntax.error.bold && theme.syntax.error.underline);
    }

    /// The three that fall back to `command` do so only when the config sets `command` — a theme
    /// that names neither keeps both defaults.
    #[test]
    fn builtin_and_function_fall_back_to_command() {
        let (theme, _) = theme_from("theme = { syntax = { command = 'blue' } }");
        let blue = Style::fg(Color::Basic {
            index: 4,
            bright: false,
        });
        assert_eq!(theme.syntax.command, blue);
        assert_eq!(theme.syntax.builtin, blue);
        assert_eq!(theme.syntax.function, blue);

        // Named explicitly, it wins over the fall-back.
        let (theme, _) = theme_from("theme = { syntax = { command = 'blue', builtin = 'red' } }");
        assert_eq!(theme.syntax.command, blue);
        assert_eq!(
            theme.syntax.builtin,
            Style::fg(Color::Basic {
                index: 1,
                bright: false
            })
        );
    }

    /// A mistyped colour is reported by name and path. Silently keeping the default looks exactly
    /// like oslo ignoring the config.
    #[test]
    fn a_bad_colour_is_named_rather_than_ignored() {
        let (theme, problems) = theme_from("theme = { syntax = { command = 'chartreuse' } }");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("oslo.theme.syntax.command"),
            "{problems:?}"
        );
        assert!(problems[0].contains("chartreuse"), "{problems:?}");
        // And the element keeps its default rather than becoming black.
        assert_eq!(theme.syntax.command, Syntax::default().command);
    }

    #[test]
    fn a_theme_that_is_not_a_table_is_reported() {
        let (_, problems) = theme_from("theme = 'dark'");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("must be a table"), "{problems:?}");
    }

    #[test]
    fn pager_kinds_are_read_through() {
        let (theme, problems) = theme_from(
            "theme = { pager = { sel_bg = '#3d375e',
                                 kind = { dir = { fg = '#000000', bg = '#82e2ff' } } } }",
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(theme.pager.sel_bg, Some(Color::Rgb(0x3d, 0x37, 0x5e)));
        assert_eq!(theme.pager.kind.dir.bg, Some(Color::Rgb(0x82, 0xe2, 0xff)));
        // A kind the config did not name keeps its default pill.
        assert_eq!(theme.pager.kind.file, KindColors::default().file);
    }
}
