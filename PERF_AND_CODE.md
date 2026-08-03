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
| Enter → next prompt | **0.46 ms** median (p90 0.48) | `scratchpad/roundtrip.py` |
| Tab press | **1.42 ms** median (p90 1.90) | `scratchpad/tab.py` |
| glob over 12,000 files | **2.1 ms** (bash: 6.0 ms) | 100 globs in one shell, timed |
| release binary | **21.9 MB** (was 29.9) | `ls -l target/release/oslo` |

**oslo starts faster than bash, and globs nearly three times faster.** Those are the numbers that
matter most for a `/bin/sh`, and they were already good before any of this work. Every interactive
figure above is an order of magnitude below the ~50 ms a person can perceive.

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

### 3. `[profile.release]` did not exist

The release profile was entirely default: no `lto`, no `codegen-units`, no `strip`.

| | binary | startup | corpus |
|---|---|---|---|
| before | 29.9 MB | 1.8 ms | 4.76 s |
| `lto = "thin"`, `codegen-units = 1`, `strip = "symbols"` | **21.9 MB** | **1.5 ms** | **4.38 s** |

**27% smaller and 8% faster**, which is the largest single win here. The cost is build time: about
4½ minutes instead of 30 seconds, paid by whoever cuts a release and by CI, never by a user.

`panic = "abort"` would have taken it to 19.8 MB and is deliberately **not** set: `catch_unwind` in
`startup::environments` restores stdout and stderr before re-raising, and under `abort` that never
runs — a panic inside a `.env.lua` would kill the shell with its own message written into a temp
file nobody reads. 2 MB is not worth a silent death.

### 4. `tracing` and `tracing-subscriber` were unused

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

## Measured, and refuted

Everything left over from `REPORT.md` has now been measured. None of it survived.

### glob allocates a `Vec<char>` per filename — real, and irrelevant

`src/expand/glob/compile.rs` does allocate per entry. Against a directory of 12,000 files, 100
globs in one shell:

```
oslo : 0.21 s   (2.1 ms per glob)
bash : 0.60 s   (6.0 ms per glob)
```

**oslo is 2.9× faster than the reference implementation.** The allocation is real and the code
could be tightened, but optimising the thing that is already winning by 3× is not where the next
hour goes.

### `block_on` for the history and tracking writes blocks the REPL — refuted

Both writes happen between the command finishing and the prompt returning, so if they cost anything
a person feels it exactly there. Measured with a warm store over 60 commands:

```
Enter → next prompt : 0.46 ms median, 0.48 ms p90, 0.74 ms max
```

That is the *whole* round trip — command, history write, tracking write, prompt render — at about
one percent of the ~50 ms a person can perceive. Moving the writes to a background channel would
add an ordering-bug class for no gain anyone could feel.

### Four matcher passes and a lock per frecency comparison — refuted

Both are real by construction: the matcher chain is first-non-empty-wins, and the sort takes a lock
per comparison. Measured end to end over 25 Tab presses:

```
Tab press : 1.42 ms median, 1.90 ms p90
```

Candidate generation, ranking and drawing together are under two milliseconds. The
`Fuzzed`/`fold-once` change above removed 166 µs of that; the remainder is not worth restructuring
the candidate pipeline for.

## How big is that, next to other shells

Measured on this machine, same day. Binary size is the fairest single column: nix closure sizes
include every runtime dependency down to libc and are not comparable with a system package's.

| shell | binary | closure / +libs | what it is |
|---|---:|---:|---|
| dash | 0.12 MB | 2.9 MB | C, POSIX only, no interactive anything |
| zsh | 0.93 MB | 4.8 MB | C |
| bash | 1.06 MB | 3.9 MB | C |
| brush | 8.16 MB | 54 MB | Rust — oslo uses its parser |
| hilbish | 9.39 MB | 48 MB | Go, Lua-scripted |
| fish | 11.90 MB | 306 MB | C++/Rust |
| **oslo** | **21.91 MB** | 26 MB | Rust, Lua, turso |
| nushell | 66.50 MB | 113 MB | Rust, structured data |

Reproduce with `stat -c%s "$(command -v <shell>)"`, and for the nix ones
`nix path-info -S nixpkgs#<shell>`.

**oslo is second largest, and a third of nushell** — which is the closest comparison, being the
other Rust shell with a structured pipeline. It is 20× bash, and that is the number worth sitting
with: bash is the thing it intends to replace as `/bin/sh`.

The gap is not the shell. `brush` is oslo's own parser plus a shell around it at 8.16 MB, so
roughly 8 MB of the 22 is "a Rust shell" and the remaining ~14 MB is what oslo adds: the Lua
interpreter, the structured pipeline, and `turso` — which drags in tantivy, icu, zstd, simsimd and
aegis, a full-text search engine and a unicode collation library, to store command history.

That last one is the whole question. Nothing else in the list ships a search engine.

## What is actually left

Nothing measured here is worth doing. If more performance is ever wanted, the honest order is:

1. **Binary size beyond the profile** — the only lever left, and it is a design question rather
   than an optimisation. See the comparison above: ~8 MB is "a Rust shell" and ~14 MB is what oslo
   adds on top, most of it `turso`'s transitive tree. Two honest options, neither yet costed:
   measure with `cargo bloat --release` first, then either turn off turso features that a history
   store cannot need (full-text search, vector similarity), or store history in something smaller.
   The second is a real loss — the range-scan query is what makes directory-aware suggestion free —
   so it is a trade, not a cleanup.
2. **Nothing else.** Startup beats bash, globbing beats bash by 3×, and every interactive path is
   between one and two milliseconds.

## Repository hygiene

The corpus scripts create files **in whatever directory they are run from**, so running them by
hand litters the repository root — 184 stray files at one point, including `tests/corpus/` itself.

`tests/posix_stays_on_the_byte_path.rs` already sandboxes each script into a temp directory, and
its comment records why: a script dropping a file called `f` changed what a *different* script did
afterwards. `tests/differential_tests.rs` and any ad-hoc runner should do the same. This is the one
finding in `REPORT.md` whose severity was understated: it is not clutter, it is cross-contamination
between tests.
