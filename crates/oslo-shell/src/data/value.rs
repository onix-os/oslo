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

/// Writing a value down for a person: human sizes, aligned columns, colour.
mod drawn;
/// Writing a value down for a program: plain, complete, escaped in band.
mod transport;

pub use drawn::{human_duration, human_size, human_time, render_display};
pub(crate) use drawn::{numeric_columns, one_line};
pub use transport::{render_transport, unescape_cell};

/// The scalar forms both renderers agree on.
///
/// Neither face's, which is why it lives here rather than in one of them: each handles the kinds it
/// renders differently — a size, a duration, a time, a table — and falls through to this for the
/// rest.
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

impl fmt::Display for Val {
    /// Display is the *human* rendering, which is what `{}` in a message means.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", render_display(self))
    }
}

#[cfg(test)]
#[path = "value/tests.rs"]
mod tests;
