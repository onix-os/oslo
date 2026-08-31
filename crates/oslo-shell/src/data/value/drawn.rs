//! The drawn face: what a person reads.

use super::*;

/// A value as a person should read it.
///
/// Sizes become `4.2G`, durations become `1.5s`, and a table gets aligned columns with a header.
/// Nothing here may ever be written to a pipe — see the module docs.
pub fn render_display(value: &Val) -> String {
    if value.is_table() {
        return table_display(value);
    }
    match value {
        Val::Size(bytes) => human_size(*bytes),
        Val::Duration(ns) => human_duration(*ns),
        Val::Time(ns) => human_time(*ns),
        Val::List(items) => items
            .iter()
            .map(render_display)
            .collect::<Vec<_>>()
            .join("\n"),
        Val::Record(record) => record
            .columns()
            .iter()
            .zip(record.values())
            .map(|(name, value)| format!("{name}: {}", render_display(value)))
            .collect::<Vec<_>>()
            .join("\n"),
        other => scalar(other),
    }
}

/// A table with a header and aligned columns.
fn table_display(value: &Val) -> String {
    // `oslo.table` — the drawn face only. Nothing read here may reach `render_transport`, which is
    // what another program sees; the two renderers are two functions for exactly that reason.
    let drawn = oslo_ui::settings::current().table.clone();
    let mut columns = value.columns();
    let Val::List(items) = value else {
        return String::new();
    };
    let mut cells: Vec<Vec<String>> = items
        .iter()
        .map(|item| {
            let Val::Record(record) = item else {
                return Vec::new();
            };
            columns
                .iter()
                .map(|name| match record.get(name) {
                    // An absent cell and a null one read the same to a person: there is nothing
                    // there. `describe` is where the difference is asked about.
                    None | Some(Val::Null) => drawn.null.clone(),
                    Some(value) => cell(&one_line(value), drawn.max_column),
                })
                .collect()
        })
        .collect();

    // A leading column of row numbers, for reading `first`/`skip` positions off the table. It is
    // drawn rather than inserted into the rows: `enumerate` is the verb for a column that survives.
    if drawn.index {
        columns.insert(0, "#".to_string());
        for (at, row) in cells.iter_mut().enumerate() {
            row.insert(0, at.to_string());
        }
    }

    let widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(i, name)| {
            cells
                .iter()
                .filter_map(|row| row.get(i))
                .map(|c| display_width(c))
                .chain(std::iter::once(display_width(name)))
                .max()
                .unwrap_or(0)
        })
        .collect();

    // **A row is one line, or the table stops being one.** Without a clamp a wide table wraps every
    // row across two or three terminal lines, and the columns a person was reading down stop lining
    // up at all — `ps | first 20` on an eighty-column terminal is unreadable rather than merely
    // wide. The width is asked for once, here, and only for the drawn face: `render_transport` is
    // never truncated, because the program on the other end asked for all of it.
    // **A column of numbers reads down its last digit.** Left-aligned, `9` and `2315` start in the
    // same place and end four apart, so comparing two rows means reading rather than glancing. The
    // decision is per *column* and not per cell: one text value in a column of numbers makes the
    // whole column text, because a column that changed alignment half way down would be worse than
    // either choice.
    let numeric: Vec<bool> = (0..columns.len())
        .map(|i| {
            let mut any = false;
            for row in &cells {
                match row.get(i) {
                    // A blank stands for nothing, not for text: one null in a column of numbers
                    // must not un-align the rest of it.
                    Some(cell) if cell.is_empty() || cell == &drawn.null => {}
                    Some(cell) if reads_as_a_number(cell) => any = true,
                    Some(_) => return false,
                    None => {}
                }
            }
            any
        })
        .collect();

    let room = terminal_cols();
    let mut out = String::new();
    let rule = |left: &str, mid: &str, right: &str, fill: &str, out: &mut String| {
        let mut line = String::from(left);
        for (i, width) in widths.iter().enumerate() {
            if i > 0 {
                line.push_str(mid);
            }
            line.push_str(&fill.repeat(width + 2));
        }
        line.push_str(right);
        out.push_str(&clamp(&line, room));
        out.push('\n');
    };
    let write = |cells: &mut dyn Iterator<Item = (usize, &String)>,
                 edge: Option<&str>,
                 out: &mut String| {
        let mut line = String::new();
        for (i, cell) in cells {
            match edge {
                // Bordered: a rule at each boundary, and one blank column either side of the text.
                Some(bar) => {
                    line.push_str(bar);
                    line.push(' ');
                }
                // Borderless: the two-space gutter this has always drawn.
                None if i > 0 => line.push_str("  "),
                None => {}
            }
            match numeric[i] {
                true => line.push_str(&pad_left(cell, widths[i])),
                false => line.push_str(&pad(cell, widths[i])),
            }
            if edge.is_some() {
                line.push(' ');
            }
        }
        if let Some(bar) = edge {
            line.push_str(bar);
        }
        let line = match edge {
            // A bordered row keeps its closing rule; only the borderless form has trailing padding
            // worth removing.
            Some(_) => line,
            None => line.trim_end().to_string(),
        };
        out.push_str(&clamp(&line, room));
        out.push('\n');
    };

    match (drawn.border.glyphs(), drawn.border.junctions()) {
        (Some(g), Some(j)) => {
            let (h, v) = (g[4], g[5]);
            rule(g[0], j[0], g[1], h, &mut out);
            write(&mut columns.iter().enumerate(), Some(v), &mut out);
            rule(g[6], j[2], g[7], h, &mut out);
            for row in &cells {
                write(&mut row.iter().enumerate(), Some(v), &mut out);
            }
            rule(g[2], j[1], g[3], h, &mut out);
        }
        _ => {
            write(&mut columns.iter().enumerate(), None, &mut out);
            for row in &cells {
                write(&mut row.iter().enumerate(), None, &mut out);
            }
        }
    }
    out.trim_end().to_string()
}

/// One cell, cut to `room` terminal cells if it is wider.
///
/// A `cmdline` is a hundred characters and would squeeze every other column off the row. The whole
/// *line* is clamped separately; this is what stops a single column owning it. `0` is no limit.
pub(super) fn cell(text: &str, room: usize) -> String {
    match room {
        0 => text.to_string(),
        room => clamp(text, room),
    }
}

/// A line cut to `room` terminal cells, with an ellipsis where it was cut.
///
/// The marker matters: a silently truncated table looks like data that ends there, and the whole
/// argument for two renderers is that a person can tell what they are looking at.
pub(super) fn clamp(line: &str, room: usize) -> String {
    if room == 0 {
        return line.to_string();
    }
    // `truncate_to_width` reserves a cell and appends the ellipsis itself — adding one here made
    // every cut end in two of them.
    truncate_to_width(line, room)
}

/// Pad to `width` **terminal cells**, not characters.
///
/// `chars().count()` is not a column: a CJK ideograph occupies two cells and a combining mark
/// none, so a table with either in it drew its columns out of line. This is the dropdown's own
/// measure — the same one the line editor uses — so the three cannot disagree about how wide
/// something is.
fn pad(text: &str, width: usize) -> String {
    let mut out = text.to_string();
    for _ in display_width(text)..width {
        out.push(' ');
    }
    out
}

/// `4.2G`, the way every tool that reports sizes writes it.
///
/// The dropdown's, because a `Val::Size` in a table and a size column in a completion menu are the
/// same number for the same reader — and the two copies of this were identical to the digit.
pub use oslo_ui::dropdown::human_size;

/// How many terminal cells a string occupies — the dropdown's measure, so the drawn table, the
/// completion menu and the line editor all agree about what a column is worth.
use oslo_ui::dropdown::{display_width, terminal_cols, truncate_to_width};

/// A point in time as a person reads one.
///
/// **Recent is a time, older is a date**, which is the rule `ls -l` has used for forty years and
/// for the same reason: within the last six months the hour is what distinguishes two files, and
/// beyond it the year is.
///
/// A `Val::Time` used to render as its raw nanosecond count in *both* faces — the tagged kind
/// existed, and nothing gave it one. So the type that makes `where 'modified > 2days'` arithmetic
/// also made the column unreadable, which is the exact trade `Val::Size` exists to avoid.
pub fn human_time(nanos: i64) -> String {
    let seconds = nanos.div_euclid(1_000_000_000);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Six months, the same window `ls` uses. Ahead of now counts as recent too: a file with a
    // timestamp in the future is worth showing the hour of.
    let recent = (now - seconds).abs() < 182 * 24 * 60 * 60;
    let format = match recent {
        true => "%b %e %H:%M",
        false => "%b %e  %Y",
    };
    oslo_base::clock::at(seconds, format)
}

/// `1.5s`, `2m30s`, `340ms` — whichever unit makes the number readable.
pub fn human_duration(nanos: i64) -> String {
    let ms = nanos / 1_000_000;
    if ms < 1000 {
        return format!("{ms}ms");
    }
    let secs = ms as f64 / 1000.0;
    if secs < 60.0 {
        return format!("{secs:.1}s");
    }
    let whole = secs as i64;
    format!("{}m{:02}s", whole / 60, whole % 60)
}

/// The mirror of [`pad`], for a column of numbers.
fn pad_left(text: &str, width: usize) -> String {
    let mut out = String::new();
    for _ in display_width(text)..width {
        out.push(' ');
    }
    out.push_str(text);
    out
}

/// Whether a drawn cell is a number a person would compare down a column.
///
/// **Read from the rendering, not from the kind**, and that is deliberate: `4.2G` is a `Val::Size`
/// and `2m30s` a `Val::Duration`, and both are things you scan down a column looking for the biggest
/// — so both count, even though neither parses as a number. What must not count is a path or a
/// command line that happens to begin with a digit, which is why a leading digit alone is not enough
/// and a space anywhere disqualifies.
pub(super) fn reads_as_a_number(text: &str) -> bool {
    let mut seen_digit = false;
    for c in text.chars() {
        match c {
            '0'..='9' => seen_digit = true,
            // The punctuation a rendered quantity carries: a decimal point, a sign, a thousands
            // separator, a percent, and the unit letters `human_size` and `human_duration` append.
            '.' | '-' | '+' | ',' | '%' => {}
            _ if seen_digit && c.is_ascii_alphabetic() => {}
            _ => return false,
        }
    }
    seen_digit
}

/// One cell of a drawn table, on one line — because a row that is two lines is not a row.
///
/// A nested value is **described rather than spelled out**. `render_display` gives a list a line
/// per item and a record a line per field, which is right when one is printed on its own and fatal
/// inside a column: a `tags` cell holding three items pushed two extra physical lines into the
/// table and every column below it stopped lining up. `<3 items>` is the same shape `Val::Bytes`
/// already uses for a value a person cannot read in place — and `flatten`, `get` and `to json` are
/// how you reach what is inside it.
fn one_line(value: &Val) -> String {
    match value {
        Val::List(items) => counted(items.len(), "item"),
        Val::Record(record) => counted(record.columns().len(), "field"),
        other => fold(&render_display(other)),
    }
}

/// The three characters in a string that would move the cursor instead of drawing, spelled the way
/// `render_transport` spells them.
///
/// A filename really can hold a newline, and a `cmdline` really can hold a tab. Backslashes are
/// **not** doubled, unlike in the transport: that escaping exists so the bytes can be read back,
/// and nothing reads the drawn face back — doubling them here would only put a character on the
/// screen that is not in the data.
fn fold(text: &str) -> String {
    match text.contains(['\n', '\r', '\t']) {
        false => text.to_string(),
        true => text
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t"),
    }
}

/// `<1 item>`, `<3 items>` — the plural agrees, because `<1 fields>` reads like a bug in the shell.
fn counted(n: usize, noun: &str) -> String {
    match n {
        1 => format!("<1 {noun}>"),
        n => format!("<{n} {noun}s>"),
    }
}
