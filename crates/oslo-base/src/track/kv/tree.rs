//! The bucket schema: what a key-value database of oslo's holds, and under which name.
//!
//! Its own file because it is a *schema*, and the one thing a schema must be is easy to read whole.
//! `mod.rs` next door is the engine seam — opening, transactions, scanning — and mixing the two
//! meant the list of buckets sat in the middle of the code that walks them.

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
    /// Stable portable history events shared between databases.
    SyncEvent,
    /// Local history ids mapped to stable event ids.
    HistoryEvent,
    /// Stable event ids mapped to local projection state.
    EventProjection,
    /// `key -> value`, for a database that belongs to a config or a plugin rather than to oslo.
    ///
    /// **One bucket is enough because one such database is one file.** `oslo.db.open(name)` gives
    /// each caller its own store, so there is nothing to keep apart inside it — and nothing here
    /// interprets what it holds, unlike every tree above.
    Plugin,
}

impl Tree {
    /// The persisted bucket name.
    pub(super) fn name(self) -> &'static str {
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
            Tree::SyncEvent => "sync_event",
            Tree::HistoryEvent => "hist_event",
            Tree::EventProjection => "event_projection",
            Tree::Plugin => "plugin",
        }
    }

    /// Every bucket of the *tracking* store, for a sweep or a migration that has to visit all of
    /// them.
    ///
    /// [`Tree::Plugin`] is deliberately absent: it never appears in the same file as these, and a
    /// sweep that looked for it in the tracking store would be looking for a bucket that is not
    /// there by design.
    pub fn all() -> [Tree; 12] {
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
            Tree::SyncEvent,
            Tree::HistoryEvent,
            Tree::EventProjection,
        ]
    }
}
