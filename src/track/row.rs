//! The schema: what a directory is, what a line run in it is, and every key that reaches one.
//!
//! Under SQL this was `CREATE TABLE` and a handful of `CREATE INDEX`. It is one module here for the
//! same reason the DDL was one constant — a key built in two places is a key that will one day be
//! built two ways, and the symptom of that is a range scan that quietly returns nothing.
//!
//! # Rows are tuples, so they are encoded like keys
//!
//! [`super::kv::Key`] and [`super::kv::Fields`] do the values as well as the keys. That is the
//! seam's instruction and it is the right one: `run`'s five counters are a tuple exactly as
//! `(dir_id, mode, argv)` is, the framing is self-delimiting either way, and a second encoder would
//! be a second thing to get wrong. What is wasted on a value is the ordering guarantee, which costs
//! nothing.
//!
//! Fixed-width fields come first in every row and the text last. A value that has been truncated or
//! was written by something else then fails in the integers, before anything has been allocated for
//! a string that was never there.
//!
//! # Two nulls, and how each is spelled
//!
//! SQL had `NULL`; this has sentinels, one per column, each chosen so that no real value can
//! collide with it.
//!
//! * `dir.root` and `dir.missing_since`, `run.last_status` — [`NEVER`] is `i64::MIN`, which is not
//!   an epoch second, not a dwell time and not an exit status. `waitpid` reports a status in
//!   `0..=255` and a signal death as a small negative, so the bottom of the range is free.
//! * `dir.root` is text, and a git toplevel is an absolute path — never the empty string. So an
//!   empty `root` field is "outside a repository", which is also what it decodes back to.
//!
//! # The five buckets this module writes into
//!
//! | bucket | key | value | what it answers |
//! |---|---|---|---|
//! | `Dir` | `dir_id` | [`DirRow`] | the row itself |
//! | `DirByPath` | `path` | `dir_id` | "which directory am I standing in" |
//! | `DirByBase` | `(base, dir_id)` | — | `cd rust`, as a range over the folded final component |
//! | `DirByRoot` | `(root, dir_id)` | — | every directory of one worktree |
//! | `Run` | `(dir_id, mode, argv)` | [`RunRow`] | the aggregate, and the suggestion range |
//! | `RunByArgv` | `(mode, argv, dir_id)` | — | the same question with no directory to pin it |
//!
//! The two index buckets carry no value at all. Everything they would hold is already in the key,
//! and a byte written per index entry per command is a byte the 8 MiB growth step eventually
//! charges for.

use super::kv::{Fields, Key, Span};
use std::borrow::Cow;

/// What a nullable integer holds when it is null. See the module note for why the bottom of the
/// range is safe for all three of the columns that need it.
const NEVER: i64 = i64::MIN;

/// A directory the shell has stood in.
///
/// `base` is the final component folded to lower case, computed once at write time — the decision
/// the design calls the one place it improves on zoxide, which lowercases every row on every query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DirRow {
    pub path: String,
    pub base: String,
    /// The git toplevel, or `None` outside a repository.
    pub root: Option<String>,
    pub visits: i64,
    /// Epoch seconds.
    pub last_visit: i64,
    /// Shell-milliseconds, not wall-clock: two shells sitting here for an hour record two hours.
    pub dwell_ms: i64,
    /// When the directory was first noticed to be gone, or `None` while it is still there.
    pub missing_since: Option<i64>,
}

impl DirRow {
    /// A directory nobody has walked to yet — resolved because a command needed attributing to it.
    pub fn unvisited(path: &str, base: String, root: Option<&str>) -> DirRow {
        DirRow {
            path: path.to_string(),
            base,
            root: root.map(str::to_string),
            visits: 0,
            last_visit: 0,
            dwell_ms: 0,
            missing_since: None,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        Key::with_capacity(48 + self.path.len() + self.base.len())
            .signed(self.visits)
            .signed(self.last_visit)
            .signed(self.dwell_ms)
            .signed(self.missing_since.unwrap_or(NEVER))
            .text(&self.path)
            .text(&self.base)
            .text(self.root.as_deref().unwrap_or(""))
            .done()
    }

    pub fn decode(bytes: &[u8]) -> Option<DirRow> {
        let mut fields = Fields::of(bytes);
        let visits = fields.signed()?;
        let last_visit = fields.signed()?;
        let dwell_ms = fields.signed()?;
        let missing_since = optional(fields.signed()?);
        let path = fields.text()?.into_owned();
        let base = fields.text()?.into_owned();
        let root = fields.text()?;
        Some(DirRow {
            path,
            base,
            root: (!root.is_empty()).then(|| root.into_owned()),
            visits,
            last_visit,
            dwell_ms,
            missing_since,
        })
    }
}

/// One command line, in one language, in one directory — folded over every time it was run.
///
/// This is the aggregate the whole store exists to be. A repeat is an increment, not a row, which
/// is what makes a year of typing a few megabytes rather than a log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RunRow {
    /// `cargo build`, not `sudo` — see [`super::redact::head_of`].
    pub head: String,
    pub runs: i64,
    pub fails: i64,
    /// Epoch seconds.
    pub last_at: i64,
    /// `None` when the command's exit was never observed, which is not the same as a failure.
    pub last_status: Option<i64>,
    pub total_ms: i64,
    pub max_ms: i64,
}

impl RunRow {
    /// The first execution of a line: one run, and the timing it took.
    pub fn first(head: String, status: Option<i64>, at: i64, ms: i64) -> RunRow {
        RunRow {
            head,
            runs: 1,
            fails: i64::from(status.is_some_and(|status| status != 0)),
            last_at: at,
            last_status: status,
            total_ms: ms,
            max_ms: ms,
        }
    }

    /// Fold one more execution into this row. Contract item 3, and the whole of it.
    ///
    /// `head` is taken from the newer observation rather than kept, because the redaction rules are
    /// the thing most likely to have changed between two runs of the same line and the newer answer
    /// is the one this binary stands behind.
    pub fn absorb(&mut self, next: &RunRow) {
        self.head = next.head.clone();
        self.runs += next.runs;
        self.fails += next.fails;
        self.last_at = next.last_at;
        self.last_status = next.last_status;
        self.total_ms += next.total_ms;
        self.max_ms = self.max_ms.max(next.max_ms);
    }

    /// Whether this line is worth offering back.
    ///
    /// The defect this fixes in passing: today's flat suggestion offers the newest prefix match
    /// with no idea whether it ever worked, so a typo is suggested for ever. A `last_status` of
    /// `None` is a command whose exit was never seen and is deliberately not a success.
    pub fn worked(&self) -> bool {
        self.last_status == Some(0) || self.runs > self.fails
    }

    /// The order the suggestion query takes its one row in: most net successes, then most recent.
    ///
    /// A tuple rather than a comparator so that a scan can hold the best it has seen in two `i64`s
    /// and allocate only when something beats it.
    pub fn standing(&self) -> (i64, i64) {
        (self.runs - self.fails, self.last_at)
    }

    pub fn encode(&self) -> Vec<u8> {
        Key::with_capacity(48 + self.head.len())
            .signed(self.runs)
            .signed(self.fails)
            .signed(self.last_at)
            .signed(self.last_status.unwrap_or(NEVER))
            .signed(self.total_ms)
            .signed(self.max_ms)
            .text(&self.head)
            .done()
    }

    pub fn decode(bytes: &[u8]) -> Option<RunRow> {
        let mut fields = Fields::of(bytes);
        let runs = fields.signed()?;
        let fails = fields.signed()?;
        let last_at = fields.signed()?;
        let last_status = optional(fields.signed()?);
        let total_ms = fields.signed()?;
        let max_ms = fields.signed()?;
        Some(RunRow {
            head: fields.text()?.into_owned(),
            runs,
            fails,
            last_at,
            last_status,
            total_ms,
            max_ms,
        })
    }
}

/// A stored integer read back as the nullable column it stands for.
fn optional(stored: i64) -> Option<i64> {
    (stored != NEVER).then_some(stored)
}

/// The keys, one builder per bucket. Nothing outside this module writes a `Key`.
pub(super) mod key {
    use super::*;

    /// `Dir`: the row for a directory.
    pub fn dir(id: u64) -> Vec<u8> {
        Key::with_capacity(8).int(id).done()
    }

    /// `DirByPath`: the unique lookup. The path as the shell spells it, not folded — two
    /// directories differing only in case are two directories.
    pub fn by_path(path: &str) -> Vec<u8> {
        Key::with_capacity(path.len() + 2).text(path).done()
    }

    /// `DirByBase`: the folded final component, then the directory it belongs to.
    pub fn by_base(base: &str, id: u64) -> Vec<u8> {
        Key::with_capacity(base.len() + 10)
            .text(base)
            .int(id)
            .done()
    }

    /// `DirByRoot`: the worktree, then a directory inside it.
    pub fn by_root(root: &str, id: u64) -> Vec<u8> {
        Key::with_capacity(root.len() + 10)
            .text(root)
            .int(id)
            .done()
    }

    /// `Run`: the aggregate's unique key, contract item 3.
    pub fn run(dir: u64, mode: &str, argv: &str) -> Vec<u8> {
        Key::with_capacity(argv.len() + mode.len() + 12)
            .int(dir)
            .text(mode)
            .text(argv)
            .done()
    }

    /// `RunByArgv`: the secondary index, contract item 2. The same three fields, rotated so that
    /// the line rather than the directory drives the range.
    pub fn by_argv(mode: &str, argv: &str, dir: u64) -> Vec<u8> {
        Key::with_capacity(argv.len() + mode.len() + 12)
            .text(mode)
            .text(argv)
            .int(dir)
            .done()
    }

    /// The `RunByArgv` key naming the same row as a `Run` key.
    ///
    /// The two indexes hold the same three fields in a different order, so one is derivable from
    /// the other and a delete never has to be handed both. That is what keeps the cascade in
    /// [`super::super::prune`] honest: it works from the keys it is already walking.
    pub fn twin_of_run(run: &[u8]) -> Option<Vec<u8>> {
        let mut fields = Fields::of(run);
        let dir = fields.int()?;
        let mode = fields.text()?;
        let argv = fields.text()?;
        Some(by_argv(&mode, &argv, dir))
    }

    /// `Meta`: the schema version and the prune stamp, by name.
    pub fn meta(name: &str) -> Vec<u8> {
        Key::with_capacity(name.len() + 2).text(name).done()
    }

    /// A bare integer, as a `Meta` value and as what `DirByPath` points at.
    pub fn id(id: u64) -> Vec<u8> {
        Key::with_capacity(8).int(id).done()
    }

    /// Read one back.
    pub fn id_of(bytes: &[u8]) -> Option<u64> {
        Fields::of(bytes).int()
    }

    /// A bare signed integer, which is every `Meta` value: a schema version, a prune stamp, the
    /// next directory id to hand out.
    pub fn number(value: i64) -> Vec<u8> {
        Key::with_capacity(8).signed(value).done()
    }

    pub fn number_of(bytes: &[u8]) -> Option<i64> {
        Fields::of(bytes).signed()
    }
}

/// The ranges, one per question the store is asked. Every one is a seek and a walk to a bound;
/// none is a scan with a filter.
pub(super) mod span {
    use super::*;

    /// Every line ever run in one directory. Contract item 4 — the cascade is a delete over this.
    pub fn runs_of(dir: u64) -> Span {
        Span::prefix(key::dir(dir))
    }

    /// Contract item 1: what was run *here*, in this language, starting with what has been typed.
    ///
    /// An empty `typed` names every line in the directory for that language, which is what a
    /// caller counting rows wants and is why the same builder serves both.
    pub fn runs_like(dir: u64, mode: &str, typed: &str) -> Span {
        Span::prefix(
            Key::with_capacity(typed.len() + mode.len() + 12)
                .int(dir)
                .text(mode)
                .text_prefix(typed)
                .done(),
        )
    }

    /// Contract item 1: the directories whose folded final component starts with `needle`.
    ///
    /// Both indexed tiers at once — naming a directory exactly is also naming its own prefix — and
    /// which of the two a row earned is decided against the path in [`super::super::score`], so
    /// that the tier and the ordering are read off the same text in the same place.
    pub fn bases_like(needle: &str) -> Span {
        Span::prefix(Key::with_capacity(needle.len()).text_prefix(needle).done())
    }

    /// Contract item 2, first half: every directory in one worktree.
    pub fn dirs_of_root(root: &str) -> Span {
        Span::prefix(Key::with_capacity(root.len() + 2).text(root).done())
    }

    /// Contract item 2, second half: every line starting with what was typed, in any directory.
    pub fn argv_like(mode: &str, typed: &str) -> Span {
        Span::prefix(
            Key::with_capacity(typed.len() + mode.len() + 4)
                .text(mode)
                .text_prefix(typed)
                .done(),
        )
    }
}

/// The fields of an index key, read back out of the key itself — which is where they live, because
/// an index row has no value.
pub(super) mod field {
    use super::*;

    /// `Dir`'s own key, and the first field of every `run` key: the directory.
    pub fn leading_id(key: &[u8]) -> Option<u64> {
        Fields::of(key).int()
    }

    /// `(base, dir_id)` and `(root, dir_id)` are the same shape: the id is what a scan wants.
    pub fn trailing_id(key: &[u8]) -> Option<u64> {
        let mut fields = Fields::of(key);
        fields.blob()?;
        fields.int()
    }

    /// `(dir_id, mode, argv)` -> the line, borrowed from the mapped file wherever it holds no NUL.
    pub fn argv_of_run(key: &[u8]) -> Option<Cow<'_, str>> {
        let mut fields = Fields::of(key);
        fields.int()?;
        fields.blob()?;
        fields.text()
    }

    /// `(mode, argv, dir_id)` -> the line and the directory it was run in.
    pub fn argv_and_dir(key: &[u8]) -> Option<(Cow<'_, str>, u64)> {
        let mut fields = Fields::of(key);
        fields.blob()?;
        let argv = fields.text()?;
        Some((argv, fields.int()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_directory() -> DirRow {
        DirRow {
            path: "/w/Alpha".to_string(),
            base: "alpha".to_string(),
            root: Some("/w".to_string()),
            visits: 7,
            last_visit: 1_700_000_000,
            dwell_ms: 91_000,
            missing_since: None,
        }
    }

    #[test]
    fn a_directory_reads_back_as_the_row_it_was_written_from() {
        let row = a_directory();
        assert_eq!(DirRow::decode(&row.encode()), Some(row));
    }

    /// The two nulls, which SQL spelled and this has to encode. Neither may come back as a zero,
    /// because a `missing_since` of zero is January 1970 and a `root` of `""` is a directory named
    /// nothing.
    #[test]
    fn a_null_column_reads_back_as_absent_rather_than_as_zero() {
        let mut row = a_directory();
        row.root = None;
        row.missing_since = None;
        let back = DirRow::decode(&row.encode()).expect("it decodes");
        assert_eq!(back.root, None, "outside a repository, not the root itself");
        assert_eq!(back.missing_since, None, "present, not missing since 1970");

        row.missing_since = Some(0);
        assert_eq!(
            DirRow::decode(&row.encode())
                .expect("it decodes")
                .missing_since,
            Some(0),
            "and a real zero is still a real zero"
        );

        let never = RunRow::first("cargo".to_string(), None, 5, 1);
        let back = RunRow::decode(&never.encode()).expect("it decodes");
        assert_eq!(
            back.last_status, None,
            "an exit nobody saw is not an exit 0"
        );
        assert_eq!(back.fails, 0, "and it is not a failure either");
    }

    /// Contract item 3: repeats are the entire point and cost nothing after the first.
    #[test]
    fn absorbing_a_run_adds_the_counters_and_keeps_the_worst_time() {
        let mut row = RunRow::first("cargo build".to_string(), Some(0), 100, 30);
        row.absorb(&RunRow::first("cargo build".to_string(), Some(1), 200, 90));
        row.absorb(&RunRow::first("cargo build".to_string(), Some(0), 300, 10));

        assert_eq!(row.runs, 3);
        assert_eq!(row.fails, 1);
        assert_eq!(row.last_at, 300, "the newest wins");
        assert_eq!(row.last_status, Some(0), "and so does its status");
        assert_eq!(row.total_ms, 130, "time adds up");
        assert_eq!(row.max_ms, 90, "and the worst of it is remembered");
        assert_eq!(RunRow::decode(&row.encode()), Some(row));
    }

    /// A line that has only ever failed is not a suggestion, and a line whose exit was never
    /// observed is not a success.
    #[test]
    fn a_line_that_never_worked_is_not_one_to_offer_back() {
        let mut failed = RunRow::first("carg".to_string(), Some(127), 1, 1);
        assert!(!failed.worked());
        failed.absorb(&RunRow::first("carg".to_string(), Some(0), 2, 1));
        assert!(failed.worked(), "it works now, so it is a command again");

        let unseen = RunRow::first("sleep".to_string(), None, 1, 1);
        assert!(
            unseen.worked(),
            "never seen to fail either — one run, no failures"
        );
    }

    /// The keys the contract is written in, decoded back out of themselves. An index row has no
    /// value, so if this is wrong the scan has nothing else to fall back on.
    #[test]
    fn an_index_key_carries_everything_a_scan_needs_to_read_off_it() {
        assert_eq!(field::trailing_id(&key::by_base("rust", 42)), Some(42));
        assert_eq!(field::trailing_id(&key::by_root("/w/beta", 9)), Some(9));
        assert_eq!(
            field::argv_of_run(&key::run(7, "sh", "cargo test")).as_deref(),
            Some("cargo test")
        );
        let rotated = key::by_argv("sh", "cargo test", 7);
        let (argv, dir) = field::argv_and_dir(&rotated).expect("it decodes");
        assert_eq!((argv.as_ref(), dir), ("cargo test", 7));

        // Bytes from somewhere else decode to nothing rather than to a plausible id.
        assert_eq!(field::trailing_id(b"not a key"), None);
        assert_eq!(field::argv_of_run(b"short"), None);
    }

    /// The ranges, against the keys they have to hold and the neighbours they must not. This is
    /// contract item 1's boundary in the smallest form it can be stated in.
    #[test]
    fn a_range_holds_its_own_rows_and_stops_at_its_neighbours() {
        let here = span::runs_like(7, "sh", "cargo te");
        assert!(here.holds(&key::run(7, "sh", "cargo test")));
        assert!(here.holds(&key::run(7, "sh", "cargo te")));
        assert!(!here.holds(&key::run(7, "sh", "cargo t")));
        assert!(!here.holds(&key::run(7, "sh", "cargo tf")));
        assert!(!here.holds(&key::run(7, "lua", "cargo test")));
        assert!(!here.holds(&key::run(6, "sh", "cargo test")));
        assert!(!here.holds(&key::run(8, "sh", "cargo test")));

        // The cascade's range: one directory, every language, every line.
        let all = span::runs_of(7);
        assert!(all.holds(&key::run(7, "lua", "print(1)")));
        assert!(!all.holds(&key::run(8, "sh", "cargo test")));

        // Naming a directory: the exact name and every extension of it, and nothing that merely
        // contains it — `prust` is a weaker tier and a different question.
        let named = span::bases_like("rust");
        assert!(named.holds(&key::by_base("rust", 1)));
        assert!(named.holds(&key::by_base("rustlings", 1)));
        assert!(!named.holds(&key::by_base("prust", 1)));
        assert!(!named.holds(&key::by_base("rus", 1)));

        // One worktree, and not the one whose path extends it.
        let inside = span::dirs_of_root("/w/beta");
        assert!(inside.holds(&key::by_root("/w/beta", 3)));
        assert!(!inside.holds(&key::by_root("/w/beta-old", 3)));

        let anywhere = span::argv_like("sh", "cargo te");
        assert!(anywhere.holds(&key::by_argv("sh", "cargo test", 99)));
        assert!(!anywhere.holds(&key::by_argv("lua", "cargo test", 99)));
        assert!(!anywhere.holds(&key::by_argv("sh", "cargo t", 99)));
    }

    /// An empty prefix is every line in the directory rather than none, which is what the cap and
    /// the counter both ask for.
    #[test]
    fn an_empty_prefix_names_the_whole_directory() {
        let span = span::runs_like(7, "sh", "");
        assert!(span.holds(&key::run(7, "sh", "")));
        assert!(span.holds(&key::run(7, "sh", "anything at all")));
        assert!(!span.holds(&key::run(7, "lua", "anything at all")));
    }

    /// A row this module did not write is refused rather than decoded into something plausible —
    /// which is the same rule the seam applies to the file as a whole.
    #[test]
    fn a_row_from_somewhere_else_decodes_to_nothing() {
        assert_eq!(DirRow::decode(b""), None);
        assert_eq!(DirRow::decode(b"far too short"), None);
        assert_eq!(RunRow::decode(&a_directory().encode()), None);
        // Enough integers, no text: the head was never there.
        assert_eq!(RunRow::decode(&[0u8; 48]), None);
    }
}
