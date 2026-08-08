//! Putting text where it fits: columns and tables, measured in cells.
//!
//! Both of these are the kind of thing every config writes badly once. They need the cell-accurate
//! measuring in [`super::text`], they need the terminal width, and the column packing needs a
//! search rather than a formula — get any of the three wrong and the output wraps, which in a
//! terminal is not a cosmetic failure but a corrupted redraw.

use super::super::util::{ok, put};
use super::text::align_to;
use oslo_lua::value::{Number, Table, Value};
use oslo_ui::dropdown::width;

/// The strings in a Lua sequence, in order, skipping anything that is not one.
fn strings(value: &Value) -> Vec<String> {
    let Value::Table(table) = value else {
        return Vec::new();
    };
    table
        .borrow()
        .sequence()
        .iter()
        .map(|v| match v {
            Value::Str(s) => s.to_string(),
            Value::Number(n) => n.to_string(),
            other => other.type_name().to_string(),
        })
        .collect()
}

/// A field of an options table.
fn option(args: &[Value], at: usize, name: &str) -> Option<Value> {
    match args.get(at) {
        Some(Value::Table(t)) => Some(t.borrow().get(&Value::str(name))),
        _ => None,
    }
}

fn option_usize(args: &[Value], at: usize, name: &str) -> Option<usize> {
    match option(args, at, name) {
        Some(Value::Number(n)) if n.as_float() >= 0.0 => Some(n.as_float() as usize),
        _ => None,
    }
}

pub fn install(ui: &mut Table) {
    // oslo.ui.columns(items, [{ width = n, gap = n }]) -> { "line", ... }
    put(ui, "columns", |_, args| {
        let items = strings(args.first().unwrap_or(&Value::Nil));
        let cols = option_usize(&args, 1, "width").unwrap_or_else(width::terminal_cols);
        let gap = option_usize(&args, 1, "gap").unwrap_or(2);
        let lines = in_columns(&items, cols, gap);
        let mut out = Table::new();
        for (i, line) in lines.into_iter().enumerate() {
            out.set(Value::Number(Number::Int(i as i64 + 1)), Value::str(line));
        }
        ok(Value::table(out))
    });

    // oslo.ui.grid(rows, [{ headers = {...}, align = {...}, gap = n, width = n }]) -> { "line", … }
    //
    // **Named `grid`, not `table`, and that is a bug fix.** This was `oslo.ui.table` and
    // `ui::prompt` installs a `table` of its own *after* this runs — a completely different
    // function, the interactive row picker — so this one was unreachable for as long as both
    // existed. `oslo.ui.table(rows, opts)` reached the picker, which answers `nil` without a
    // terminal, so a script asking for a formatted table quietly got nothing.
    //
    // The picker keeps `table` because the `ui` builtin's `table` is the picker too, and the two
    // surfaces should not disagree about what a name means. This one is a grid of cells.
    put(ui, "grid", |_, args| {
        let rows: Vec<Vec<String>> = match args.first() {
            Some(Value::Table(t)) => t.borrow().sequence().iter().map(strings).collect(),
            _ => Vec::new(),
        };
        let headers = option(&args, 1, "headers").map(|h| strings(&h));
        let aligns = option(&args, 1, "align").map(|a| strings(&a));
        let gap = option_usize(&args, 1, "gap").unwrap_or(2);
        let cols = option_usize(&args, 1, "width").unwrap_or_else(width::terminal_cols);
        let lines = as_table(&rows, headers.as_deref(), aligns.as_deref(), gap, cols);
        let mut out = Table::new();
        for (i, line) in lines.into_iter().enumerate() {
            out.set(Value::Number(Number::Int(i as i64 + 1)), Value::str(line));
        }
        ok(Value::table(out))
    });

    // oslo.ui.rule([char], [width]) -> a horizontal line
    put(ui, "rule", |_, args| {
        let ch = match args.first() {
            Some(Value::Str(s)) if !s.is_empty() => s.chars().next().unwrap_or('─'),
            _ => '─',
        };
        let cols = match args.get(1) {
            Some(Value::Number(n)) if n.as_float() > 0.0 => n.as_float() as usize,
            _ => width::terminal_cols(),
        };
        ok(Value::str(ch.to_string().repeat(cols)))
    });

    // oslo.ui.print(lines_or_line) — write a sequence of lines, or one line
    //
    // Takes a bare string as well as a sequence because every other function here answers with a
    // sequence *except* `rule`, and `ui.print(ui.rule())` printing nothing at all is the kind of
    // silent no-op that makes a library feel broken.
    put(ui, "print", |_, args| {
        match args.first() {
            Some(Value::Str(line)) => println!("{line}"),
            Some(value) => {
                for line in strings(value) {
                    println!("{line}");
                }
            }
            None => {}
        }
        Ok(Vec::new())
    });
}

/// Lay `items` out down-then-across, in as many columns as fit.
///
/// **Down-then-across, like `ls`**, not across-then-down: a reader scans a column, and a list that
/// reads left-to-right across a 120-column terminal is one nobody can find anything in.
///
/// The column count is searched downwards from the most that could possibly fit rather than
/// computed, because the width of a column depends on which items land in it — which depends on the
/// column count. There is no closed form, and the loop is over at most `items.len()` candidates of
/// a cheap measurement.
pub fn in_columns(items: &[String], cols: usize, gap: usize) -> Vec<String> {
    if items.is_empty() {
        return Vec::new();
    }
    let widths: Vec<usize> = items.iter().map(|s| width::display_width(s)).collect();
    let widest = widths.iter().copied().max().unwrap_or(1).max(1);
    // Even one column may not fit; that is still one column, with the items overflowing, because
    // dropping them would be worse than a wrap the caller can see.
    let most = ((cols + gap) / (widest + gap)).clamp(1, items.len());

    for columns in (1..=most).rev() {
        let rows = items.len().div_ceil(columns);
        let mut column_widths = Vec::with_capacity(columns);
        for c in 0..columns {
            let start = c * rows;
            let end = (start + rows).min(items.len());
            column_widths.push(widths[start..end].iter().copied().max().unwrap_or(0));
        }
        let total: usize = column_widths.iter().sum::<usize>() + gap * columns.saturating_sub(1);
        if total <= cols || columns == 1 {
            return draw_columns(items, rows, &column_widths, gap);
        }
    }
    items.to_vec()
}

fn draw_columns(items: &[String], rows: usize, widths: &[usize], gap: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(rows);
    for r in 0..rows {
        let mut line = String::new();
        for (c, w) in widths.iter().enumerate() {
            let Some(item) = items.get(c * rows + r) else {
                continue;
            };
            if c > 0 {
                line.push_str(&" ".repeat(gap));
            }
            // The last column on a row is not padded: trailing spaces are invisible, and on a row
            // that exactly fills the terminal they are what turns a fitting line into a wrapped one.
            let last = widths
                .iter()
                .enumerate()
                .skip(c + 1)
                .all(|(later, _)| items.get(later * rows + r).is_none());
            if last {
                line.push_str(item);
            } else {
                line.push_str(&align_to(item, *w, "left"));
            }
        }
        out.push(line);
    }
    out
}

/// Render `rows` as aligned columns, with optional headers and per-column alignment.
///
/// Columns are as wide as their widest cell, then shrunk together if the total exceeds the
/// terminal: the widest column gives up a cell at a time until it fits, so one long path does not
/// squeeze every other column into uselessness.
pub fn as_table(
    rows: &[Vec<String>],
    headers: Option<&[String]>,
    aligns: Option<&[String]>,
    gap: usize,
    cols: usize,
) -> Vec<String> {
    let count = rows
        .iter()
        .map(Vec::len)
        .chain(headers.map(<[String]>::len))
        .max()
        .unwrap_or(0);
    if count == 0 {
        return Vec::new();
    }

    let mut widths = vec![0usize; count];
    for row in rows.iter().map(Vec::as_slice).chain(headers) {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(width::display_width(cell));
        }
    }
    shrink_to_fit(&mut widths, gap, cols);

    let mut out = Vec::with_capacity(rows.len() + headers.is_some() as usize);
    if let Some(headers) = headers {
        out.push(line_of(headers, &widths, aligns, gap));
    }
    for row in rows {
        out.push(line_of(row, &widths, aligns, gap));
    }
    out
}

/// Take cells off the widest column until the row fits.
fn shrink_to_fit(widths: &mut [usize], gap: usize, cols: usize) {
    let spacing = gap * widths.len().saturating_sub(1);
    // Below this there is no layout worth computing; leave it and let the caller see the overflow.
    const NARROWEST: usize = 3;
    while widths.iter().sum::<usize>() + spacing > cols {
        let Some(widest) = widths
            .iter()
            .enumerate()
            .filter(|(_, w)| **w > NARROWEST)
            .max_by_key(|(_, w)| **w)
            .map(|(i, _)| i)
        else {
            return;
        };
        widths[widest] -= 1;
    }
}

fn line_of(row: &[String], widths: &[usize], aligns: Option<&[String]>, gap: usize) -> String {
    let mut line = String::new();
    for (i, w) in widths.iter().enumerate() {
        let cell = row.get(i).map(String::as_str).unwrap_or("");
        if i > 0 {
            line.push_str(&" ".repeat(gap));
        }
        let align = aligns
            .and_then(|a| a.get(i))
            .map(String::as_str)
            .unwrap_or("left");
        let cut = width::truncate_to_width(cell, *w);
        // As in `draw_columns`: the final column is not padded.
        if i + 1 == widths.len() {
            line.push_str(&cut);
        } else {
            line.push_str(&align_to(&cut, *w, align));
        }
    }
    // A row with fewer cells than the table has columns would otherwise end in the gap before the
    // column it does not have. Trailing spaces are invisible on screen and wrong everywhere else —
    // in a diff, in a test, and on the row that exactly fills the terminal, where they wrap.
    line.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// Down-then-across, like `ls`. Reading order is the whole point of columns.
    ///
    /// Narrow enough to force two columns: four one-cell items in a wide terminal are one row, and
    /// that is also correct — it is what `ls` does — so a wide terminal cannot test the ordering.
    #[test]
    fn columns_read_downwards() {
        assert_eq!(
            in_columns(&owned(&["a", "b", "c", "d"]), 5, 2),
            vec!["a  c", "b  d"]
        );
        assert_eq!(
            in_columns(&owned(&["a", "b", "c", "d"]), 20, 2),
            vec!["a  b  c  d"],
            "and everything on one row when there is room"
        );
    }

    /// No line may exceed the terminal, or the layout has caused the wrap it exists to prevent.
    #[test]
    fn no_line_is_ever_wider_than_the_terminal() {
        let items = owned(&[
            "short",
            "a-much-longer-entry",
            "mid",
            "x",
            "another-long-one",
        ]);
        for cols in [10, 20, 40, 80] {
            for line in in_columns(&items, cols, 2) {
                let w = width::display_width(&line);
                assert!(
                    w <= cols || items.iter().any(|i| width::display_width(i) > cols),
                    "{cols} cols produced a {w}-cell line: {line:?}"
                );
            }
        }
    }

    /// A narrow terminal falls back to one column rather than dropping anything.
    #[test]
    fn one_column_is_the_floor_not_zero() {
        let lines = in_columns(&owned(&["aaaaaaaaaa", "bbbbbbbbbb"]), 4, 2);
        assert_eq!(lines.len(), 2, "both items must still be shown");
    }

    #[test]
    fn an_empty_list_lays_out_as_nothing() {
        assert!(in_columns(&[], 80, 2).is_empty());
        assert!(as_table(&[], None, None, 2, 80).is_empty());
    }

    /// Columns line up, and the header is measured with the body.
    #[test]
    fn a_table_aligns_its_columns() {
        let rows = vec![owned(&["a", "1"]), owned(&["bbbb", "22"])];
        let headers = owned(&["name", "n"]);
        let lines = as_table(&rows, Some(&headers), None, 2, 80);
        assert_eq!(lines, vec!["name  n", "a     1", "bbbb  22"]);
    }

    #[test]
    fn a_column_can_be_right_aligned() {
        let rows = vec![owned(&["a", "1"]), owned(&["bb", "22"])];
        let aligns = owned(&["left", "right"]);
        // The last column is never padded, so right-alignment shows in the *first* column here.
        let lines = as_table(&rows, None, Some(&aligns), 1, 80);
        assert_eq!(lines, vec!["a  1", "bb 22"]);
    }

    /// A row narrower than its content shrinks the widest column, not every column equally.
    #[test]
    fn a_narrow_terminal_shrinks_the_widest_column() {
        let rows = vec![owned(&["short", "a-very-long-value-here"])];
        let lines = as_table(&rows, None, None, 1, 20);
        assert_eq!(lines.len(), 1);
        assert!(
            width::display_width(&lines[0]) <= 20,
            "{:?} is {} cells",
            lines[0],
            width::display_width(&lines[0])
        );
        assert!(
            lines[0].starts_with("short"),
            "the short column survived intact"
        );
    }

    /// A missing cell is an empty one, not a panic and not a shifted row.
    #[test]
    fn a_short_row_is_padded_rather_than_shifting() {
        let rows = vec![owned(&["a", "1"]), owned(&["b"])];
        let lines = as_table(&rows, None, None, 2, 80);
        assert_eq!(lines, vec!["a  1", "b"]);
    }
}
