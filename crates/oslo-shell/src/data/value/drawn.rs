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
                    Some(value) => cell(&render_display(value), drawn.max_column),
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
    let room = terminal_cols();
    let mut out = String::new();
    let mut line = String::new();
    let mut write = |cells: &mut dyn Iterator<Item = (usize, &String)>, out: &mut String| {
        line.clear();
        for (i, cell) in cells {
            if i > 0 {
                line.push_str("  ");
            }
            line.push_str(&pad(cell, widths[i]));
        }
        out.push_str(&clamp(line.trim_end(), room));
        out.push('\n');
    };
    write(&mut columns.iter().enumerate(), &mut out);
    for row in &cells {
        write(&mut row.iter().enumerate(), &mut out);
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
