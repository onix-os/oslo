# A smarter cd, and the store behind it

Four agents studied zoxide, deja, the wider ecosystem (atuin, mcfly, autojump, fasd) and the
schema question; a fifth turned the findings into this. It is the design for oslo's own frecency
`cd` and for the store that feeds it — which is meant to feed other things too, so the shape
matters more than the feature.

## What the studies found

**zoxide** — Studied zoxide at source level (cloned the current `main` of src/db/{mod,dir,stream}.rs,
  src/config.rs, src/util.rs, src/cmd/{add,query,edit}.rs, templates/{posix,zsh}.txt, README,
  the Algorithm wiki, and the issue tracker), and read the oslo machinery it would land in
  (history_db.rs, frecency_store.rs + spec/frecency.rs, recall.rs x2,
  directories/{cd,chdir,ring,stack,dirs}.rs, repl.rs, lua/api/shell.rs, data/value.rs).  The
  short version of zoxide: **rank is a raw click counter, not a decayed score.** Every visit
  does `rank += 1.0` and stamps `last_accessed = now`. The only decay is a four-step step
  function applied at *query* time (x4 under an hour, x2 under a day, x0.5 under a week, x0.25
  beyond), and a global rescale — not a per-entry half-life — that fires only when the sum of
  all ranks crosses `_ZO_MAXAGE` (10000): every rank is multiplied by `0.9 * max_age / total`,
  and anything that lands under 1.0 is dropped. There is no half-life anywhere in the
  codebase. Matching is a reverse substring walk over the lowercased whole path string, with
  the sole structural rule that nothing after the last keyword's match may contain a path
  separator. The current directory is excluded by the *shell function*, passing `--exclude
  "$(pwd)"`, compared as an exact string.  For oslo: zoxide's score model is strictly weaker
  than what oslo already has in `spec/frecency.rs` (a continuous `count / (1 + ln(1 +
  age_hours))`), and zoxide's four loudest open complaints (#247, #260, #929, #956 —
  exact/close matches losing to frequent partial matches, no use of the query string in
  ranking, no preference for the subtree you are in) are all things a per-directory, per-tool
  store keyed the way the owner described would fix for free. Details, arithmetic and
  citations below.

**deja** — I cloned and read deja in full (Go, ~5.3k LOC, MIT, at commit 58cf131 / v0.4.0) plus its issue
  tracker, and read the oslo machinery you named. Deja is **not** a zoxide — it does no
  directory jumping at all. It is a zsh autosuggestion engine that ranks whole command lines
  by a four-signal blend (fuzzy + frecency + directory affinity + bigram sequence). Two of its
  four ideas are exactly what the owner asked for, and both survive without a daemon. The
  daemon is pure IPC amortisation for a shell that cannot hold state — its own issue #80
  measures 29.6 ms/keystroke of which almost all is fork+exec of a 7 MB binary. oslo *is* the
  daemon: `State` (in-memory stats + dirCounts + seqByPrev behind an RWMutex) is a
  `Mutex<...>` in a long-lived process, which oslo already has three of. Nothing in deja needs
  a background process except cross-terminal cache coherence, which oslo should solve by not
  caching aggressively rather than by adding one. Deja records `exit_code`, `duration_ms` and
  `session_id` and never reads any of them; the owner's dwell-time and per-tool-duration ideas
  go beyond anything deja does, and dwell time in particular is a trap that needs an idle cap.
  The concrete recommendation is: one new turso store keyed by `(line, mode, dir)` rather than
  deja's global `command_stats` + N+1 dir aggregate; a `cd` that tries POSIX first and only
  gets clever when POSIX has already failed *and* the shell is interactive; and a ring reduced
  to a plain bounded deque once `prevd`/`nextd` go.

**ecosystem** — oslo already computes, and then discards, every single field this feature needs. In
  `src/startup/repl.rs` one iteration of the REPL loop knows the command text (:167), the
  language (:180-183), whether it is secret (:176), the directory before (:199), the directory
  after (:237), the wall-clock duration (:200,:244) and the exit status (:254). Only `line`
  and `mode` reach a table (`history_db.rs:64-68`). The proposal is therefore not "add
  telemetry" but "stop throwing it away": one new turso database in the *library* (not
  `src/startup/`, which is binary-only — `main.rs:4` vs `lib.rs:15-24`, and `builtin_cd` must
  be able to query it), written from the four call sites that already exist, with no daemon
  and no new crate.  Two things follow that the whole zoxide/autojump/fasd/z/jump/pazi family
  cannot do. First, dwell time: every directory entry and exit happens inside the REPL loop,
  so time-in-directory is exact arithmetic on `Instant`s at `repl.rs:237-244` — no polling, no
  daemon. None of those tools weight by it; they all count arrivals, so a directory you passed
  through for three seconds ranks equal to one you worked in all afternoon. Second, per-
  directory command suggestion, which is atuin's and mcfly's ground and where oslo should take
  mcfly's line (directory as a *term in the score*, not a filter) and atuin's git-root
  workspace filter (`cwd LIKE git_root%`) — that filter is precisely what makes the owner's
  `cargo run --example xyz` example work from a subdirectory.  On the cd ladder:
  `src/expand/sugar.rs:63` is the exact precedent for the POSIX gate (interactive-only,
  unresolved name left untouched), `interactive::prompt::git_root()` at `prompt.rs:39-48`
  already implements `cd root` with no `git` subprocess, and `oslo.dirs`/`@name`
  (`sugar.rs:69-71`, `settings.rs:27`) is already wd's warp points. The owner is right about
  prevd/nextd, and removing them also removes the `back` cursor that is the only reason
  `ring.rs` is complicated — but `cd -1` and `cd -` are not equivalent today, which is a live
  bug (finding 7).

**schema** — I read the existing machinery and then measured turso 0.7.2 directly (six throwaway probe
  suites, since the SQL surface of a SQLite rewrite is not something to assume). The headline
  recommendation is that the new store must be an **aggregate, not an event log** — three
  tables, `dir` / `run` / `meta`, keyed on `(dir_id, argv)`.  Three measured facts force that
  shape: 1. **turso 0.7.2 cannot VACUUM** (`VACUUM is an experimental feature`; `auto_vacuum`
  likewise refused). The file is a permanent high-water mark, so it must never blow up in the
  first place. 2. **history.db is already the event log.** `history(line, mode, at)`
  (history_db.rs:64-68) records every command line with a timestamp and language. A second log
  doubles the write cost and doubles the privacy surface for nothing. 3. All five questions
  the owner asked are answerable from aggregates, and I proved it — I built the proposed
  schema, ran all five queries, and confirmed every one is index-backed via `EXPLAIN QUERY
  PLAN`.  The aggregate is bounded by *distinct behaviour* rather than by time, which is
  simultaneously the answer to "how large after a year" (~4 MB/year, reaching steady state
  under pruning, vs ~11 MB/year unbounded for a log) and to "what is pruned" (almost nothing
  needs to be).  On dwell time with no daemon: **never store an open interval.** The open
  interval lives in the shell process as one epoch mark; only closed intervals are written,
  flushed at every command boundary rather than only at `cd`. A `kill -9` or power loss then
  costs at most the time since the last command and can never leave a corrupt half-row — the
  crash-safety problem is designed out rather than handled.  Measured release-mode costs: **81
  µs** to write one command (three statements, one transaction, against a
  3000-directory/25000-row database), **13 µs** for the in-directory suggestion, **1.07 ms**
  for the zoxide-style ranking scan, **53 µs** to open. Storage **194 bytes per distinct
  row**.

## The schema

ONE STORE: `$XDG_DATA_HOME/oslo/track.db`, beside `history.db`, opened by the same `turso` +
`OnceLock<Runtime>` pattern as `src/startup/history_db.rs:52-61`. Zero new dependencies
(Cargo.toml:60-61 already has turso 0.7.2 and tokio rt+macros).

IT IS AN AGGREGATE, NOT AN EVENT LOG. Three reasons, in order of force: (a) `history.db` is
already the event log — `history(line, mode, at)` at history_db.rs:64-68. A second log doubles
the write cost and doubles the privacy surface for a chronology that already exists. (b) turso
0.7.2 has no VACUUM and no auto_vacuum (both refused as experimental). A log's file size is a
permanent high-water mark; an aggregate is bounded by *distinct behaviour*, not by time. (c)
Every question the owner asked is an aggregate question. What this gives up is the exact order
of individual executions. That is recoverable by joining `history.db` on `(line, at)` if
anything ever needs it.

WHERE IT LIVES IN THE TREE: `src/track/`, a new `pub mod track` in lib.rs. NOT `src/startup/`.
`src/main.rs:4` declares `mod startup;` private to the binary, and `builtin_cd`
(src/env/builtins/directories/cd.rs) is library code — it cannot reach `history_db.rs` today and
it must be able to reach this. The handle is a process-global `static TRACK:
OnceLock<Option<Track>>` installed by `repl.rs` only, exactly like
`exec::pipeline::set_interactive` and `autocd::AUTOCD` (autocd.rs:21, whose comment gives the
reasoning: "a property of the invocation"). A script never installs it, so `track::get()` is
`None` and every clever path is *structurally* dead in a script rather than gated by a
remembered `if`.

THE SQL, in full:

    PRAGMA foreign_keys = ON;
    PRAGMA busy_timeout = 250;     -- small on purpose; see write path

    CREATE TABLE IF NOT EXISTS meta (
        key   TEXT PRIMARY KEY,
        value INTEGER NOT NULL);            -- 'schema', 'last_prune'

    CREATE TABLE IF NOT EXISTS dir (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        path          TEXT    NOT NULL UNIQUE,
        base          TEXT    NOT NULL,     -- lowercased basename; this is the match key
        root          TEXT,                 -- git toplevel, or NULL
        visits        INTEGER NOT NULL DEFAULT 0,
        last_visit    INTEGER NOT NULL DEFAULT 0,   -- epoch seconds, as history_db.rs:95-98
        dwell_ms      INTEGER NOT NULL DEFAULT 0,
        missing_since INTEGER);             -- NULL = present on disk

    CREATE INDEX IF NOT EXISTS dir_base ON dir(base);
    CREATE INDEX IF NOT EXISTS dir_root ON dir(root);

    CREATE TABLE IF NOT EXISTS run (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        dir_id      INTEGER NOT NULL REFERENCES dir(id) ON DELETE CASCADE,
        mode        TEXT    NOT NULL,       -- history_db::MODE_SHELL / MODE_LUA
        argv        TEXT    NOT NULL,       -- the line as typed, or just `head` when redacted
        head        TEXT    NOT NULL,       -- 'cargo build', not 'sudo'
        runs        INTEGER NOT NULL DEFAULT 0,
        fails       INTEGER NOT NULL DEFAULT 0,
        last_at     INTEGER NOT NULL DEFAULT 0,
        last_status INTEGER,                -- NULL = never observed finishing
        total_ms    INTEGER NOT NULL DEFAULT 0,
        max_ms      INTEGER NOT NULL DEFAULT 0);

    CREATE UNIQUE INDEX IF NOT EXISTS run_key  ON run(dir_id, mode, argv);
    CREATE INDEX        IF NOT EXISTS run_argv ON run(mode, argv);
    CREATE INDEX        IF NOT EXISTS run_head ON run(head, last_at);
    CREATE INDEX        IF NOT EXISTS run_age  ON run(last_at) WHERE runs = 1;

FOUR DECISIONS INSIDE THAT SCHEMA WORTH DEFENDING:

1. `run` is keyed on `(dir_id, mode, argv)` — the whole line, not the command word. The owner's own example is `cargo run --example xyz` here versus `--example abc` there; keyed on `head` alone the two are indistinguishable. `mode` is in the key for the reason history_db.rs:3-6 gives: a Lua line and a shell line are not alternatives for the same slot, and `print(1)` is valid text in both.

2. `head` is denormalised alongside `argv` so "what does cargo cost me, everywhere" is one index-backed read (`run_head`) instead of a string-split over a 2 KB column at query time. `head` is atuin's `interesting_command`, not `argv.split(' ').next()`: strip leading `VAR=value` assignments, strip wrapper words (`sudo`, `doas`, `env`, `time`, `nice`), and keep two words when the second is a known subcommand verb. `sudo cargo build x` groups as `cargo build`. Grouping everything a user does under `sudo` makes the per-tool timing table a joke, and grouping `cargo build` with `cargo test` makes it useless.

3. `dir.base` with an index is the one place I improve on zoxide rather than copy it. zoxide's matcher is a full scan with an allocation per row (`util::to_lowercase` inside `filter_by_keywords`, db/stream.rs:86). An indexed lowercased basename makes exact- and prefix-of-basename O(log n) and is what lets the match cascade in the frecency section exist at all.

4. `dir.root` is the git toplevel, computed by `interactive::prompt::git_root()` (prompt.rs:39-48) — already written, walks up for `.git`, no `git` subprocess, and correct for worktrees because `.git` there is a file and `.exists()` does not care. One walk-up per directory *change*, not per command. It pays for `cd root`, for the workspace-widened suggestion, and for a future "jump between projects".

Versioning: `PRAGMA user_version` (verified to work on turso 0.7.2) plus a mirrored
`meta.schema` row so the file is legible by hand — the same argument history_db.rs:27-28 makes
for storing the mode as text. This is the ONE place I deliberately depart from history_db's
convention. history_db.rs:63 says "there is no migration step", which is right for three columns
that will never change and wrong for a store the owner says other tools will use. Rules:
additive `ALTER TABLE ... ADD COLUMN` only, never destructive; a version this binary does not
understand means stop writing and keep reading, never drop-and-recreate.

Failure convention copied verbatim from history_db.rs:78-91: every failure answers `None`. A
shell whose tracker will not open is a working shell with a dumber `cd`.

## The write path

ONE SITE. `repl.rs:167-256` already computes every input within sixty lines of each other and
then throws all but two away: `text`, `mode`, `secret`  -> repl.rs:167 (`Input::Command`)
`before` (the directory)  -> repl.rs:199 `started`                 -> repl.rs:200 `after`
-> repl.rs:237 `elapsed`                 -> repl.rs:244 `res` (the status)        -> repl.rs:254
Only `line` and `mode` reach a table (repl.rs:178). So this is not "add telemetry", it is "stop
discarding it". The whole hook is one call inserted next to the `postcmd` fire at repl.rs:255,
taking a struct built from locals that already exist. Nothing is threaded through anything.

NOT the `postcmd` hook itself — that fires only on `Ok(status)` (repl.rs:254), so every failed
command would go unrecorded, and failures are exactly what the `fails` column is for.

THREE STATEMENTS, ONE TRANSACTION, measured 81 us average / 602 us worst in release against a
3000-dir / 25000-run database:

    -- (1) only when after != before, which repl.rs:238 already tests
    INSERT INTO dir (path, base, root, visits, last_visit) VALUES (?1, ?2, ?3, 1, ?4)
    ON CONFLICT(path) DO UPDATE SET
      visits = visits + 1, last_visit = excluded.last_visit,
      root = excluded.root, missing_since = NULL;

    -- (2) dwell, closing the segment that just ended
    UPDATE dir SET dwell_ms = dwell_ms + ?1 WHERE id = ?2;

    -- (3) the run, attributed to `before` — the directory the command ran in
    INSERT INTO run (dir_id, mode, argv, head, runs, fails, last_at, last_status, total_ms, max_ms)
    VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8, ?8)
    ON CONFLICT(dir_id, mode, argv) DO UPDATE SET
      runs        = runs + 1,
      fails       = fails + excluded.fails,
      last_at     = excluded.last_at,
      last_status = excluded.last_status,
      total_ms    = total_ms + excluded.total_ms,
      max_ms      = MAX(max_ms, excluded.max_ms);

Multi-column conflict targets, `excluded.*` and `MAX()` in a SET clause all work on turso 0.7.2.

THE BUG THE NAIVE VERSION HAS: statement (3) needs `dir_id` for `before`, and statement (1) only
ran if the directory *changed*. On the first command of a session, or the first command after a
`cd` into a directory whose row was pruned, that id does not exist. Do not paper over it with a
subselect that yields NULL against a NOT NULL column. The store caches `current: Option<(i64,
String)>` and resolves-or-inserts when it is `None`; `repl.rs` primes it once at startup with
`$PWD`, which is the same line that fixes the `cd -1` bug (see ring verdict).

COST PER COMMAND, honestly: 81 us. The owner types thousands a day; ten thousand commands is 0.8
seconds of database work spread across a day. It is not measurable next to the fork+exec of the
command itself. I am running it INLINE on the REPL thread, synchronously, like `db.append`
already is.

NO WORKER THREAD, NO CHANNEL, NO DAEMON. The 81 us does not justify one, and a queue introduces
a shutdown-drain problem for a store whose whole value proposition is that it survives a `kill
-9` without corruption.

Two things a naive implementation would get wrong and that I am handling instead of
parallelising around:
 * `PRAGMA busy_timeout = 250`, not 2000. Contention between two shells is measured to block a writer for exactly the timeout and then fail with "database is locked". A big timeout turns contention into a visible prompt stall; a small one turns it into one dropped sample, which for a statistics table is by far the cheaper failure. Readers are never blocked by a writer — WAL, and turso 0.7.2 is already in WAL mode without being asked, so `history.db` has been getting it for free all along.
 * Do NOT set `PRAGMA journal_mode=WAL` through `execute()`. It returns a row and `execute()` fails with "unexpected row during execution". Same trap for `wal_checkpoint`. Both go through `query()`.

A FREE WIN WHILE ADJACENT: repl.rs:187 runs `db.trim(settings.max_size)` on *every single
command*, which is `DELETE ... WHERE id NOT IN (SELECT ... LIMIT N)` (history_db.rs:157-163) — a
full scan per line. Put it behind a counter (every 100 commands, plus once at exit). That alone
probably pays for the new store's entire per-command cost.

DWELL, and the trap in it. Never store an open interval: a row with a NULL end is the thing that
cannot survive a kill. The open segment is one `SystemTime` mark held in the process; only
*closed* segments are written, as the `dwell_ms = dwell_ms + ?` increment above — an increment
computed in SQL, not a read-then-write in Rust, so two shells in one directory add up instead of
clobbering each other (the same failure `frecency_store.rs:9-12` exists to avoid). Flush at
every command boundary, not only at `cd`, so a SIGKILL costs seconds rather than hours. Cap each
contribution at 15 minutes, using the WALL clock: a laptop suspend must not record nine hours in
`~/`, and a backwards NTP step contributes zero rather than a negative. Note the unit honestly
in the column comment: this is SHELL-milliseconds, not wall-clock — two shells sitting in a
directory for an hour records two hours. That is correct for a ranking signal and wrong for a
report, and deduplicating it would need a session table and an interval-overlap computation on
read. I am not building that.

Command duration: `started.elapsed()` at repl.rs:244 is already correct and already monotonic
(`Instant`, so unaffected by NTP steps and suspend). Store milliseconds — sub-millisecond
resolution on a shell command is noise, and `total_ms` in an i64 does not overflow for 292
million years. One caveat: `elapsed` includes time a command spent stopped under Ctrl-Z, which
will poison a mean. Apply the same 15-minute cap.

PRUNE, without a background process. A `meta.last_prune` row; on open, if it is more than a day
old, hand the sweep to a detached `std::thread`. That is not a daemon and it is not new
machinery — `command_index::warm` at repl.rs:80-83 already spawns exactly this kind of one-shot
startup thread for the PATH scan, and the comment there gives the reasoning ("time the scan gets
for free"). The sweep measured 32 ms:
  1. `DELETE FROM run WHERE runs = 1 AND last_at < ?` (90 days). This is the rule that bounds the table and it is principled rather than arbitrary — a line run exactly once, months ago, is not a suggestion. It catches precisely the unbounded shapes: `git commit -m "..."`, `kill 12345`, one-off paths. The partial index `run_age` makes it a range scan, and the index is correctly maintained when `runs` goes 1 -> 2 through the upsert.
  2. `stat()` each `dir.path`; set `missing_since` on the gone, `DELETE FROM dir WHERE missing_since < ?` (30 days). Two-phase because an unmounted USB stick is not a deleted directory. `ON DELETE CASCADE` takes the runs with it; foreign keys are genuinely enforced under `PRAGMA foreign_keys=ON`.
  3. `PRAGMA wal_checkpoint(TRUNCATE)` via `query()`.
Plus one always-on bound needing no schedule: cap `run` rows per directory at 500, evicting
lowest `runs` then oldest `last_at`. That protects against one pathological directory rather
than against time.

I am explicitly NOT copying zoxide's `_ZO_MAXAGE` global rescale. Its failure is real and
documented in its own issue #292: one burst of new directories pushes the total over the
threshold and a single proportional multiply-and-prune wipes the entire recent long tail while a
stale high-rank entry survives untouched. `last_accessed` plays no part in it at all. A per-row
age rule plus an LRU cap is strictly better and costs the same.

## Frecency, and why not zoxide's

DO NOT COPY ZOXIDE. Two separate claims.

FIRST, THE CURVE. zoxide's `rank` is not a frecency score and conflating them is the single
easiest mistake here. `rank` is a monotonically increasing visit counter on disk (`dir.rank +=
1.0` in `add_update`); the frecency-looking part is `score(now) = rank * {4.0, 2.0, 0.5, 0.25}`
computed at query time from four age buckets (db/dir.rs:8-33). There is no half-life anywhere in
that codebase — the word does not appear. The buckets are discontinuous: a directory loses 4x of
its score the instant it crosses one hour old, and another 4x at one day, so the ordering of two
similar directories flips on a clock boundary for no reason a user can perceive. That is a 2011
shell-script artefact — fasd computes the same thing in awk — and oslo is not paying that
constraint.

Use the house formula, unchanged, in SQL:

    score = visits / (1.0 + ln(1.0 + (now - last_visit) / 3600.0))

That is `spec/frecency.rs:65-67` verbatim, `ln()` exists in turso, and using the *same
expression* in SQL and in Rust means the completion ranker and the `cd` jump agree about what
"best" means. That matters more than which curve it is.

For the record, normalised to a fresh visit = 1.0, the house multiplier is:
    1 hour  0.591     1 day  0.237     1 week  0.163     1 month  0.132     1 year  0.099
and zoxide's is 1.0 / 0.5 / 0.125 / 0.0625 / 0.0625. So the house curve is *flatter* in the
first day and *steeper* past a month. It is stickier than zoxide inside a working session, which
is the right bias for a shell: within an hour you want frequency to decide, not the clock.

SECOND, AND MORE IMPORTANT: the curve is not the ranking. zoxide's four loudest open complaints
are not about its curve at all — #260 "the search string should be used in determining the
score" (the keywords are a pure *filter*; `Stream::new` sorts by score and the query contributes
nothing), #956 (`prust` rank 308 beats `rust` rank 36 for the query `rust`), #247 (`z code`
prefers `code3` over `code`), #929 (a `src` in the project you are standing in loses to a `src`
you visited more last month). All four are the same defect: match quality is not in the ranking.

So the jump is a TIERED CASCADE, autojump's structure, with the house frecency as the order
*within* a tier. Query `q`, lowercased once:

    T3  dir.base = q                                  -- exact basename
    T2  dir.base LIKE q || '%'   (as a range on dir_base, not LIKE)
    T1  dir.base LIKE '%' || q || '%'
    T0  dir.path LIKE '%' || q || '%'  AND zoxide's last-keyword rule

First non-empty tier wins outright. Within a tier: `score DESC`, then `length(path) ASC` — the
shortest-path tie-break is rupa/z's rule that zoxide dropped and #956 asks to have back.

Worked, against the actual bug reports:
 * `cd rust` with prust(308 visits) and rust(36): T3 contains only `rust`. Correct. zoxide sends you to prust.
 * `cd code` with code(20) and code3(400): T3 contains only `code`. Correct. zoxide sends you to code3.
 * `cd co` with code, code3, cocoa: T3 empty, T2 has all three, frecency orders them. Correct — this is the case where frequency *should* decide.
T3 and T2 are index-backed on `dir_base` (T2 as a half-open range `base >= q AND base < q++`).
T1 and T0 are a scan, measured at 1.07 ms over 3000 directories — the same full scan zoxide does
on every query, and acceptable because they are only reached when the indexed tiers came up
empty, and only for a user-initiated `cd`.

TAKE VERBATIM FROM ZOXIDE, because these are right:
 * The last-keyword rule for T0. Not "the last keyword must be the final component" — the code is: after the rightmost occurrence of the keyword there may be no `/`. So `foo` matches `/a/foo` and `/a/foobar` but not `/foo/bar`. This one rule is why `z cargo` does not land you in `~/src/cargo-helpers/vendor/x`. Reproduce upstream's own test table (db/stream.rs:185-205) as oslo's tests — it is the specification.
 * Case-insensitive always. But fold once at write time into `dir.base` rather than per-row per-query.
 * Never return `$PWD`. zoxide does this in the *shell function* by string equality against `pwd`, and it silently fails whenever `pwd -L` and `pwd -P` disagree. Here it is a `path <> ?` bind of `logical_pwd(env)` in the query, which cannot drift.
 * Lazily skip candidates that no longer exist; mark `missing_since` rather than deleting on sight.

TWO THINGS I AM NOT PUTTING IN THE RANKING:
 * `dwell_ms`. It is recorded because the owner asked for it and it is a genuine report, but neither zoxide nor deja ranks on time-in-directory and I think that is deliberate. A tmux pane left overnight in `~/downloads` outranks the project you actually worked in all week. If a wall-clock term is ever wanted, `SELECT SUM(total_ms) FROM run WHERE dir_id = ?` is *active* time and is free, already stored, and immune to the idle-pane failure. Ship with the weight at zero.
 * A learned/negative-feedback term (zoxide #1020: "if I `z blah` twice in a row, demote the first answer"). Real, and out of scope.

MINIMUM CONFIDENCE: a directory with `visits < 2` is only jumped to from T3 (exact basename).
Otherwise one stray `cd` into a typo'd path teleports you there forever. And the jump ALWAYS
prints its destination, for the reason cd.rs:63-65 already gives for `cd -`: "the destination
came from the environment rather than from anything visible in the script".

## cd, case by case

THE ONE STRUCTURAL CHANGE, and it must come first. `change_directory` prints the diagnostic
itself at chdir.rs:198 and *then* returns `None`. Hooking cd.rs:115 naively produces `oslo: cd:
foo: No such file or directory` followed by a successful jump. So split it:

    fn attempt_directory(env, operand, mode, ) -> std::result::Result<String, io::Error>   // silent
    pub fn change_directory(env, operand, mode, caller) -> Option<String>                  // prints, calls the above

Two lines of real change in chdir.rs, every existing caller (`pushd`, `popd`, `dirs`) untouched.

THE LADDER inside `builtin_cd`, in evaluation order. Cases 1-6 are today's code, moved not
modified.

 1. `parse_mode` (cd.rs:17-44). UNCHANGED. `-L`/`-P`/`--`, last flag wins, a bare `-` is an operand, `-3` breaks out as an operand. Not touched, so `cd -LP dir` still means `cd -P dir` and `cd -x` still reports 2.
 2. `operands.len() > 1` -> status 2, "too many arguments". UNCHANGED. This forecloses multi-keyword `cd foo bar` for v1, deliberately — see open questions.
 3. No operand -> `$HOME`, or status 1 with "HOME not set". UNCHANGED, and I am specifically NOT building the first branch of the owner's zsh function. `cd ~ && cd -` makes bare `cd` a no-op that arms `cd -` to jump home. POSIX says `$HOME` and the constraint says bare `cd` behaves exactly as today. He already hedged it himself behind `[ -f /env/dot/.func/code/pro ]`, which suggests he knew it was not safe unconditionally.
 4. `-` -> `$OLDPWD`, announced. UNCHANGED.
 5. `-N` -> `ring::nth_back(n)`, announced. UNCHANGED in code; fixed in behaviour by seeding the ring (see ring verdict).
 6. Any other operand -> `attempt_directory(env, operand, mode)`. On `Ok(dest)`: identical to today, including the CDPATH search and its POSIX echo (chdir.rs:172-191). `cd ..`, `cd /abs`, `cd ./rel`, `cd link` under `-L` versus `-P`, and a real directory literally named `root` all keep their present meaning byte for byte, because this branch is evaluated before anything clever exists.
 7. NEW, and reachable only when 6 returned `Err` AND `track::get().is_some()`:
      a. operand == "root" -> `interactive::prompt::git_root()` (prompt.rs:39-48).
      b. otherwise -> the tiered frecency jump against `dir`, excluding `logical_pwd(env)`, skipping rows whose path no longer stats.
      On a hit: `attempt_directory` on the absolute result — so the destination is validated by a real `chdir` and `$PWD`/`$OLDPWD` are set by the one function that owns them — then print the destination, then return 0.
 8. Nothing found -> emit the *original* error from step 6, `oslo: cd: {operand}: {e}`, exactly as chdir.rs:198 does today, and return 1.

WHY EVERY POSIX CASE IS SAFE, individually:
 * `cd nonexistent` in a script: step 7 is unreachable because `track::get()` is `None` — a script never installs the handle, since only `repl.rs` calls `install`. Not "we remembered to check a flag": there is no store to consult. Same message from the same `eprintln!`, same status 1. This is the `expand/sugar.rs:63` precedent (`if !env.interactive() { return field.to_string(); }`) and the `autocd.rs:65-69` one, whose comment states the principle: "a script's meaning must not depend on which directories happen to exist beside it."
 * `cd nonexistent` interactively with a *cold* store: no candidate clears the confidence floor, step 8 fires, identical message and status.
 * `cd ..`, `cd -`, `cd -P`, bare `cd`: never reach step 7, because they either resolve at step 6 or fail before it in an arm that returns directly.
 * A directory named `root` in `$PWD`: wins, because step 6 runs before 7a. This is the same precedence the owner's zsh function has (`[ -d "$1" ]` before the `root` test) and it is what saves you from `./root`.
 * CDPATH: strictly BETTER than the zsh function. The zsh version tests `[ -d "$1" ]` *before* falling through to `z`, which silently defeats CDPATH — that is zoxide's single most-upvoted open issue, #620, with 18 thumbs up. Hooking the *failure* of `change_directory`, which has already searched CDPATH (chdir.rs:172-191), gets it right for free.
 * `-L` versus `-P` on the jump: `mode` is passed through to `attempt_directory`, so `cd -P somekeyword` resolves symlinks in the destination like any other physical cd.

WHAT `cd` DOES NOT GAIN: no interactive picker, no fzf, no `zi`. pazi's one good idea — show the
scores when it asks — is right, and oslo already owns `src/interactive/dropdown/` so it needs no
dependency, but it is a second PR. For v1 the jump is deterministic and always prints where it
went, which is the honest minimum.

## Directory-aware suggestions

THE CARGO CASE, concretely. Two directories, `~/src/alpha` (dir_id 7, root `~/src/alpha`) and
`~/src/beta` (dir_id 12, root `~/src/beta`). In alpha you have run `cargo run --example xyz` 14
times; in beta, `cargo run --example abc` 9 times. Both are rows in `run` keyed `(dir_id, 'sh',
argv)`.

You are in `~/src/beta` and you type `cargo run --ex`. `interactive::recall::suggest`
(recall.rs:72-94) is the hook — it already answers for the language the prompt is showing *now*,
which is the hard part and is already solved. Change it to ask the store first and fall back to
today's walk:

    // exact directory
    SELECT argv FROM run
     WHERE dir_id = ?1 AND mode = ?2 AND argv >= ?3 AND argv < ?4
       AND (last_status = 0 OR runs > fails)
     ORDER BY (runs - fails) DESC, last_at DESC
     LIMIT 1;

with `?3 = 'cargo run --ex'` and `?4 = 'cargo run --ey'` — the half-open range, NOT `LIKE`. This
is the single most performance-relevant detail in the whole design: `argv LIKE 'cargo run
--ex%'` degrades to `SEARCH run USING INDEX run_key (dir_id=?)` and then scans every row for
that directory, whereas the range gets a true B-tree range scan, `SEARCH run USING INDEX run_key
(dir_id=? AND mode=? AND argv>? AND argv<?)`. Measured 13 us. That is a per-keystroke budget
with three orders of magnitude of headroom, which is why there is no cache and therefore no
cache-coherence problem between terminals — deja needed a daemon largely to hold a cache it then
failed to invalidate (its own issue #21: new commands were unsuggestable "until the daemon is
restarted, and daemons survive across shell sessions, so that would be ~never").

Result: in beta you get `abc`, in alpha you get `xyz`. That is the feature, and it is one
indexed SELECT.

THE SUBDIRECTORY PROBLEM, which is the half that actually bites. You typed it in `~/src/beta`
but you are now in `~/src/beta/crates/api`. Exact-cwd finds nothing. Second query, atuin's
`FilterMode::Workspace`, which is a prefix match on the git root:

    SELECT r.argv FROM run r JOIN dir d ON d.id = r.dir_id
     WHERE d.root = ?1 AND r.mode = ?2 AND r.argv >= ?3 AND r.argv < ?4
       AND (r.last_status = 0 OR r.runs > r.fails)
     ORDER BY (r.runs - r.fails) DESC, r.last_at DESC LIMIT 1;

`run_argv ON run(mode, argv)` lets the range drive this one and the `d.root` join filter follow;
it is bounded by one repository either way. `d.root` comes from `dir.root`, which was written at
visit time by `prompt::git_root()`. No `git` subprocess on the keystroke path.

THIRD FALLBACK: today's behaviour, `recall::suggest`'s reverse walk over the language-filtered
remembered set (recall.rs:80-93). So the suggestion DEGRADES to what oslo does now in a fresh
directory rather than going silent. That ordering — here, then this repo, then anywhere — is the
whole ranking. I am deliberately not building deja's four-signal weighted blend (`1.0*fuzzy +
0.5*sequence + 0.4*frecency + 0.3*dir_affinity`, scorer.go:106-148). Its directory term is
`P(dir | command)` — the share of a command's runs that happened here — which rewards
exclusivity so hard that a command run *once*, here only, scores a perfect 1.0, identical to a
workhorse run 500 times here. A cascade of three ordered queries is more predictable and has no
weights to tune.

A DEFECT THIS FIXES AS A SIDE EFFECT: `recall::suggest` today offers the newest prefix match
with no idea whether it worked, so a typo'd command is suggested forever. `AND (last_status = 0
OR runs > fails)` ends that. It costs nothing because `fails` and `last_status` are columns on
the row already being read, and `last_status` being nullable is handled correctly — `WHERE
last_status = 0` excludes NULL, so a command whose exit was never observed is never mistaken for
a success.

WHAT ELSE READS THIS STORE. The owner said it will be used by other tools, and the read side
should be a *tool* in `src/data/tools/`, not a print-only builtin. `Val::Size(u64)`,
`Val::Duration(i64)` nanoseconds and `Val::Time(i64)` nanoseconds-since-epoch already exist
(data/value.rs:25-45) and already render and sort correctly, so a tool emitting `{path, visits,
last_visit: Time, dwell: Duration, mean: Duration}` needs no new value kind and no formatting
code. That is the difference between zoxide's `query --list --score` (a hardcoded `{score:>6.1}
{path}` string, db/dir.rs:58-66, which exists only because there was no other way to inspect a
bincode blob) and something you can pipe. `lua/api/tools.rs` is already 402 of the 600 allowed,
so this goes in its own `src/data/tools/track.rs`, not appended there.
  * "which tool in which directory": `SELECT head, COUNT(*), SUM(runs) FROM run GROUP BY dir_id, head`
  * "how long each tool takes": `SELECT head, SUM(runs), SUM(total_ms)/SUM(runs), MAX(max_ms) FROM run GROUP BY head ORDER BY SUM(total_ms) DESC` — index-backed on `run_head`.
  * "how long in each directory": `SELECT path, dwell_ms FROM dir ORDER BY dwell_ms DESC`

NOT BUILDING: deja's sequence bigrams (`sequences(prev, next, count)`). It is deja's most
distinctive idea and it is nearly free, but it is a fourth table and a fourth ranking signal,
and it should not land in the same change as the schema it depends on. If it does land later,
key it `(prev, next, mode)` — a shell line must never predict a Lua line — and leave `dir` out
of the key, which makes the table sparse and the counts too thin to rank. NOT BUILDING: an
empty-prefix suggestion. `recall::suggest` already returns `None` for an empty line
(recall.rs:73-75) and deja's constant-fuzzy-score-of-1 trick to paint a guess on a bare prompt
is noise. NOT BUILDING: a second fuzzy matcher. Prefix only. oslo's completion already ranks
candidates and its hinting already orders history -> completions -> paths; a differently-tuned
second engine buys inconsistency.

## prevd and nextd

THE OWNER IS RIGHT, and removing them deletes the only thing that makes `ring.rs` complicated.

GOES:
 * `builtin_prevd`, `builtin_nextd` (ring.rs:109-127) and their registrations at `env/builtins/mod.rs:135-136`.
 * The `back` cursor (ring.rs:32). It exists solely so `nextd` can walk forward. With it gone, `nth_back` loses its `1 + ring.back` offset (ring.rs:70) and becomes `visited[len - 1 - n]`.
 * `step_back`, `step_forward` (ring.rs:75-92).
 * The abandon-the-forward-history truncation in `record` (ring.rs:48-52), which then becomes a plain push-and-dedup.
 * `walk_to` (ring.rs:95-107), and good riddance — it calls `set_current_dir` and sets `PWD` by hand while never touching `$OLDPWD`, so a `prevd` silently desynchronised `cd -`. That is a latent bug being deleted, not a feature.
 * Two tests: `the_ring_walks_both_ways` (ring.rs:152-165) and `a_new_directory_abandons_the_forward_history` (ring.rs:167-179).
ring.rs goes from 204 lines to roughly 110.

STAYS:
 * `record`, `history`, `nth_back`. `cd -N` (cd.rs:87-101) is the feature being kept and these are all it needs.
 * `dirh` (ring.rs:129-137, registered mod.rs:137). The owner named prevd and nextd, not dirh, and it is the only way to *see* the numbers `cd -N` takes — `cd -3` is unusable without it. It is zsh's `cdr -l` and it is fine.
 * `the_same_directory_twice_is_one_entry` (ring.rs:196-203) and `nth_back_counts_from_here` (ring.rs:183-193), unchanged.
 * `DEPTH = 32`.

THE RING STAYS IN MEMORY AND STAYS SESSION-LOCAL. This is the decision I most want on the
record, because "we now have a database, put the ring in it" is the obvious next thought and it
is wrong. `cd -2` means "two back through *this shell's* wandering". Backed by a shared table,
three open terminals would make `cd -2` land somewhere the user has never been in this window.
The database answers "where do I usually go"; the ring answers "where was I just now"; merging
them ruins both. `OnceLock<Mutex<Ring>>` (ring.rs:35-38) is correct as it stands.

A BUG TO FIX WHILE IN HERE: `cd -1` is not `cd -` today, and the owner's stated model ("cd -
already does that, and cd -2 means two directories before") is therefore not currently true. `cd
-` reads `$OLDPWD` (cd.rs:74-83); `cd -N` reads `ring::nth_back` (cd.rs:94-100); the ring starts
empty (ring.rs:36-38) and is only appended after a successful cd (cd.rs:112). So in a fresh
shell in `/a`, after `cd /b` the ring is `["/b"]`, `nth_back(1)` does `0.checked_sub(1)` =
`None`, and `cd -1` prints "no such entry in the directory history" while `cd -` works. Fix:
`ring::record(&current_directory())` once at REPL startup. One line, and it is the same line
that primes the store's `current` dir_id.

MOVE THE RECORDING DOWN. `ring::record` is called from exactly one place, cd.rs:112. `pushd` and
`popd` call `change_directory` directly (stack.rs:141, 163, 193, 226) and never record, so "the
directories you have actually been in" (ring.rs:1) silently omits every pushd. Move the call
into `change_directory` after `$PWD` is set (chdir.rs:205), which fixes pushd, popd, and a `cd`
inside a shell function all at once. That is the argument chdir.rs:1-6 already makes for itself:
"the one place the shell changes directory", so the three things easy to get subtly wrong are
decided once.

Note the ring recording and the *database* visit recording go to different places on purpose.
The ring goes in `change_directory` (every chdir, cheap, in-memory). The database visit goes at
repl.rs:238 where `after != before` is already computed — one write per prompt, not one per
chdir, so a shell function that cds in a loop does not write a thousand rows.

BEFORE DELETING: grep `docs/`, the README, `examples/` and any Lua rc for `prevd`/`nextd`. They
are registered builtins and something may bind them.

## Privacy and size

PRIVACY. This writes the command line AND the directory it ran in. The directory is often the
more identifying half — a client's name, an employer's, an unreleased project. Six layers,
arranged so no single one has to be perfect.

 1. THE LEADING-SPACE RULE ALREADY EXISTS. `history::is_secret` (history.rs:107-115) and repl.rs:175-177 already gate both `$HISTFILE` and `history.db`. The tracker gates on the SAME `secret` flag at the SAME point. This is the only mechanism the user controls deliberately, so it is the one that must never be bypassed. Note the trap: the dwell/visit write at repl.rs:238 is OUTSIDE the `if !secret` block, so a naive implementation records the directory of a secret command. Both writes go inside.
 2. NEVER WRITE FROM A NON-INTERACTIVE SHELL. Enforced structurally: only `repl.rs` calls `track::install`, so a script, `oslo -c`, or a subshell has `track::get() == None`. A CI job's command lines never touch the file. This is a privacy answer and a volume answer at once.
 3. HONOUR `HISTFILE=""` AND `HISTSIZE=0`. history.rs:46-47 documents an empty `HISTFILE` as "the documented way to run a session that leaves no trace". A user who took that step and then found a new tracking file had appeared would be right to be angry.
 4. THE STRUCTURAL MITIGATION, and the reason the schema carries both `argv` and `head`: a secret is almost never the command *name*, it is in the arguments. When a line trips any filter below, write the row with `argv` set to `head` alone and keep `dir_id`, `runs`, `total_ms`, `last_status`. Every timing and directory statistic survives; only the risky text is dropped. A denylist that must be perfect is a bad design; a denylist whose worst failure is reduced resolution is a fine one.
 5. THE FILTERS, none load-bearing alone. Command names: pass, gpg, openssl, vault, op, ssh-add, mysql, psql, secret-tool, security, keyctl, htpasswd, and the auth subcommands of gh/aws/docker/npm. Option shapes: `--?(password|passwd|token|secret|api[-_]?key|auth|bearer|credential)([=:].*)?`, glued `-p<value>`, `-u user:pass`, a literal `Authorization:`. Assignments: a leading `VAR=value` word, or any variable name containing TOKEN/SECRET/KEY/PASS. Value shape: >= 24 chars with mixed case and digits and no `/` and no `.` is a key, not a path; plus the known prefixes `gh[pousr]_`, `sk-`, `AKIA`, `eyJ`, `-----BEGIN`. Length: refuse any argv over 4 KB — it is a paste, not a habit, it will never be suggested, and it is the shape a leaked key arrives in. Never record a heredoc body. Never record a line that failed to parse, because a typo is often a password typed into the wrong prompt.
 6. A DIRECTORY EXCLUSION LIST, which zoxide has (`_ZO_EXCLUDE_DIRS`) and oslo does not. Globs via the existing `expand::glob::ShellPattern`, keeping a whole subtree out of `dir` and not merely out of `run`. Defaults: `$HOME` itself (zoxide's default too — your home directory is never a jump target), `/tmp`, and anything under a `node_modules` or `.git` component. A store that ranks `~/.cargo/registry/src/index.crates.io-xxxx/serde-1.0.219` because a build touched it is worse than useless.

`history -c`: repl.rs:222-233 already clears the editor's history, `history.db` and the recall
set together, and its own comment says why — a shell that went on suggesting lines it had just
been told to forget "would be lying". A tracker that kept them would be lying in exactly the
same way. So `history -c` must `DELETE FROM run`. It must NOT touch `dir`: "forget my command
lines" is not "forget where I work". Provide `oslo.track.forget(path)` for the other half.

SIZE. Measured 194 bytes per row on turso 0.7.2, with all indexes, 55-char argv and 45-char
paths, after a checkpoint. At 200 commands/day = 73,000 executions/year, the table grows only on
*new* `(dir, mode, argv)` triples — repeats are the entire point and cost nothing. At a distinct
fraction of a quarter to a third, that is ~20,000 new rows/year: about 4 MB/year for `run`,
under 1 MB for `dir`, reaching a steady state of a few MB once the 90-day `runs = 1` prune
bites. The same year as a raw event log would be ~11 MB and growing without bound,
unreclaimable.

THE WAL TRAP, which will otherwise become the largest file in `~/.local/share/oslo`: the WAL
grows to 1.6-2.2 MB and NEVER truncates on its own — not on `Database` drop, not on reopen.
Measured: db 1,908,736 bytes / wal 1,623,312 bytes after a clean drop. `PRAGMA
wal_checkpoint(TRUNCATE)` via `query()` takes it to exactly 0. Checkpoint at shell exit and
after each prune.

AND THE HARD CONSTRAINT BEHIND ALL OF IT: turso 0.7.2 has no VACUUM (`VACUUM is an experimental
feature`) and no `auto_vacuum`. Freed pages go to the freelist and are reused, but the file
never shrinks — it is a permanent high-water mark. That is why the store is an aggregate, and
the documented escape hatch for a pathologically large file is `rm
~/.local/share/oslo/track.db`. That is safe precisely because every byte in it is derived and
rebuildable by use, which is also the reason it must never be allowed to refuse to start a
shell.

## Staging

1. 1. chdir.rs: split `change_directory` into a silent `attempt_directory` returning
   `Result<String, io::Error>` and a printing wrapper. Move `ring::record` from cd.rs:112 into
   the wrapper after `$PWD` is set, which fixes the pushd/popd gap. No behaviour change;
   existing tests must pass untouched. ~25 lines.
2. 2. ring.rs: delete `builtin_prevd`, `builtin_nextd`, `back`, `step_back`, `step_forward`,
   `walk_to`, the forward-history truncation, and the two cursor tests. Unregister at
   env/builtins/mod.rs:135-136. Seed the ring with `$PWD` at REPL startup, which makes `cd -1`
   == `cd -`. Add a test pinning that equivalence. ring.rs 204 -> ~110 lines. Ships
   independently of everything below.
3. 3. src/track/mod.rs + write.rs: open, PRAGMAs, schema, `PRAGMA user_version`, the process-
   global `OnceLock<Option<Track>>` and `install()`, the three-statement transaction.
   `install()` called only from repl.rs. Wire the write at repl.rs:255 next to the postcmd
   fire. Gate on `secret` at the same point repl.rs:175-177 gates `db.append`. Nothing reads
   the store yet. Also move `db.trim` behind a 100-command counter while in this loop.
4. 4. src/track/redact.rs: `head` extraction (atuin's interesting_command) and the argv
   redaction rules. Land this in the SAME commit as step 3 or the store collects unredacted
   lines before the filter exists. It is the most testable part of the whole feature and wants
   its own file and its own test module.
5. 5. src/track/prune.rs: the daily sweep on a detached startup thread, modelled on
   `command_index::warm` at repl.rs:80-83, plus the `wal_checkpoint(TRUNCATE)` via `query()`.
   Land before the store has been running long enough to need it, not after.
6. 6. src/track/match_.rs + query.rs: the T3..T0 cascade with zoxide's last-keyword rule,
   reproducing upstream's test table from db/stream.rs:185-205 verbatim as oslo's tests. Still
   no caller.
7. 7. src/env/builtins/directories/jump.rs: the step-7 arm of the cd ladder — `root` via
   prompt::git_root(), then the frecency jump, then fall through to the original diagnostic.
   cd.rs gains about eight lines and stays near its present 192. Differential tests: `cd
   nonexistent` in a script must be byte-identical, `cd ..`/`cd -`/bare `cd`/`cd -P`
   unchanged, a real `./root` directory still wins.
8. 8. recall.rs: `suggest` asks the store for exact-cwd, then git-root-prefix, then falls back
   to today's walk. Three queries, ~13 us, no cache. This is the owner's headline feature and
   it goes last because it depends on the store having data.
9. 9. src/data/tools/track.rs: the structured read side emitting Val::Time / Val::Duration
   rows, so the store is inspectable and tunable through the pipeline rather than through a
   bespoke `--list --score` format.

## Still open

- Bare `cd` going home-and-back. The first branch of the owner's zsh function (`cd ~ && cd -`)
  directly contradicts the stated non-negotiable that bare `cd` behaves exactly as today, and
  POSIX says $HOME. I did not design it in. If he wants it, the shape is an AtomicBool exactly
  like `autocd::AUTOCD` (autocd.rs:21), settable from `shopt -s cdtoggle` and `OSLO_CDTOGGLE`,
  gated on `pipeline::is_interactive()` which is NOT configurable. He already hedged it
  himself behind a per-machine sentinel file, which suggests he knew. Needs an explicit yes or
  no.
- Does `history -c` wipe the `run` table? I say yes and I say `dir` survives, but this is a
  product decision, not an implementation detail. Wiping `run` also wipes the per-directory
  suggestion, which is the feature he asked for, so `history -c` becomes noticeably more
  destructive than it is today.
- Multi-keyword `cd foo bar`. I kept the single-operand arity check (cd.rs:56-61) because
  relaxing it changes the error status for a real POSIX misuse from 2 to something else.
  zoxide's `z foo bar` is genuinely useful. It is a separate decision and a separate PR.
- Does `dwell_ms` earn its place? Neither zoxide nor deja tracks time-in-directory, so there
  is no prior art saying it ranks well, and I am shipping it with a ranking weight of zero. If
  after a month the table shows nothing that `SUM(total_ms)` does not, drop the column rather
  than keep collecting it. Note that deja has been recording exit_code, duration_ms and
  session_id since v0.1 and reads none of them — I grepped every non-test Go file.
- Absorbing `frecency_store.rs`. It is a second, weaker copy of what `run` holds (count, last-
  use, name), it lives at `$HOME/.oslo_frecency` in contradiction of the XDG argument at
  history_db.rs:8-12, and `SELECT head, SUM(runs), MAX(last_at) FROM run GROUP BY head` is a
  strict superset that is also directory-aware. I am deliberately NOT folding it in for v1 —
  it is on the completion hot path, it works, and it has a test
  (`two_shells_do_not_clobber_each_other`, frecency_store.rs:177) worth not disturbing in the
  same change. But it should not be forgotten, or oslo ends up with two ranking stores that
  disagree.
- turso 0.7.2 is pre-1.0 and a pure-Rust SQLite reimplementation. `history.db` already carries
  that risk with one append per command; this is a materially hotter workload. Verify two
  shells writing simultaneously under the real REPL, not just under a probe, before merging —
  and confirm the partial index `run_age ... WHERE runs = 1` is still correctly maintained
  when the upsert takes `runs` from 1 to 2 under concurrency, not only single-threaded.
- `HOOKS` is declared `[&str; 4]` at lua/api/shell.rs:29. If a `dirchange` hook is ever wanted
  the array size changes with it — but the dwell and visit accounting must stay native Rust at
  repl.rs:238, never a Lua hook, because handlers are removable via `handle:remove()`
  (shell.rs:165-180) and accounting a config can silently switch off is worse than none.
