# Performance and cleanup: what was checked, and what was true

Every claim in `REPORT.md` was re-checked against the code and, where it was a performance claim,
against a measurement on a **release** build. Roughly half did not survive.

This document keeps the refuted half as well as the confirmed half, deliberately. A wrong finding
that is merely deleted gets rediscovered and re-investigated six months later; one that is written
down with the number that disproves it does not.

## The baseline

Measured on this machine, release build. Every figure here is reproducible with the command beside
it — a number nobody can re-run is a rumour.

| | | how |
|---|---|---|
| startup | **1.8 ms** (bash: 2.0 ms) | `time (for i in $(seq 1 20); do ./target/release/oslo -c true; done)` |
| one corpus script | **9.3 ms** | `time (for i in $(seq 1 50); do ./target/release/oslo tests/corpus/builtin_read_options.sh; done)` |
| whole corpus | **4.76 s** (~408 scripts) | `time (for f in tests/corpus/*.sh; do ./target/release/oslo "$f"; done)` |
| syscalls per keystroke | **7.8**, zero subprocesses | `scratchpad/keystroke.py` (straces a real pty session) |
| release binary | 29.9 MB unstripped | `ls -l target/release/oslo` |

**oslo starts faster than bash.** That is the number that matters most for a `/bin/sh`, and it is
already good; the effort is better spent elsewhere.

## Confirmed, and fixed

### 1. `fuzzy_score` refolded the typed pattern for every candidate

`src/interactive/matching.rs`. Ranking a Tab press scores one short pattern against every
executable on `$PATH` — ~3,300 here — and each call rebuilt the *same* pattern into a `Vec<char>`.

Fixed with `Fuzzed::new(typed, fuzzy)`, which folds once for the batch. Measured with
`cargo bench --bench fuzzy`:

```
fold per candidate   : 436 µs
fold once per press  : 269 µs
saved                : 166 µs (38%)
```

Honest framing: 38% of something that was never the problem. 436 µs per Tab press is imperceptible.
It is kept because it is free and the code is simpler, not because anyone could feel it.

### 2. `[[ =~ ]]` recompiled its pattern on every evaluation

`src/env/builtins/conditionals/matching.rs`. Compiling is the expensive half of a regex by a wide
margin, and a script filtering lines in a loop paid for an identical build every iteration.

```
2000 evaluations, before : 30 ms   (loop alone: 10 ms → 10 µs per match)
2000 evaluations, after  : 10 ms   (≈ the loop alone → ~1 µs per match)
```

**A ~10× on that construct**, and the one confirmed finding that matters for a `/bin/sh`: it is
paid by scripts, not by a person typing. Fixed with a bounded thread-local cache — thread-local
rather than a global with a lock, because a mutex on the execution path trades one cost for
another; bounded because `=~ "^$prefix"` inside a loop would otherwise grow it without limit.

### 3. `tracing` and `tracing-subscriber` were unused

Both removed from `Cargo.toml`. Nothing in `src/` referenced either; the four grep hits were
comments about shell `set -x` tracing. `tracing` remains in the tree transitively via `turso`, so
this removes one direct dependency and all of `tracing-subscriber`'s.

## Refuted, with the measurement that refutes it

### `git_branch()` runs on every keystroke — **false, and it was the headline**

The claim: `right_prompt_escape()` walks up the directory tree and reads `.git/HEAD` per keypress.

`right_prompt_escape(right: &str, …)` takes an **already-rendered string**; it cannot call
`git_branch()`. The render happens in `src/startup/read.rs:91`, once per prompt. Measured over 16
keystrokes in a real pty:

```
syscalls while typing : 124  (7.8 per keystroke)
  touching .git       : 1
  execve              : 0
```

One `.git` stat for the whole line, not one per key. Caching it would have saved nothing and added
an invalidation bug.

### Dead code that is not dead

Each of these is called; the report's line references point at the definition, not at the callers.

| claimed unused | actually |
|---|---|
| `find::owner` | called twice, `src/direnv/mod.rs:164` and `:291` |
| `JobManager` import in `repl.rs` | used at `repl.rs:154` |
| `Invocation::login` | read at `src/env/builtins/exec.rs:120` |
| `Val::Bytes` / `Duration` / `Time` | constructed 2, 3 and 3 times outside `value.rs` |

`Diff::is_empty` **is** genuinely uncalled — it exists because clippy denies `len` without
`is_empty`, and the lint is right. `Diff::len` is used only by its own test. Both are three lines;
removing them costs more in churn than it saves.

## Not measured, and therefore not claimed

Named so nobody assumes they were cleared:

- **glob `Vec<char>` per filename** (`src/expand/glob/compile.rs`) — plausible and unmeasured. The
  right experiment is a directory of 10,000 files and a wildcard, timed before and after.
- **Completion's four matcher passes** each re-locking the environment and re-scanning `$PATH`.
  Real by construction — the chain is first-non-empty-wins — but the cost is unmeasured, and Tab
  latency is already 436 µs, so the ceiling on the win is small.
- **Frecency scoring inside `sort_by`**, taking a lock per comparison. Same: real shape, unmeasured
  size, small ceiling.
- **`block_on` for the history and tracking writes** after each command. The design note claims
  81 µs; nobody has verified it, and the interesting question is not the write but whether it lands
  before the prompt returns.
- **Binary size.** 29.9 MB unstripped is large for a static `/bin/sh`. Nothing here established
  what is in it — `cargo bloat` or `bloaty` would, and `[profile.release]` has no `lto`, `strip` or
  `codegen-units` settings at all, which is the first thing to try and the easiest to measure.

## Repository hygiene

The corpus scripts create files **in whatever directory they are run from**, so running them by
hand litters the repository root — 184 stray files at one point, including `tests/corpus/` itself.

`tests/posix_stays_on_the_byte_path.rs` already sandboxes each script into a temp directory, and
its comment records why: a script dropping a file called `f` changed what a *different* script did
afterwards. `tests/differential_tests.rs` and any ad-hoc runner should do the same. This is the one
finding in `REPORT.md` whose severity was understated: it is not clutter, it is cross-contamination
between tests.
