# Oslo History Implementation Plan

## Status

- **State:** implemented and audited on 2026-08-09
- **Branch:** `feat/tagdata-storage`
- **Baseline:** commit `64ec8ec`; the Tagdata migration is committed in `f6c8728`
- **Priority:** implement phases in order; do not expose sync before the stable event model exists
- **Binary size:** hard constraint; every phase must measure and minimize its release-binary delta
- **Documentation constraint:** do not add this plan or a command reference to `README.md`

This file is the implementation plan. It replaces the old performance notes and
the temporary files under `plans/`.

## PLAN_DIVERGES.md protocol

`PLAN_DIVERGES.md` is a live override supplied by the user. Before starting any
phase, and again before declaring that phase complete:

1. Read `PLAN.md` and the current `PLAN_DIVERGES.md` if it exists.
2. Apply every relevant divergence to the remaining work before editing source.
3. Update this plan when a divergence changes architecture, command behavior,
   ordering, scope, or acceptance criteria.
4. Treat the newest divergence as authoritative when it conflicts with this
   plan, unless it would corrupt data or cannot be implemented with the current
   storage engine. Report such a conflict instead of silently ignoring it.
5. Never assume a previously read copy of `PLAN_DIVERGES.md` is still current.

## Goal

Turn the current `oslo history` placeholder into a useful administrative and
sync tool for Oslo's Tagdata history store. The main requirement is safe,
repeatable synchronization between two database files:

```text
oslo history sync FILE1 FILE2
```

The result must preserve independent events from both computers, propagate
updates and deletions, avoid duplicate aggregate counts, and converge when the
same sync is repeated.

## Product boundaries

### In scope

- Inspect, search, filter, show, and summarize command history.
- Verify and back up an existing history database.
- Bidirectionally synchronize two history databases.
- Delete or clear history using sync-aware tombstones.
- Export and import history using a versioned portable format.
- Run local pruning without turning local retention into remote deletion.
- Support both human-readable output and stable machine-readable output.

### Out of scope

- A server, cloud account, daemon, background sync, or network transport.
- Encryption or secret management beyond existing filesystem permissions.
- Byte-identical database files after sync.
- Synchronizing local directory visit counts, dwell time, missing-path state,
  prune timestamps, local numeric IDs, or open-handle statistics.
- A raw whole-database merge command.
- A `save` command; Tagdata transactions already persist writes.
- In-place compaction while shells may hold the old file open.
- Changes to the flat `$HISTFILE` compatibility format.
- Adding a long history manual to `README.md`.

## Binary size constraint

Oslo must remain a minimal static shell. Binary size is an acceptance criterion,
not a cleanup task to defer until the end.

1. Run `make build` before Phase 1 and record the exact byte size of
   `target/x86_64-unknown-linux-musl/release/oslo` as the baseline.
2. Run the same release build after every phase and record the exact size and
   delta in the size ledger below. Debug builds are not valid measurements.
3. Treat every increase as something to explain and reduce. A phase is not
   accepted while it contains an unexplained size regression.
4. Prefer existing dependencies and compact local implementations. Reuse the
   already-linked `sha2`, `serde_json`, and regex facilities where applicable.
5. Do not add a CLI framework, UUID framework, date/time framework, serialization
   derive stack, async runtime, or another digest/random/version family for this
   tool. Use the existing manual CLI style and framed codecs.
6. Before enabling a Tagdata feature or adding even a small dependency, measure
   the release binary with and without it. Keep it only when the functionality
   cannot be implemented safely with code already linked into Oslo.
7. Avoid generic abstractions and duplicated human/JSON data models that
   monomorphize large amounts of code. Stream list/search/export records rather
   than collecting or cloning entire databases where the algorithm permits it.
8. Do not trade away correctness, collision resistance, file safety, or the
   static no-`INTERP`/no-`NEEDED` guarantee merely to save bytes. When two safe
   designs exist, choose the smaller measured result.

### Size ledger

Fill this table during implementation using exact bytes, not only the rounded MB
printed by `make build`.

| Checkpoint | Exact bytes | Delta from baseline | Dependency change | Decision |
|---|---:|---:|---|---|
| Before Phase 1 | 5,962,688 | 0 | Tagdata 0.1.3, default features | Accepted |
| After Phase 1 | 6,015,936 | +53,248 | Reused the existing `getrandom` 0.4 and `sha2` 0.10 package families | Accepted |
| After Phase 2 | 6,130,624 | +167,936 | No dependency change | Accepted |
| After Phase 3 | 6,200,256 | +237,568 | No dependency change | Accepted |
| After Phase 4 | 6,282,176 | +319,488 | Enabled Tagdata `maintenance`; no new package family | Accepted |
| Final audited build | 6,302,656 | +339,968 | No CLI/framework/runtime dependency added | Accepted |

The phase checkpoints were reconstructed at the Phase 4 audit from the
then-current source by exposing only the command surface available through each
phase and rebuilding the static release artifact. Release dead-code elimination
therefore measured reachable code rather than merely counting source. Disabling
Tagdata maintenance and replacing backup with an unavailable stub produced a
6,265,792-byte binary, 16,384 bytes smaller than the 6,282,176-byte Phase 4
checkpoint. The feature is retained because it is the only implemented path
that provides a validated, consistent backup while live handles remain open.
The final audit added 20,480 bytes for conflict-preserving concurrent sync,
strict projection and import validation, exact local revision replacement, and
private atomic backup staging. That measured increase is retained because each
part closes a data-loss, data-consistency, or file-permission failure mode.

## Baseline implementation findings

- At baseline, `src/cli/tools.rs` advertised history search/export/prune but only
  printed help and rejected all history subcommands.
- `src/main.rs` already forwards tool arguments through `Action::Tool`.
- `crates/oslo-base/src/track/` already provides recent history, clear, trim,
  append, rewrite, drop, observations, outcomes, command aggregation, forget,
  sweep, profile paths, and store availability.
- The Tagdata migration exposes database statistics and verification. Its
  maintenance feature exposes consistent backup support.
- Persisted `History`, `Outcome`, and directory/index records are joined through
  database-local numeric IDs.
- `RunRow` is an aggregate. Its counts must be combined or reversed, not
  overwritten. Its newest host/session fields also need to follow the newest
  observation.
- The runtime writes a history row before execution, may rewrite or drop it,
  records aggregate observations, and records the final outcome last. A portable
  event must support this incomplete-to-complete lifecycle.
- The Tagdata store is the canonical source for Oslo's language-aware history
  and seeds interactive recall. `$HISTFILE` remains a compatibility append log.

## Why a raw Tagdata merge is unsafe

Two computers can independently allocate the same history ID and directory ID
for unrelated records. A whole-store `DB::merge_from` can therefore:

- overwrite one command with another;
- attach an outcome or run to the wrong directory;
- corrupt path and worktree indexes;
- replace machine-local metadata counters;
- double-count or discard aggregated runs.

Synchronization must merge one portable event bucket and then rebuild or update
each file's local projections. All other buckets remain local.

## Target command surface

```text
oslo history path
oslo history status [FILE] [--json]
oslo history list [QUERY] [-n N] [--oldest] [--json] [--null]
oslo history search QUERY [-n N] [FILTERS] [--json] [--null]
oslo history show EVENT_ID [--json]
oslo history stats [--host HOST] [--since DURATION] [--json]
oslo history verify [FILE] [--json]
oslo history sync OTHER [--dry-run] [--json]
oslo history sync FILE1 FILE2 [--dry-run] [--json]
oslo history delete EVENT_ID... [--yes]
oslo history clear --yes
oslo history prune [--dry-run] [--yes]
oslo history export [FILE|-] [--format jsonl|text]
oslo history import FILE [--dry-run]
oslo history backup FILE
```

### Command behavior

- `path` prints the current profile's database path and performs no write.
- `status` reports path, schema version, file size, Tagdata statistics, visible
  event count, tombstone count, and pending/local projection counts.
- `list` emits events chronologically. Its optional query is a simple substring.
- `search` supports `--exact`, `--prefix`, or `--contains`, plus `--host`,
  `--cwd`, `--status`, `--since`, and `--before` filters.
- `show` accepts the stable event ID, never a database-local history ID.
- `stats` reports command counts, success/failure counts, hosts, directories,
  durations, and time range without exposing local storage keys.
- `verify` uses an error-returning existing-file open and Tagdata verification;
  it must never create, rename, replace, or migrate the supplied file.
- `sync OTHER` synchronizes the current profile database with `OTHER`.
- `sync FILE1 FILE2` synchronizes two explicit files. The order does not select
  a winner; conflict resolution is deterministic.
- `delete` tombstones specific events. It requires confirmation unless `--yes`
  is present.
- `clear` tombstones every visible event and always requires `--yes`.
- `prune` performs local sweep/retention maintenance only. It must not create
  sync tombstones for rows hidden by local `$HISTSIZE` or missing remote paths.
- `export` writes JSONL by default. Text export writes command lines only.
- `import` preserves JSONL event IDs and revisions; text input creates new local
  events. Reimporting the same JSONL must be idempotent.
- `backup` uses Tagdata's consistent backup API rather than copying a live file.

Human output goes to stdout, diagnostics go to stderr, and JSON output remains
valid even when there are no records. `--null` applies only to text records and
terminates them with NUL for safe scripting. CLI parse errors exit 2; operational
or verification failures exit 1; successful empty queries exit 0.

## Storage architecture

### Stable event identity

Add `EventId`, encoded as exactly 32 bytes and printed as 64 lowercase hex
characters.

- New local events receive cryptographically random IDs through `getrandom`.
- Schema migration derives a deterministic SHA-256 ID from the persisted history
  key and encoded history value.
- Byte-identical rows from a copied common database deduplicate.
- Equal local numeric IDs with different content remain distinct.
- Repeated executions of the same command remain distinct events.

Do not use command text, timestamp, or the current local ID alone as identity.

### Portable history event

Add one versioned portable event record containing:

- event ID;
- revision;
- deletion flag and random conflict tie-breaker;
- line and language/mode;
- recorded timestamp;
- origin host and stable origin session;
- sequence and rewrite state;
- optional completion data: host-qualified working directory, worktree/root,
  exit status, duration, and chain segment outcomes.

An appended command begins as incomplete. Rewrite, outcome completion, and
deletion create newer revisions of the same event.

Winner ordering is `(revision, deleted, tie_breaker)`. Identical payloads are
unchanged even if their stamps differ. Deletion wins a same-revision conflict so
an older live row cannot resurrect an explicitly deleted event.

### Portable and local buckets

Add three root buckets:

1. `SyncEvent`: `EventId -> HistoryEvent`; this is the only bucket merged
   between files.
2. Local history ID to event ID.
3. Event ID to local history ID, applied revision, contribution snapshot, and
   locally-hidden state.

The projection buckets are never merged. Existing runtime `History`, `Outcome`,
`Run`, directory, and index buckets remain fast local views.

### Projection rules

- Applying the same `(EventId, revision)` twice changes nothing.
- A newly imported visible event receives fresh local history/directory IDs.
- A completed imported event contributes exactly once to `Run` aggregates.
- Updating or deleting an applied event reverses its previous contribution
  before applying the new winner.
- Reversal uses the saved contribution snapshot, not a reconstruction from the
  already-overwritten event.
- A locally trimmed projection remains hidden when the unchanged remote event is
  seen again.
- Explicit `delete`, `clear`, and `forget` create tombstones.
- Local `$HISTSIZE` trimming hides projections without creating tombstones.
- Remote directories retain origin host, do not enter local `DirByPath`,
  `DirByBase`, or `DirByRoot` navigation indexes, and are skipped by local
  missing-path pruning.
- `RunRow::absorb` updates host/session when a newer observation becomes the
  aggregate's newest observation.

Append a trailing host field to `DirRow`. Existing rows without it decode as
legacy/local without inventing a false origin host.

### Schema migration

Bump the database schema from 2 to 3 and replace the current direct version
stamp with a resumable migration runner.

For every schema-2 history row:

1. Derive its deterministic legacy event ID.
2. Preserve the stored timestamp instead of discarding it during decode.
3. Resolve any matching outcome, directory, root, status, duration, and chain
   segment data.
4. Infer an origin host only when existing data proves it; otherwise store
   unknown rather than the host performing the migration.
5. Write the portable event and both projection mappings.

Process bounded chunks and persist a migration cursor in `Meta`. Reopening after
a crash resumes without duplicate events, renumbered rows, or double aggregate
contributions. Stamp schema 3 only after all required rows and mappings exist.
Aggregate-only rows with no history event remain local and are not fabricated
into portable events.

## File safety and synchronization algorithm

### Administrative open mode

Add a strict, error-returning `open_existing` path for explicit-file commands.
Unlike the interactive best-effort path, it must:

- never create a missing file;
- never replace an unsupported file with a fresh store;
- never rename a file to `.unreadable`;
- never swallow engine, permission, schema, or decoding errors;
- reject future schemas without writing;
- leave `--dry-run` and `verify` completely read-only.

Canonicalize both sync paths and reject the same file, including symlink and
hard-link aliases. A missing second database is an error; file creation can be a
future explicit option, not an implicit side effect.

Validate Tagdata format/page size, Oslo schema, and every portable event in both
files before the first write. Keep database and generated export/backup files at
mode `0600`.

### Bidirectional convergence

Never call `DB::merge_from` for an Oslo store. Use Tagdata
`Bucket::merge_from` only on `SyncEvent`:

1. Open and validate both existing databases without mutation.
2. Read both event sets and compute deterministic winners and per-side reports.
3. For `--dry-run`, print the report and stop without migration or writes.
4. Merge B-only event keys into A with `KeepExisting`.
5. Write custom conflict winners into A.
6. Merge A's converged sync bucket into B with `Overwrite`.
7. Apply changed winners to each database's local projections in separate local
   transactions.
8. Report added, updated, deleted, unchanged, and locally applied counts for
   both files.

Never hold writer transactions for both files simultaneously. Open/operate in
canonical path order so concurrent opposite-order sync commands cannot deadlock.
The two files cannot be committed atomically together. If the process stops
after A is updated, rerunning the same command must converge both files without
duplicates or counter changes.

Logical portable events converge. Physical pages, free lists, machine-local
state, local history IDs, directory IDs, and projection order may differ.

## Implementation phases

### Phase 0: Reconfirm baseline and divergences

1. Re-read `PLAN_DIVERGES.md` and update this plan if needed.
2. Confirm branch, Tagdata version/features, current schema, bucket list, CLI
   dispatch, runtime write ordering, and Makefile targets.
3. Run focused baseline checks and record unrelated failures before changes.
4. Run `make build`, record the exact release-binary baseline in the size ledger,
   and inspect the current dependency versions before adding any dependency.
5. Do not edit `README.md`.

**Gate:** the Tagdata dependency must expose bucket-level merge with
`KeepExisting`, `Overwrite`, and `Error` policies. Stop if portable bucket merge
cannot happen inside a normal transaction.

### Phase 1: Stable events, migration, and projections

Likely files:

- `crates/oslo-base/Cargo.toml`
- `Cargo.lock`
- `crates/oslo-base/src/track/{mod,db,log,outcome,history,row,write}.rs`
- `crates/oslo-base/src/track/kv/mod.rs`
- `crates/oslo-base/src/track/sync.rs` or `track/sync/`
- corresponding `oslo-base` tests

Steps:

1. Add `EventId`, update stamps, portable event/completion/segment models, and
   strict codecs with independent event-format versioning.
2. Add the portable and projection buckets and bucket uniqueness tests.
3. Implement schema 2-to-3 migration with cursor, bounded batches, resume, and
   idempotency.
4. Dual-write append, rewrite, outcome, drop, forget, clear, and trim mutations
   in the same Tagdata transaction as their existing local rows.
5. Implement idempotent projection application and contribution reversal.
6. Host-qualify directory rows, exclude remote directories from local indexes,
   and fix newest host/session aggregation.

**Phase acceptance:** every new/migrated history row has one stable event and
consistent mappings; local mutations cannot commit only one representation;
reapplying an event does not change counts. The release-binary delta is measured,
recorded, and reduced to the smallest safe implementation found.

### Phase 2: Read-only history commands

Likely files:

- `src/cli/tools.rs`
- new `src/cli/history.rs` and focused parser/render tests
- `crates/oslo-base/src/track/query.rs`
- `crates/oslo-base/src/track/profile.rs`
- strict administrative-open code under `track/kv/`

Steps:

1. Replace the history placeholder with a dedicated argument parser and command
   dispatcher without adding another large CLI dependency.
2. Implement `path`, `status`, `list`, `search`, `show`, `stats`, and `verify`.
3. Expose query records by stable event ID and add filtering/pagination without
   leaking Tagdata types outside `track/kv/`.
4. Add consistent human, JSON, and NUL-delimited output helpers.
5. Keep help concise and self-contained; do not copy it into `README.md`.

**Phase acceptance:** all read-only commands work against current-profile and
explicit files, malformed arguments exit 2, store failures exit 1, and no
read-only command changes file bytes or timestamps. No CLI/parsing dependency is
added, and the measured binary delta is recorded and reviewed.

### Phase 3: Bidirectional file sync

Likely files:

- `crates/oslo-base/src/track/sync/`
- `crates/oslo-base/src/track/kv/`
- `src/cli/history.rs`
- real-file integration tests

Steps:

1. Implement strict two-file validation and canonical identity checks.
2. Build an in-memory reconciliation plan and deterministic conflict resolver.
3. Implement bucket-only A/B convergence using the algorithm above.
4. Apply each side's local projections after its portable bucket is updated.
5. Add `sync OTHER` and `sync FILE1 FILE2`, `--dry-run`, human reports, and JSON
   reports.
6. Preserve failures with enough context to identify which file and stage failed
   while never printing command contents in generic error messages.

**Phase acceptance:** repeated sync reports zero changes; interruption after the
first file commit is repaired by rerun; same local IDs from independent files do
not collide; aggregate counts remain exact. The measured binary delta is recorded
and any avoidable generic/duplicate reconciliation code is removed.

### Phase 4: Destructive operations and interchange

Likely files:

- `src/cli/history.rs`
- `crates/oslo-base/src/track/sync/`
- `crates/oslo-base/src/track/prune/`
- Tagdata maintenance feature configuration
- focused command and real-file tests

Steps:

1. Implement confirmed `delete` and `clear` as portable tombstones plus local
   projection removal.
2. Expose local-only `prune --dry-run` and confirmed application.
3. Define versioned JSONL export records from portable events and safe text
   export, including multiline and embedded-NUL handling.
4. Implement idempotent JSONL import and new-event text import.
5. Enable/use Tagdata's maintenance backup API for consistent `backup` output.
6. Ensure output files are created with `0600` and partial outputs are not
   mistaken for successful exports/backups.

**Phase acceptance:** tombstones propagate through sync, local trim/prune does
not delete remote history, JSONL round-trips stable IDs and outcomes, repeated
import is idempotent, and a backup verifies successfully. Tagdata maintenance and
interchange code remain only if their measured size cost is the smallest safe
option.

### Phase 5: Final audit

1. Re-read and apply the newest `PLAN_DIVERGES.md`.
2. Run every repository gate below through `make`.
3. Exercise every documented command against temporary real Tagdata files.
4. Confirm no whole-store merge exists and direct Tagdata calls remain within
   `crates/oslo-base/src/track/kv/`.
5. Confirm `README.md` contains no implementation-plan or expanded command
   reference added by this work.
6. Run the final release build, enter its exact size and baseline delta in the
   ledger, and audit new dependency/version families and feature activation.
7. Report any unrelated baseline failure separately; do not hide it by weakening
   tests.

## Required test matrix

### Schema and codecs

- Random event ID round-trip and invalid ID text.
- Deterministic legacy IDs for copied byte-identical rows.
- Distinct event IDs for equal local numeric IDs with different content.
- Multiline, embedded-NUL, non-ASCII, empty, and very large command lines.
- Truncated fields, invalid UTF-8, unknown mandatory format, and trailing data.
- Empty/populated schema-2 migration, partial resume, and repeat migration.
- Future-schema and malformed legacy-row refusal without mutation.

### Local writes and projections

- Incomplete append followed by rewrite and completed outcome.
- Atomic rollback when either portable or local write fails.
- Same revision reapplication produces no changes.
- Imported event contributes exactly once to aggregate counts.
- Newer event updates and deletion reverse the prior contribution exactly once.
- Same command from two hosts counts as two executions.
- Newest run host/session follows the newest observation.
- Outcomes and chain segments remap to new local IDs correctly.
- Local trim hides without tombstoning.
- Explicit forget/clear/delete creates tombstones.
- Remote directory with a local-looking path never enters local navigation
  indexes and survives local missing-directory sweeps.

### Sync

- Disjoint databases become the union on both sides.
- Repeating sync produces zero changes and no count changes.
- Databases copied from a common base do not duplicate common rows after
  independent divergence.
- Unrelated records with identical local history and directory IDs both survive.
- Incomplete event followed by a complete revision converges.
- Concurrent rewrite/delete converges deterministically and deletion wins at the
  same revision.
- Tombstones from delete, clear, and forget propagate.
- A crash after the first file commit converges on retry.
- Same path, symlink alias, hard-link alias, missing file, permissions failure,
  corruption, incompatible page format, and future schema refuse safely.
- `--dry-run` creates, migrates, renames, and writes nothing.
- Active persistent read handles and concurrent shell writes do not lose events.
- Database permissions remain `0600`.

### CLI and interchange

- Help and unknown commands/flags have stable exit codes.
- Human, JSON, and NUL output handle empty and multiline records.
- Search modes and each filter combine correctly.
- Event IDs from list/search/show/delete refer to the same portable event.
- Confirmation is required for delete, clear, and mutating prune.
- JSONL export/import round-trip is lossless and repeat import is idempotent.
- Text import creates distinct new events.
- Backup made while another handle is open verifies successfully.

## Repository verification

Use the Makefile for all relevant tasks:

```sh
make fmt
make fmt-check check clippy rustdoc check-loc check-readme
make test OURS='-p oslo-base'
make test
make check CARGO='rustup run 1.89.0 cargo'
make build
```

After each `make build`, record the exact byte count of the resulting release
binary in the size ledger. Compare dependency and feature changes against the
baseline, not only the rounded MB printed by the Makefile. The static build must
retain the repository's existing no-`INTERP`, no-`NEEDED` guarantee and must be
the smallest measured safe implementation of this plan.

At plan-writing time, full `make test` has an unrelated stale expected-failure
bookkeeping failure: these corpus cases now pass while remaining listed as
expected failures:

- `arith_for_unspaced_sections.sh`
- `syntax_unsupported_coproc.sh`
- `syntax_unsupported_select.sh`

Do not change that list as part of history implementation. Focused history tests
must be green, and the unrelated baseline must be reported separately until it
is fixed in its own scope.

## Stop conditions

Stop implementation and report instead of improvising if:

- `PLAN_DIVERGES.md` requests behavior incompatible with safe data preservation;
- Tagdata no longer provides the required bucket-level merge policies;
- history and portable event writes cannot share one transaction;
- imported aggregate contributions cannot be reversed without affecting an
  unrelated local event;
- a migration would have to invent origin identity and mislabel old data;
- a proposed design requires whole-database merge;
- explicit-file inspection would need to create or replace the file;
- implementation begins exposing direct Tagdata types outside `track/kv/`;
- a phase adds an unexplained binary-size regression or duplicate dependency
  family and no size comparison has been recorded.

## Completion checklist

- [x] Current `PLAN_DIVERGES.md` has been applied; no override file existed at the final audit.
- [x] Schema 2 migrates resumably and idempotently to schema 3.
- [x] All new local history mutations atomically update portable events.
- [x] Stable event projection is idempotent and reversible.
- [x] Remote directory metadata cannot affect local navigation/pruning.
- [x] Read-only history commands are implemented and truly read-only.
- [x] Both sync forms converge safely and repeatably.
- [x] Sync never merges the whole Oslo database.
- [x] Delete and clear propagate tombstones; local trim/prune do not.
- [x] Export/import/backup are safe, versioned, and tested.
- [x] Required focused and repository-wide `make` gates have been run.
- [x] Static and MSRV guarantees remain intact.
- [x] Every phase has an exact release-binary measurement and reviewed delta.
- [x] No avoidable dependency, duplicate version family, or large framework was
      added for history functionality.
- [x] The final build is the smallest measured safe implementation considered.
- [x] `README.md` was not expanded by this implementation.
