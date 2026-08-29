//! What flows between two stages of a structured pipeline.
//!
//! **Not the Lua `Value`.** Three reasons it cannot be. It is `Rc<RefCell<Table>>`, so it is not
//! `Send` and is only valid while an interpreter is live on this thread — but a pipeline value has
//! to be constructible by a Rust builtin with no Lua anywhere in sight. Its hash part had no order
//! until recently, and a record whose columns come out shuffled is not a record. And it has no room
//! for a tagged scalar, so a size would have to be smuggled in as `{__kind="size"}`, which makes
//! every consumer defensive about a shape the producer never intended.
//!
//! See `docs/features/structured-pipelines.md` for the design this belongs to. Two things there are
//! load-bearing here:
//!
//! * **[`Val::Error`] is a value, not a failure.** `ps` meets a process that exits mid-scan; `df`
//!   meets a stale NFS mount. Text tools warn about the one row and carry on, and that is exactly
//!   why people trust them. An error that aborts the stream would make structure worse than text.
//! * **Two renderers, from the first commit.** [`render_display`] is for a person — colour, human
//!   sizes, aligned columns. [`render_transport`] is for a program — plain, untruncated, one record
//!   per line. Writing one function with a flag is how a box-drawing character ends up on the stdin
//!   of `grep`, which is nushell's most damaging bug and the thing this design exists to avoid.

use std::fmt;

/// One value in a structured pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// Bytes that are not text. Distinct from [`Val::Str`] on purpose: without it, reading a JPEG
    /// gives mojibake rather than an honest blob.
    Bytes(Vec<u8>),
    /// A byte count. Renders as `4.2G`, compares and sorts as the number it is — which is the whole
    /// point, and what `ls -lh | sort` cannot do.
    Size(u64),
    /// Nanoseconds.
    Duration(i64),
    /// Nanoseconds since the epoch.
    Time(i64),
    List(Vec<Val>),
    Record(Record),
    /// A failure in *one cell*, which the rest of the stream survives.
    Error(String),
}

/// An ordered set of named fields.
///
/// Two parallel vectors with linear lookup, not a map. Records are three to fifteen columns wide,
/// so a scan is faster than hashing, and the order is not incidental — it decides the order the
/// columns are drawn and serialised in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Record {
    names: Vec<String>,
    values: Vec<Val>,
}

impl Record {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a field, keeping its place if it already had one.
    pub fn set(&mut self, name: &str, value: Val) {
        match self.names.iter().position(|n| n == name) {
            Some(at) => self.values[at] = value,
            None => {
                self.names.push(name.to_string());
                self.values.push(value);
            }
        }
    }

    pub fn get(&self, name: &str) -> Option<&Val> {
        self.names
            .iter()
            .position(|n| n == name)
            .map(|at| &self.values[at])
    }

    pub fn columns(&self) -> &[String] {
        &self.names
    }

    pub fn values(&self) -> &[Val] {
        &self.values
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Build one from pairs, which is what a tool writing a row actually wants to say.
    pub fn from_pairs<I, S>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (S, Val)>,
        S: AsRef<str>,
    {
        let mut record = Record::new();
        for (name, value) in pairs {
            record.set(name.as_ref(), value);
        }
        record
    }
}

impl Val {
    /// A table is a list of records. There is no fourth type, deliberately: one shape fewer to
    /// think about at every step, and nushell's separate table type earns nothing that this does
    /// not.
    pub fn table(rows: Vec<Record>) -> Val {
        Val::List(rows.into_iter().map(Val::Record).collect())
    }

    /// The columns a list of records has, in the order they first appear.
    ///
    /// Rows are allowed to disagree — `ps` may know a command line for one process and not another
    /// — so the header is the union, not the first row's fields.
    pub fn columns(&self) -> Vec<String> {
        let Val::List(items) = self else {
            return match self {
                Val::Record(r) => r.columns().to_vec(),
                _ => Vec::new(),
            };
        };
        let mut out: Vec<String> = Vec::new();
        for item in items {
            if let Val::Record(record) = item {
                for name in record.columns() {
                    if !out.iter().any(|existing| existing == name) {
                        out.push(name.clone());
                    }
                }
            }
        }
        out
    }

    /// Whether this is something a person would want drawn as a table.
    pub fn is_table(&self) -> bool {
        matches!(self, Val::List(items) if !items.is_empty()
            && items.iter().all(|i| matches!(i, Val::Record(_))))
    }
}

/// A value as a program should read it: plain, complete, one record per line.
///
/// No colour, no borders, no truncation, no abbreviation. A size is its number of bytes, because
/// the program on the other end will do arithmetic on it and `4.2G` is not a number.
///
/// **A cell is escaped, because the separators are in band.** Records are separated by a newline
/// and cells by a tab, so a cell that contains either used to break the framing silently: one row
/// arrived as two, and every column after the tab shifted by one. That made `to text` lossy for
/// exactly the data a shell meets most — a filename with a tab in it, a `cmdline` spanning lines —
/// and it corrupted every hand-over into a byte suffix, which is rendered the same way.
///
/// See [`escape_cell`] for the form. Nothing *un*escapes on the way back in: `lines` and `parse`
/// read arbitrary bytes from programs that never heard of oslo, and a backslash in their output is
/// a backslash.
pub fn render_transport(value: &Val) -> String {
    match value {
        Val::List(items) => items
            .iter()
            .map(render_transport)
            .collect::<Vec<_>>()
            .join("\n"),
        Val::Record(record) => record
            .values()
            .iter()
            .map(|cell| escape_cell(&render_transport(cell)))
            .collect::<Vec<_>>()
            .join("\t"),
        Val::Size(bytes) => bytes.to_string(),
        Val::Duration(ns) => ns.to_string(),
        Val::Time(ns) => ns.to_string(),
        other => scalar(other),
    }
}

/// A cell with the separators spelled rather than written.
///
/// `\` first, or unescaping could not tell `\t` the two characters from `\t` the tab. A nested list
/// or record inside a cell is rendered by [`render_transport`] with its own newlines and tabs, and
/// this catches those too — which is why it is applied to the rendered cell rather than to the
/// string inside it.
fn escape_cell(text: &str) -> String {
    if !text.contains(['\\', '\t', '\n', '\r']) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

/// The inverse of [`escape_cell`], for a reader that knows it is reading oslo's own transport.
///
/// Deliberately **not** applied by `lines` or `parse`: those read whatever a program wrote, and a
/// program that emits a literal backslash means one.
pub fn unescape_cell(text: &str) -> String {
    if !text.contains('\\') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            // Not a form this wrote: keep both characters rather than eat the backslash.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

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

/// The scalar forms both renderers agree on.
fn scalar(value: &Val) -> String {
    match value {
        Val::Null => String::new(),
        Val::Bool(b) => b.to_string(),
        Val::Int(i) => i.to_string(),
        Val::Float(f) => f.to_string(),
        Val::Str(s) => s.clone(),
        // Never the bytes themselves: a renderer's output is text, and a JPEG is not.
        Val::Bytes(b) => format!("<{} bytes>", b.len()),
        Val::Error(message) => format!("<error: {message}>"),
        other => render_transport(other),
    }
}

/// A table with a header and aligned columns.
fn table_display(value: &Val) -> String {
    let columns = value.columns();
    let Val::List(items) = value else {
        return String::new();
    };
    let cells: Vec<Vec<String>> = items
        .iter()
        .map(|item| {
            let Val::Record(record) = item else {
                return Vec::new();
            };
            columns
                .iter()
                .map(|name| record.get(name).map(render_display).unwrap_or_default())
                .collect()
        })
        .collect();

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

    let mut out = String::new();
    for (i, name) in columns.iter().enumerate() {
        if i > 0 {
            out.push_str("  ");
        }
        out.push_str(&pad(name, widths[i]));
    }
    out.push('\n');
    for row in &cells {
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                out.push_str("  ");
            }
            out.push_str(&pad(cell, widths[i]));
        }
        out.push('\n');
    }
    out.trim_end().to_string()
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
use oslo_ui::dropdown::display_width;

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

impl fmt::Display for Val {
    /// Display is the *human* rendering, which is what `{}` in a message means.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", render_display(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pairs: &[(&str, Val)]) -> Record {
        Record::from_pairs(pairs.iter().map(|(n, v)| (*n, v.clone())))
    }

    /// Columns keep the order they were set in, and setting one again does not move it.
    #[test]
    fn a_record_is_ordered() {
        let mut r = Record::new();
        r.set("filesystem", Val::Str("/dev/sda1".into()));
        r.set("size", Val::Size(1024));
        r.set("used", Val::Size(512));
        assert_eq!(r.columns(), ["filesystem", "size", "used"]);
        r.set("filesystem", Val::Str("/dev/sdb1".into()));
        assert_eq!(r.columns(), ["filesystem", "size", "used"], "no reordering");
        assert_eq!(r.get("filesystem"), Some(&Val::Str("/dev/sdb1".into())));
    }

    /// Rows may disagree about their columns, so the header is the union of them.
    #[test]
    fn a_tables_columns_are_the_union_of_its_rows() {
        let table = Val::table(vec![
            row(&[("pid", Val::Int(1)), ("cmd", Val::Str("init".into()))]),
            row(&[("pid", Val::Int(2)), ("user", Val::Str("bo".into()))]),
        ]);
        assert_eq!(table.columns(), ["pid", "cmd", "user"]);
        assert!(table.is_table());
    }

    /// **The invariant the design rests on**: transport is plain and complete, display is for a
    /// person. A size reaches a program as a number it can do arithmetic on.
    #[test]
    fn the_two_renderings_are_different_functions() {
        let value = Val::Size(4_509_715_660);
        assert_eq!(render_display(&value), "4.2G");
        assert_eq!(render_transport(&value), "4509715660");

        let table = Val::table(vec![row(&[
            ("name", Val::Str("root".into())),
            ("free", Val::Size(2048)),
        ])]);
        let display = render_display(&table);
        assert!(display.contains("name"), "display has a header: {display}");
        assert!(display.contains("2.0K"), "display humanises: {display}");

        let transport = render_transport(&table);
        assert_eq!(transport, "root\t2048", "transport is plain and complete");
        assert!(!transport.contains("2.0K"));
    }

    /// An error is one cell's problem. The row it sits in still arrives, and so do the others.
    #[test]
    fn an_error_is_a_value_and_does_not_stop_the_stream() {
        let table = Val::table(vec![
            row(&[("mount", Val::Str("/".into())), ("free", Val::Size(100))]),
            row(&[
                ("mount", Val::Str("/nfs".into())),
                ("free", Val::Error("stale handle".into())),
            ]),
            row(&[("mount", Val::Str("/tmp".into())), ("free", Val::Size(200))]),
        ]);
        let Val::List(rows) = &table else {
            panic!("a table is a list");
        };
        assert_eq!(rows.len(), 3, "every row survives");
        assert!(render_display(&table).contains("stale handle"));
    }

    /// Durations read the way every other tool writes them. Sizes are the dropdown's now, and
    /// tested where they live.
    #[test]
    fn durations_are_readable() {
        assert_eq!(human_duration(340_000_000), "340ms");
        assert_eq!(human_duration(1_500_000_000), "1.5s");
        assert_eq!(human_duration(150_000_000_000), "2m30s");
    }

    /// **The separators are in band, so a cell that holds one is spelled rather than written.**
    ///
    /// A tab in a cell used to end that cell and shift every column after it; a newline ended the
    /// record and made one row arrive as two. Both silently, on exactly the data a shell meets —
    /// a filename with a tab, a `cmdline` spanning lines.
    #[test]
    fn a_cell_holding_a_separator_does_not_break_the_framing() {
        let table = Val::table(vec![row(&[
            ("name", Val::Str("two\twords".into())),
            ("note", Val::Str("first\nsecond".into())),
        ])]);
        let transport = render_transport(&table);
        assert_eq!(
            transport.lines().count(),
            1,
            "one record is one line: {transport:?}"
        );
        assert_eq!(transport, "two\\twords\tfirst\\nsecond");
        assert_eq!(
            transport.matches('\t').count(),
            1,
            "exactly one cell separator"
        );
    }

    /// A backslash goes first, or reading back could not tell `\t` the two characters from a tab.
    #[test]
    fn escaping_round_trips() {
        for original in [
            "plain",
            "a\tb",
            "a\nb",
            "back\\slash",
            "already\\tspelled",
            "\r\n",
            "",
        ] {
            let table = Val::table(vec![row(&[("c", Val::Str(original.into()))])]);
            assert_eq!(
                unescape_cell(&render_transport(&table)),
                original,
                "{original:?} did not survive"
            );
        }
    }

    /// A nested cell renders with its own separators, and those are caught too — the escape is
    /// applied to the rendered cell, not to the string inside it.
    #[test]
    fn a_nested_cell_cannot_break_the_framing() {
        let table = Val::table(vec![row(&[
            ("id", Val::Int(1)),
            (
                "tags",
                Val::List(vec![Val::Str("a".into()), Val::Str("b".into())]),
            ),
        ])]);
        let transport = render_transport(&table);
        assert_eq!(transport.lines().count(), 1, "got {transport:?}");
        assert_eq!(transport, "1\ta\\nb");
    }

    /// A column is terminal cells, not characters: a CJK ideograph is two cells wide and a table
    /// that counted it as one drew its columns out of line.
    #[test]
    fn a_wide_column_lines_up() {
        let table = Val::table(vec![
            row(&[("name", Val::Str("名前".into())), ("n", Val::Int(1))]),
            row(&[("name", Val::Str("ab".into())), ("n", Val::Int(2))]),
        ]);
        let drawn = render_display(&table);
        // **Cells, not bytes.** `名前` is six bytes and four columns, so a byte offset would call
        // these rows misaligned when they are drawn correctly — the same mistake `pad` was making.
        let starts: Vec<usize> = drawn
            .lines()
            .filter_map(|line| line.find(['1', '2']).map(|at| display_width(&line[..at])))
            .collect();
        assert_eq!(starts.len(), 2, "two data rows: {drawn}");
        assert_eq!(
            starts[0], starts[1],
            "the second column starts in the same cell on both rows:\n{drawn}"
        );
    }

    /// Bytes are never rendered as text: a JPEG through `render_display` is a description, not
    /// mojibake.
    #[test]
    fn binary_is_described_rather_than_mangled() {
        let value = Val::Bytes(vec![0xff, 0xd8, 0xff, 0xe0]);
        assert_eq!(render_display(&value), "<4 bytes>");
    }
}
