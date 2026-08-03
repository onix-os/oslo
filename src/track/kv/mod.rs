//! Every `jammdb` call oslo makes, and nothing else.
//!
//! **This module is a seam.** Nothing outside `src/track/kv/` may `use jammdb`, and that is not
//! tidiness. jammdb was picked after twelve stores were measured; `redb` and `sanakirja` were both
//! real candidates and either could win a rerun, and jammdb itself is one maintainer at 0.11. The
//! engine is therefore held at one boundary so that moving again is a day of rewriting this
//! directory rather than a week of chasing `Data::KeyValue` through the shell. If you find
//! yourself wanting a `Tx` in `write.rs`, add a method here instead.
//!
//! # One file, and no handle held open
//!
//! This is the one place where the plan changed under measurement, so it is stated first.
//!
//! jammdb was chosen for multi-process access — oslo is a shell, several terminals are open at
//! once, and every one of them uses the store. What `DB::open` actually does is take a **blocking
//! exclusive `flock`** and hold it for the life of the `DB` value (`db.rs:247`, `fs4::lock_exclusive`,
//! `flock(LOCK_EX)`). Measured here: a second process opening the same file blocked for 3.49 s and
//! returned the moment the first dropped its handle. A shell that held the handle for its lifetime
//! would therefore hang every *other* terminal at startup, for ever — worse than `redb`, which at
//! least refuses with an error a shell can degrade on.
//!
//! So the store holds no handle. [`Store`] is a path and a promise about the file; every
//! [`Store::read`] and [`Store::write`] opens, works and closes, and the `flock` becomes what it
//! should have been all along — the mutual exclusion between terminals, taken for microseconds and
//! released by the kernel even on `kill -9`.
//!
//! The cost of that was the thing to check, and it is affordable. Measured on a release build
//! against 25,000 rows, median of two hundred, through the API below rather than around it:
//!
//! ```text
//! whole read    open + tx + prefix range + close    15.1 us
//! whole write   open + tx + put + commit + close    27.9 us     (the commit is an fsync)
//! ```
//!
//! Broken down, the open-and-mmap is 11 us of the read and the range scan itself is 5 us; with the
//! handle held the scan alone is 1.5 us, and that 13.6 us of difference is the whole price of
//! several terminals working. turso answered the same question in 13 us with its handle held open
//! and its 13.3 MB of engine, so this is not a regression at the prompt either.
//!
//! Under contention: three processes writing flat out committed 900 rows to one file with none
//! lost, worst single operation 1.3 ms. The rule that buys is worth writing down for whoever adds
//! the prune sweep — **no operation may hold a transaction open for long**, because every other
//! terminal's next keystroke is queued behind it. A sweep over every directory belongs in chunks,
//! not in one write.
//!
//! # A read answers nothing rather than failing
//!
//! `super::db`'s rule, unchanged: a shell whose tracker will not open is a working shell with a
//! dumber `cd`. Every method here answers `Option`, and every failure — a busy file, a corrupt
//! page, a bucket that does not exist yet — is `None`. There is no caller that could act on more.
//!
//! # Panics are caught, because jammdb's are not errors
//!
//! Measured on 0.11.0: `DB::open` **panics** rather than erroring on every file that is not a
//! database, and a truncated one opens cleanly and panics on the read. `super::kv::file`
//! refuses the foreseeable cases by reading the header first; the `catch_unwind` around each
//! operation is for the rest. A panic here is a store that answers `None` — never a shell that
//! stops.

mod file;
mod key;
mod range;

pub use file::is_a_database;
pub use key::{Fields, Key};
pub use range::{Span, upper_bound};

use jammdb::{Data, OpenOptions, Tx};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

/// The buckets the store keeps, and the only names jammdb is ever given.
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
}

impl Tree {
    /// The name jammdb knows it by. Kept short: it is stored in the file once per bucket.
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
        }
    }

    /// Every bucket, for a sweep or a migration that has to visit all of them.
    pub fn all() -> [Tree; 8] {
        [
            Tree::Dir,
            Tree::DirByPath,
            Tree::DirByBase,
            Tree::DirByRoot,
            Tree::Run,
            Tree::RunByArgv,
            Tree::Meta,
            Tree::History,
        ]
    }
}

/// The most bytes of rows one transaction may delete, and the reason it is a quarter of a page.
///
/// jammdb merges a node the moment it holds less than `pagesize / 4` (`node.rs`, `needs_merging`),
/// so every node a transaction *starts* with holds at least that much — and a transaction that
/// removes less than that from a bucket therefore cannot be the one that empties a node. Emptying a
/// node is the thing that panics; see [`Store::delete_span_in_chunks`].
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

/// An open store: a path, a directory that is closed to everybody else, and a file that has been
/// checked to be one of ours.
///
/// Holds no file descriptor, no lock and no memory map — see the module note. Cheap to clone, and
/// safe to keep for the life of the shell precisely *because* it holds nothing.
#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
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
        let store = Store {
            path: path.to_path_buf(),
        };
        // Open once here so that a file jammdb refuses is refused now, at startup, rather than on
        // the first keystroke — and so that there is a file to make private, which `DB::open`
        // creates whether or not anything is written through it. A *read* transaction on purpose:
        // committing an empty write one would cost every shell an `fsync` at startup to prove a
        // thing the open has already proved.
        store.enter(false, |_| Some(()))?;
        file::make_private(path)?;
        Some(store)
    }

    /// Where the store is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How large the file has grown, in bytes.
    ///
    /// Worth watching rather than ignoring: jammdb allocates in 8 MiB steps and never gives one
    /// back, so a store that outgrows its initial thirty-two pages jumps from 128 KiB to 8.5 MiB
    /// and stays there. That is what makes the per-directory cap load-bearing rather than tidy.
    pub fn size(&self) -> u64 {
        std::fs::metadata(&self.path)
            .map(|meta| meta.len())
            .unwrap_or(0)
    }

    /// Read from the store.
    ///
    /// The transaction is opened, `work` is run against it and it is dropped. A read transaction is
    /// never committed — there is nothing to commit — so a `None` from `work` costs nothing but the
    /// open.
    pub fn read<T>(&self, work: impl FnOnce(&Reader<'_, '_>) -> Option<T>) -> Option<T> {
        self.enter(false, |tx| {
            let reader = Reader { tx: &tx };
            work(&reader)
        })
    }

    /// Write to the store, committing when `work` answers `Some` and discarding when it answers
    /// `None`.
    ///
    /// This exists so that no caller writes `db.tx(true)` and forgets the commit. The whole of
    /// `work` is one transaction: a crash between two `put`s cannot leave a run attributed to a
    /// directory whose arrival was never recorded, which is the property the SQL version got from
    /// `BEGIN`/`COMMIT`.
    ///
    /// A discarded transaction has written nothing — jammdb writes no page until `commit`, so
    /// dropping is the rollback.
    pub fn write<T>(&self, work: impl FnOnce(&Writer<'_, '_>) -> Option<T>) -> Option<T> {
        self.enter(true, |tx| {
            let done = {
                let writer = Writer(Reader { tx: &tx });
                work(&writer)
            };
            let value = done?;
            tx.commit().ok().map(|()| value)
        })
    }

    /// Remove every row in `span`, a small chunk to a transaction, and answer how many went.
    ///
    /// **Use this rather than [`Writer::delete_span`] for anything that is not a handful of rows.**
    /// The reason is a defect in jammdb 0.11 that costs a silent no-op rather than a crash, which
    /// is the worst way for a bound to fail — measured here, not read anywhere:
    ///
    /// ```text
    /// 3,500 rows, delete the last 100 in one transaction   -> panic, nothing deleted
    /// 3,500 rows, 25 at a time                             -> all 100 deleted
    /// 10,100 rows, delete the last 100 in one transaction  -> panic, nothing deleted
    /// 10,100 rows, 25 at a time                            -> all 100 deleted
    /// ```
    ///
    /// The panic is `node.rs:412`, `first_key` on an empty node. `merge_nodes` refuses to merge a
    /// node away when its parent holds exactly one branch (`bucket.rs:875`, `if branches.len() == 1
    /// { continue }`), so a leaf that a transaction emptied outright survives to `spill`, which
    /// sorts children by their first key and finds it has none. It needs a bucket deep enough to
    /// have a branch of branches, which is why it never shows up in a small test — and
    /// `Store::enter`'s `catch_unwind` turns it into `None`, so the caller sees a delete that
    /// simply did not happen.
    ///
    /// The avoidance is exact rather than superstitious: a node jammdb has not already merged holds
    /// at least a quarter of a page (`node.rs`, `needs_merging`), so a transaction that removes less
    /// than that cannot be the one that empties a node. `DELETE_BUDGET` is that quarter page and
    /// the chunk is measured in bytes rather than rows, because a history line can be a paragraph
    /// and twenty of those are not twenty of anything else.
    ///
    /// Letting go between chunks is the module note's other rule kept as well: a cascade over a
    /// directory with thousands of runs is many short locks instead of one long one.
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
    /// Same budget, same defect, same rule: a transaction that removes less than a quarter page
    /// from a bucket cannot be the one that empties a node, and emptying a node is what silently
    /// throws the whole transaction away. Keys this store never held cost their overhead against
    /// the budget and nothing else, which is the safe direction to be wrong in.
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

    /// Open the file, run one transaction against it and close everything.
    ///
    /// `work` is handed the transaction by value so that it can `commit`, which consumes it. The
    /// `catch_unwind` is the module note's last paragraph: jammdb panics where it should error, and
    /// a panicking read must cost a suggestion rather than the shell. Nothing is left half-written
    /// by one, because a transaction that never reached `commit` never wrote a page.
    fn enter<T>(&self, writable: bool, work: impl FnOnce(Tx<'_>) -> Option<T>) -> Option<T> {
        catch_unwind(AssertUnwindSafe(|| {
            let db = OpenOptions::new()
                .pagesize(file::PAGE_SIZE)
                .open(&self.path)
                .ok()?;
            let tx = db.tx(writable).ok()?;
            work(tx)
        }))
        .ok()
        .flatten()
    }
}

/// A transaction that can be read.
///
/// The lifetimes are jammdb's and callers never name them: `'r` is the borrow of the transaction,
/// `'tx` the transaction's own.
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
    /// Both are taken by value rather than borrowed, which is jammdb's requirement and not a
    /// choice: `Bucket::put` wants bytes that outlive the transaction, so a `&[u8]` pointing at a
    /// local does not compile. Building the key with [`Key`] already produces the `Vec` this
    /// wants.
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
    /// **Bounded spans only — a few rows, and never a whole index.** A transaction that empties a
    /// leaf node outright panics inside jammdb and deletes nothing at all, which is measured and
    /// explained on [`Store::delete_span_in_chunks`]. That is the one to use for a cascade or a
    /// sweep; this one is for the two or three rows a single write knows it is replacing.
    ///
    /// Two passes on purpose: the keys are collected first and deleted afterwards, because
    /// deleting from under a cursor that is still walking is not a supported thing to do.
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
