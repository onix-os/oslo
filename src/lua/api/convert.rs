//! Turning another program's output into something a script can index.
//!
//! `oslo.fs.ls()` answers with fields because we wrote it. `df` does not, and never will — so a
//! shell that talks to the rest of the system needs a way in. These are the explicit converters:
//! the caller says what shape the text is, and gets a table.
//!
//! Explicit rather than automatic. Guessing whether output is columns or JSON works until the day
//! a filename contains a space or a log line happens to start with `{`, and a converter that is
//! usually right is worse than one that is always asked.

use super::util::{list, ok, opt_text, put, text};
use crate::lua::eval::value::{Table, Value};

pub fn build() -> Value {
    let mut it = Table::new();

    // oslo.from_lines(text) -> {"first", "second", ...}
    //
    // A trailing newline ends the last line rather than starting an empty one, which is what
    // `wc -l` counts and what anyone means by "the lines of this output".
    put(&mut it, "from_lines", |_, args| {
        let source = text(&args, 1, "oslo.from_lines")?;
        let body = source.strip_suffix('\n').unwrap_or(&source);
        if body.is_empty() {
            return ok(list([]));
        }
        ok(list(body.split('\n').map(Value::str)))
    });

    // oslo.from_columns(text, header) -> a table per line
    //
    // With no header, each line becomes a *list* of fields, split on runs of whitespace the way
    // `awk` does. With one, each line becomes a record keyed by the names given, so
    // `oslo.from_columns(df, {"fs", "size", "used", "avail", "pct", "mount"})` reads like the
    // thing it describes rather than like `$4`.
    put(&mut it, "from_columns", |_, args| {
        let source = text(&args, 1, "oslo.from_columns")?;
        let names: Vec<String> = match args.get(1) {
            Some(Value::Table(t)) => t
                .borrow()
                .sequence()
                .iter()
                .map(|v| match v {
                    Value::Str(s) => s.to_string(),
                    other => other.to_display(),
                })
                .collect(),
            _ => Vec::new(),
        };

        let mut rows = Vec::new();
        for line in source.lines() {
            if line.trim().is_empty() {
                continue;
            }
            // `split_whitespace`, so runs of spaces are one separator and a leading indent does
            // not produce an empty first field. Column *alignment* is not preserved, which is the
            // trade every `awk` one-liner already makes.
            let fields: Vec<&str> = line.split_whitespace().collect();
            if names.is_empty() {
                rows.push(list(fields.iter().map(Value::str)));
                continue;
            }
            let mut row = Table::new();
            for (i, name) in names.iter().enumerate() {
                // A missing field is absent rather than empty: a short line and a line with an
                // empty column are different, and only one of them is a parsing problem.
                if let Some(value) = fields.get(i) {
                    row.set(Value::str(name), Value::str(value));
                }
            }
            // The tail, when a command printed more columns than were named — `ps` puts the whole
            // command line in the last one, spaces included, and dropping it loses the answer.
            if fields.len() > names.len()
                && let Some(last) = names.last()
            {
                row.set(
                    Value::str(last),
                    Value::str(fields[names.len() - 1..].join(" ")),
                );
            }
            rows.push(Value::table(row));
        }
        ok(list(rows))
    });

    // oslo.from_pairs(text, separator) -> {NAME = value, ...}
    //
    // For the `key=value` output half the system speaks: `env`, `/etc/os-release`, `systemctl
    // show`. The separator defaults to `=`.
    put(&mut it, "from_pairs", |_, args| {
        let source = text(&args, 1, "oslo.from_pairs")?;
        let separator = opt_text(&args, 2, "oslo.from_pairs")?.unwrap_or_else(|| "=".to_string());
        let mut table = Table::new();
        for line in source.lines() {
            let Some((key, value)) = line.split_once(&separator) else {
                continue;
            };
            // Quotes are stripped because `/etc/os-release` writes `NAME="Alpine Linux"` and a
            // caller comparing against `Alpine Linux` should not have to know that.
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);
            table.set(Value::str(key.trim()), Value::str(value));
        }
        ok(Value::table(table))
    });

    // `oslo.from_json` is not built here: it is the *same function* as `oslo.json.decode`,
    // aliased in `super::install` so that `from_*` reads as one family without there being two
    // parsers to keep in step.

    Value::table(it)
}
