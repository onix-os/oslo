//! The Tagdata boundary for tracking storage.
//!
//! Tracking failures return `None`, leaving the shell usable without history or ranking. Database
//! transactions stay inside this module and long deletions are split into short write transactions.

mod file;
mod key;
mod range;

pub use file::is_a_database;
pub use key::{Fields, Key};
pub use range::{Span, upper_bound};

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use tagdata::{DB, Data, OpenOptions, Tx};

/// The buckets in the tracking database.
///
/// An enum rather than a string at the call site: a typo in `"run"` is not a compile error, it is
/// a second bucket that is always empty, and the symptom is "it stopped suggesting anything".
/// Adding a bucket means adding a variant here — which is also the list of everything a migration
/// would have to walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tree {
    /// `dir_id -> the directory's row`. `dir_id` is an integer so that everything referring to a
    /// directory is eight fixed bytes rather than a path repeated in every key.
    Dir,
    /// `path -> dir_id`. The unique lookup behind "which directory am I standing in".
    DirByPath,
    /// `(base, dir_id) -> ()`. The range that answers `cd rust` — the folded final component,
    /// scanned by prefix.
    DirByBase,
    /// `(root, dir_id) -> ()`. The join that widens a suggestion to the whole worktree.
    DirByRoot,
    /// `(dir_id, mode, argv) -> the counters`. The aggregate, and the primary range scan.
    Run,
    /// `(mode, argv, dir_id) -> ()`. The secondary range, for the question that is not asked about
    /// one directory.
    RunByArgv,
    /// `name -> value`. The schema version and the prune stamp.
    Meta,
    /// `u64::MAX - id -> (line, mode, at)`. Appended to, and read only as "the last N".
    ///
    /// The id *descending*, so the newest line is the first row and the last N is a walk of N rows
    /// from the start of the bucket rather than a reverse cursor this seam does not have.
    /// `startup::history_db` owns the encoding.
    History,
    /// `(u64::MAX - history_id, segment) -> (join, status, duration_ms)`.
    ///
    /// What each link of `a && b || c` did, joined to the log row by id. Segment `0` is the line
    /// itself; `1..` are its links. Keyed with the *same* descending encoding as `History`, so one
    /// threshold trims both and the two can never drift apart.
    Outcome,
}

impl Tree {
    /// The persisted bucket name.
    fn name(self) -> &'static str {
        match self {
            Tree::Dir => "dir",
            Tree::DirByPath => "dir_path",
            Tree::DirByBase => "dir_base",
            Tree::DirByRoot => "dir_root",
            Tree::Run => "run",
            Tree::RunByArgv => "run_argv",
            Tree::Meta => "meta",
            Tree::History => "hist",
            Tree::Outcome => "out",
        }
    }

    /// Every bucket, for a sweep or a migration that has to visit all of them.
    pub fn all() -> [Tree; 9] {
        [
            Tree::Dir,
            Tree::DirByPath,
            Tree::DirByBase,
            Tree::DirByRoot,
            Tree::Run,
            Tree::RunByArgv,
            Tree::Meta,
            Tree::History,
            Tree::Outcome,
        ]
    }
}

/// The approximate row bytes removed in one write transaction.
const DELETE_BUDGET: usize = (file::PAGE_SIZE / 4) as usize;

/// What one row costs a leaf beyond its own key and value: `size_of::<LeafElement>()`, which is a
/// tag and three `u64`s.
const LEAF_OVERHEAD: usize = 32;

/// Whether a scan wants the next row.
///
/// The reason a scan takes a closure at all: `LIMIT 1` is the commonest read in the store, and a
/// scan that collected before it stopped would pay for every row in the range to answer with one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Walk {
    /// Keep going.
    On,
    /// That was the row I wanted.
    Stop,
}

/// An open tracking store.
#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    db: DB,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Store")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Open the store at `path`, creating it and its directory if they are not there.
    ///
    /// `None` when the directory cannot be made private, when there is something at the path that
    /// is not one of our databases, or when the engine will not have it. All three are "this shell
    /// has no store", which is a supported way to run.
    pub fn open(path: &Path) -> Option<Store> {
        file::prepare_directory(path)?;
        if !file::is_a_database(path) {
            return None;
        }
        let db = catch_unwind(AssertUnwindSafe(|| {
            OpenOptions::new().pagesize(file::PAGE_SIZE).open(path).ok()
        }))
        .ok()
        .flatten()?;
        file::make_private(path)?;
        Some(Store {
            path: path.to_path_buf(),
            db,
        })
    }

    /// Where the store is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How large the file has grown, in bytes.
    pub fn size(&self) -> u64 {
        std::fs::metadata(&self.path)
            .map(|meta| meta.len())
            .unwrap_or(0)
    }

    /// Read from the store.
    ///
    /// The transaction is opened, `work` is run against it and it is dropped. A read transaction is
    /// never committed because it changes nothing.
    pub fn read<T>(&self, work: impl FnOnce(&Reader<'_, '_>) -> Option<T>) -> Option<T> {
        let tx = self.db.tx(false).ok()?;
        catch_unwind(AssertUnwindSafe(|| work(&Reader { tx: &tx })))
            .ok()
            .flatten()
    }

    /// Write to the store, committing when `work` answers `Some` and discarding when it answers
    /// `None`.
    ///
    /// This exists so that no caller writes `db.tx(true)` and forgets the commit. The whole of
    /// `work` is one transaction: a crash between two `put`s cannot leave a run attributed to a
    /// directory whose arrival was never recorded, which is the property the SQL version got from
    /// `BEGIN`/`COMMIT`.
    ///
    /// Dropping an uncommitted transaction rolls it back.
    pub fn write<T>(&self, work: impl FnOnce(&Writer<'_, '_>) -> Option<T>) -> Option<T> {
        let tx = self.db.tx(true).ok()?;
        let value = catch_unwind(AssertUnwindSafe(|| {
            let writer = Writer(Reader { tx: &tx });
            work(&writer)
        }))
        .ok()
        .flatten()?;
        tx.commit().ok().map(|()| value)
    }

    /// Remove every row in `span`, a small chunk to a transaction, and answer how many went.
    ///
    /// Use this for long sweeps so other processes can acquire the writer between chunks.
    pub fn delete_span_in_chunks(&self, tree: Tree, span: &Span) -> usize {
        let mut gone = 0;
        let mut previous: Option<Vec<u8>> = None;
        loop {
            let Some(doomed) = self.read(|reader| {
                let mut spent = 0;
                let mut doomed = Vec::new();
                reader.scan(tree, span, |key, value| {
                    doomed.push(key.to_vec());
                    spent += LEAF_OVERHEAD + key.len() + value.len();
                    if spent >= DELETE_BUDGET {
                        Walk::Stop
                    } else {
                        Walk::On
                    }
                });
                Some(doomed)
            }) else {
                return gone;
            };
            if doomed.is_empty() {
                return gone;
            }
            // The loop is driven by the store getting smaller, so it has to notice when the store
            // does not. A committed transaction that removed nothing would otherwise be read back,
            // deleted again and committed again, for ever — and a shell that hangs is very much
            // worse than a bound that is not enforced. Seeing the same first key twice is the exact
            // statement of "no progress", and it costs one comparison a chunk.
            if previous.as_deref() == Some(doomed[0].as_slice()) {
                return gone;
            }
            previous = Some(doomed[0].clone());
            // A chunk that fails is a chunk that changed nothing, so stopping leaves the store
            // exactly as consistent as finishing would have — with more rows in it.
            if self
                .write(|writer| {
                    for key in &doomed {
                        writer.delete(tree, key);
                    }
                    Some(())
                })
                .is_none()
            {
                return gone;
            }
            gone += doomed.len();
        }
    }

    /// Remove a list of keys that is not a span, under the same budget and for the same reason.
    ///
    /// [`Store::delete_span_in_chunks`] is the one to reach for; this is its sibling for the case a
    /// range cannot express. A secondary index is the whole of that case: `run_argv` is keyed
    /// `(mode, argv, dir_id)`, so the entries belonging to one directory are scattered through it
    /// and the only way to name them is one at a time. `super::prune` derives each from the primary
    /// key it is already deleting.
    ///
    /// Missing keys still count toward the chunk budget.
    pub fn delete_keys_in_chunks(&self, tree: Tree, keys: &[Vec<u8>]) -> usize {
        let mut gone = 0;
        let mut rest = keys;
        while !rest.is_empty() {
            let mut spent = 0;
            let take = rest
                .iter()
                .take_while(|key| {
                    let first = spent == 0;
                    spent += LEAF_OVERHEAD + key.len();
                    first || spent < DELETE_BUDGET
                })
                .count();
            let (chunk, tail) = rest.split_at(take);
            rest = tail;
            // A chunk that fails changed nothing, so stopping leaves the store exactly as
            // consistent as finishing would have — with more rows in it.
            let Some(taken) = self
                .write(|writer| Some(chunk.iter().filter(|key| writer.delete(tree, key)).count()))
            else {
                return gone;
            };
            gone += taken;
        }
        gone
    }
}

/// A transaction that can be read.
///
/// `'r` is the transaction borrow and `'tx` is the engine transaction lifetime.
pub struct Reader<'r, 'tx> {
    tx: &'r Tx<'tx>,
}

impl Reader<'_, '_> {
    /// The value under `key`, or `None` — which is also the answer for a bucket no write has
    /// created yet, because an empty store and an empty bucket hold the same rows.
    pub fn get(&self, tree: Tree, key: &[u8]) -> Option<Vec<u8>> {
        let bucket = self.tx.get_bucket(tree.name()).ok()?;
        match bucket.get(key)? {
            Data::KeyValue(pair) => Some(pair.value().to_vec()),
            Data::Bucket(_) => None,
        }
    }

    /// Whether `key` is there at all, without copying its value.
    pub fn has(&self, tree: Tree, key: &[u8]) -> bool {
        self.tx
            .get_bucket(tree.name())
            .ok()
            .and_then(|bucket| bucket.get_kv(key))
            .is_some()
    }

    /// Walk `span` in key order, stopping when `visit` says so.
    ///
    /// A seek to the lower bound followed by a walk to the upper one — **not** a full scan with a
    /// filter, which is the same mistake `LIKE` was. The keys and values are borrowed from the
    /// mapped file, so a visitor that only looks at them allocates nothing.
    pub fn scan(&self, tree: Tree, span: &Span, mut visit: impl FnMut(&[u8], &[u8]) -> Walk) {
        let Ok(bucket) = self.tx.get_bucket(tree.name()) else {
            return;
        };
        for found in bucket.range(span.bounds()) {
            if let Data::KeyValue(pair) = found
                && visit(pair.key(), pair.value()) == Walk::Stop
            {
                return;
            }
        }
    }

    /// The first row of `span` that `pick` answers `Some` for.
    ///
    /// The `LIMIT 1` of the store: it stops at the row it takes rather than reading the range.
    pub fn find<T>(
        &self,
        tree: Tree,
        span: &Span,
        mut pick: impl FnMut(&[u8], &[u8]) -> Option<T>,
    ) -> Option<T> {
        let mut found = None;
        self.scan(tree, span, |key, value| match pick(key, value) {
            Some(taken) => {
                found = Some(taken);
                Walk::Stop
            }
            None => Walk::On,
        });
        found
    }

    /// Everything in `span` that `take` answers `Some` for, in key order.
    pub fn collect<T>(
        &self,
        tree: Tree,
        span: &Span,
        mut take: impl FnMut(&[u8], &[u8]) -> Option<T>,
    ) -> Vec<T> {
        let mut found = Vec::new();
        self.scan(tree, span, |key, value| {
            if let Some(taken) = take(key, value) {
                found.push(taken);
            }
            Walk::On
        });
        found
    }

    /// How many rows are in `span`. Counted by walking it, so it is a range and not the bucket.
    pub fn count(&self, tree: Tree, span: &Span) -> usize {
        let mut rows = 0;
        self.scan(tree, span, |_, _| {
            rows += 1;
            Walk::On
        });
        rows
    }
}

/// A transaction that can also be written.
///
/// Derefs to [`Reader`], because a write path reads: the upsert that makes this store an aggregate
/// is a `get`, an add and a `put`.
pub struct Writer<'r, 'tx>(Reader<'r, 'tx>);

impl<'r, 'tx> std::ops::Deref for Writer<'r, 'tx> {
    type Target = Reader<'r, 'tx>;

    fn deref(&self) -> &Reader<'r, 'tx> {
        &self.0
    }
}

impl Writer<'_, '_> {
    /// Put `value` under `key`, creating the bucket if this is the first row in it.
    ///
    /// Keys and values are owned for the transaction lifetime.
    pub fn put(&self, tree: Tree, key: Vec<u8>, value: Vec<u8>) -> Option<()> {
        let bucket = self.0.tx.get_or_create_bucket(tree.name()).ok()?;
        bucket.put(key, value).ok()?;
        Some(())
    }

    /// Remove `key`. `false` when it was not there, which is not a failure.
    pub fn delete(&self, tree: Tree, key: &[u8]) -> bool {
        self.0
            .tx
            .get_bucket(tree.name())
            .is_ok_and(|bucket| bucket.delete(key).is_ok())
    }

    /// Remove every row in `span`, in this transaction.
    ///
    /// Keys are collected before deletion so the active range iterator is not mutated.
    pub fn delete_span(&self, tree: Tree, span: &Span) -> usize {
        let doomed: Vec<Vec<u8>> = self.0.collect(tree, span, |key, _| Some(key.to_vec()));
        let mut gone = 0;
        for key in &doomed {
            if self.delete(tree, key) {
                gone += 1;
            }
        }
        gone
    }

    /// Empty a bucket completely, by dropping it.
    ///
    /// What `history -c` and `forget_runs` need. A bucket that is not there reads as empty, so the
    /// next `put` simply makes it again.
    pub fn clear(&self, tree: Tree) -> bool {
        self.0.tx.delete_bucket(tree.name()).is_ok()
    }
}

#[cfg(test)]
mod tests;
