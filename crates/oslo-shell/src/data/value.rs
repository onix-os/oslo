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
//!   `df` is where one actually comes from: `df -P` writes `-` for every figure of a mount it
//!   cannot reach, and that row used to be dropped — so `df | length` under-counted and the mount
//!   worth looking at was the only one missing.
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

    /// A field to write into, for [`crate::data::path::Path::set`] descending into a nested one.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Val> {
        self.names
            .iter()
            .position(|n| n == name)
            .map(|at| &mut self.values[at])
    }

    pub fn columns(&self) -> &[String] {
        &self.names
    }

    /// Drop a field, closing the gap. Answers whether there was one.
    pub fn remove(&mut self, name: &str) -> bool {
        match self.names.iter().position(|n| n == name) {
            Some(at) => {
                self.names.remove(at);
                self.values.remove(at);
                true
            }
            None => false,
        }
    }

    /// Give a field a new name **in its own place**, because a record's order is not incidental —
    /// a rename that moved the column to the end would silently reorder the drawn table.
    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        match self.names.iter().position(|n| n == from) {
            Some(at) => {
                self.names[at] = to.to_string();
                true
            }
            None => false,
        }
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
fn cell(text: &str, room: usize) -> String {
    match room {
        0 => text.to_string(),
        room => clamp(text, room),
    }
}

/// A line cut to `room` terminal cells, with an ellipsis where it was cut.
///
/// The marker matters: a silently truncated table looks like data that ends there, and the whole
/// argument for two renderers is that a person can tell what they are looking at.
fn clamp(line: &str, room: usize) -> String {
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

    /// **The drawn table reads process-wide settings, and one test writes them.**
    ///
    /// Tests run in parallel, so without this the settings test's `max_column = 8` was visible to
    /// every other test that draws a table — for as long as it held them. The same guard `plan.rs`
    /// puts around its edge counter, for the same reason: shared mutable state needs the tests that
    /// touch it to take turns.
    static DRAWN: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn drawing() -> std::sync::MutexGuard<'static, ()> {
        DRAWN.lock().unwrap_or_else(|e| e.into_inner())
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
        let _turn = drawing();
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
        let _turn = drawing();
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
        let _turn = drawing();
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

    /// **A time has a human face too.** It rendered as its raw nanosecond count in *both* faces, so
    /// the tagged kind that makes `where 'modified > 2days'` arithmetic also made the column
    /// unreadable — the exact trade `Val::Size` exists to avoid.
    #[test]
    fn a_time_reads_as_a_date_and_transports_as_a_number() {
        let _turn = drawing();
        // 2019-03-05, comfortably outside the six-month window, so the year is shown.
        let old = Val::Time(1_551_744_000_000_000_000);
        let drawn = render_display(&old);
        assert!(
            drawn.contains("2019"),
            "an old time shows the year: {drawn}"
        );
        assert!(!drawn.contains("1551744"), "not the raw count: {drawn}");
        assert_eq!(
            render_transport(&old),
            "1551744000000000000",
            "a program gets the number to compute with"
        );

        // Recent shows the hour instead, which is what distinguishes two files from today.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let recent = render_display(&Val::Time(now * 1_000_000_000));
        assert!(
            recent.contains(':'),
            "a recent time shows the hour: {recent}"
        );
    }

    /// **A row is one line.** Without a clamp a wide table wraps every row across two or three
    /// terminal lines and the columns stop lining up at all. The marker matters as much as the cut:
    /// a silently truncated table looks like data that ends there.
    #[test]
    fn a_wide_line_is_cut_with_a_marker() {
        let _turn = drawing();
        assert_eq!(clamp("short", 20), "short");
        assert_eq!(clamp("exactly-ten", 11), "exactly-ten", "fits exactly");
        let cut = clamp("a rather long line indeed", 10);
        assert_eq!(display_width(&cut), 10, "cut to the room there is");
        assert!(cut.ends_with('…'), "and says it was cut: {cut:?}");
        // A width of nothing is a terminal that would not say, so nothing is cut.
        assert_eq!(clamp("untouched", 0), "untouched");
    }

    /// **`oslo.table` is the drawn face, and only the drawn face.**
    ///
    /// The two renderers are two functions so that a preference cannot reach another program's
    /// standard input. This is the test that says so: every setting here changes the table and none
    /// of them changes the transport.
    #[test]
    fn the_drawn_table_is_configurable_and_the_transport_is_not() {
        let _turn = drawing();
        // The settings are process-wide, so this test owns them for its duration and puts them back.
        let restore = oslo_ui::settings::current().as_ref().clone();
        let table = Val::table(vec![
            row(&[
                ("n", Val::Int(1)),
                ("long", Val::Str("abcdefghijklmnop".into())),
            ]),
            row(&[("n", Val::Int(2)), ("missing", Val::Null)]),
        ]);
        let before = render_transport(&table);

        let mut settings = restore.clone();
        settings.table.index = true;
        settings.table.null = "-".to_string();
        settings.table.max_column = 8;
        oslo_ui::settings::install(settings);

        let drawn = render_display(&table);
        assert!(drawn.starts_with('#'), "an index column leads: {drawn}");
        assert!(
            drawn.contains('-'),
            "a null shows as the null text: {drawn}"
        );
        assert!(
            drawn.contains('…') && !drawn.contains("abcdefghijklmnop"),
            "a wide cell is cut at max_column: {drawn}"
        );

        assert_eq!(
            render_transport(&table),
            before,
            "not one of those settings may reach the transport"
        );
        assert!(render_transport(&table).contains("abcdefghijklmnop"));

        oslo_ui::settings::install(restore);
    }

    /// `max_column = 0` is how "no limit" is spelled, and the default leaves ordinary cells alone.
    #[test]
    fn a_cell_is_only_cut_when_it_is_too_wide() {
        let _turn = drawing();
        assert_eq!(cell("short", 60), "short");
        assert_eq!(
            cell("untouched by a limit of none", 0),
            "untouched by a limit of none"
        );
        let cut = cell("abcdefghijklmnop", 8);
        assert_eq!(display_width(&cut), 8, "cut to the room there is: {cut:?}");
        assert!(cut.ends_with('…'));
        assert_eq!(cut.matches('…').count(), 1, "exactly one ellipsis: {cut:?}");
    }

    /// Transport is never truncated: the program on the other end asked for all of it.
    #[test]
    fn transport_is_never_clamped() {
        let wide = Val::table(vec![row(&[(
            "c",
            Val::Str("a very long value that no terminal is this wide for, by some way".into()),
        )])]);
        assert!(render_transport(&wide).contains("by some way"));
        assert!(!render_transport(&wide).contains('…'));
    }

    /// Bytes are never rendered as text: a JPEG through `render_display` is a description, not
    /// mojibake.
    #[test]
    fn binary_is_described_rather_than_mangled() {
        let value = Val::Bytes(vec![0xff, 0xd8, 0xff, 0xe0]);
        assert_eq!(render_display(&value), "<4 bytes>");
    }
}
