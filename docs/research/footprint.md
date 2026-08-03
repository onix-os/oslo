# Where the binary goes: a costed record

oslo is not slow. It is large. This document says where the megabytes are, what was done about
them, what should be done next, and — the part that stops this being re-derived every quarter —
what was measured and found *not* worth doing, with the number that killed it.

Every claim here has the command that produced it. A claim without a number is not in this
document.

## Result

| | binary | Δ | startup | corpus (408) | arith 200k | parse 3000x | glob 12k ×10 |
|---|---|---|---|---|---|---|---|
| before | 22,971,144 | — | 1.21 ms | 4.17 s | 0.756 s | 0.050 s | 0.045 s |
| **after (shipped)** | **18,326,960** | **−4.43 MiB** | 1.21 ms | 4.17 s | 0.756 s | 0.050 s | 0.045 s |
| bash 5.x, same machine | 1,540,520 | | 0.91 ms | — | 0.456 s | 0.021 s | 0.075 s |

One line of `Cargo.toml`, no source change, no measurable effect on anything that runs. The speed
columns are the *same build* — they are there to show nothing moved, and they are the same numbers
before and after because the code that was deleted was never called.

The `before` binary size is the figure two prior sessions independently produced from a clean
`cargo build --release` on this tree. It was not re-measured here, to stay inside a two-build
budget; the section-by-section deltas below reconcile with it to within 23 KB, which is the
corroboration.

## Where the 21.9 MB went

`cargo bloat --release --crates -n 60` on the *before* binary, `.text` only, plus `size -A -d` for
the sections. `.text` was 17,113,154 B = 16.32 MiB of the 21.91 MiB file.

| crate | .text | share | status |
|---|---:|---:|---|
| `turso_core` | 6.30 MiB | 38.6% | stays (see below) |
| `tantivy` | 778.8 KiB | 4.7% | **gone** — `fts` |
| `[Unknown]` (zstd C objects, simsimd) | 663.2 KiB | 4.0% | mostly **gone** |
| `bitpacking` | 546.9 KiB | 3.3% | **gone** — tantivy |
| `turso_parser` | 529.5 KiB | 3.2% | stays |
| `zstd-sys` | 318.1 KiB | 1.9% | **gone** — mimalloc/tantivy |
| `tantivy-columnar` | 254.7 KiB | 1.5% | **gone** |
| `regex` family (`regex_automata` 374.7 + `regex_syntax` 187.5 + `aho_corasick` 142.7 + `regex` 9.9) | 714.8 KiB | 4.3% | stays |
| `brush_parser` | 198.1 KiB | 1.2% | stays |
| `full_moon` | 184.1 KiB | 1.1% | stays |
| `rustyline` | 142.3 KiB | 0.9% | stays |
| `rust_stemmers` | 128.5 KiB | 0.8% | **gone** — tantivy |
| `hashbrown` | 101.9 KiB | 0.6% | stays |
| `icu_collator` | 81.9 KiB | 0.5% | stays (turso) |
| `num_bigint` | 61.4 KiB | 0.4% | stays (turso) |
| `chrono` | 61.0 KiB | 0.4% | stays (turso) |
| `tantivy-fst` | 60.7 KiB | 0.4% | **gone** |
| `libmimalloc-sys` | 58.4 KiB | 0.4% | **gone** |
| `rayon_core` | 45.4 KiB | 0.3% | **gone** — tantivy |
| `tantivy-sstable` | 42.7 KiB | 0.3% | **gone** |
| `serde_json` | 48.6 KiB | 0.3% | stays |
| `sha2` | 11.1 KiB | 0.1% | stays |
| `tempfile` | 7.3 KiB | <0.1% | stays |
| `nix` | 5.3 KiB | <0.1% | stays |
| `which` | <4.4 KiB | <0.1% | stays (below the cutoff) |
| **oslo's own code** | **1.80 MiB** | **11.0%** | — |
| std, `tokio`, `aes`/`aegis`, `uuid`, `roaring`, `bigdecimal`, `time`, misc | balance | | |

Read the last two rows first. **oslo's own code is 1.8 MiB of a 21.9 MiB binary.** Broken down by
module, from an `nm` sum over `.text`:

```
lua 536 KiB (api 269 / eval 218 / other 49)   builtins 260   interactive 224
startup 129   track 121   expand 109   parser+lexer 90   exec 84
data 50   direnv 45   other 57                        = 1,707 KiB
```

**This corrects the brief's model.** The working assumption was "~8 MB is a Rust shell, the other
~14 MB is Lua, the structured pipeline and turso". The measurement says otherwise:

* Lua costs **0.72 MB** — `full_moon` 184 KiB plus `oslo::lua` 536 KiB.
* The structured pipeline costs **0.05 MB** — that is all of `oslo::data`.
* turso costs roughly **13 MB** of the *before* file: 61% of `.text`, and essentially 100% of
  `.rodata`. `size -A` on the shipping binary showed `.rodata` at 2,097,304 B against 2,090,488 B
  for a standalone turso benchmark binary — every byte of oslo's read-only data is turso's ICU
  collation and compression tables.

There is nothing to cut in Lua or the pipeline. Do not put them on the chopping block for size.

### Sections, before → after

| section | before | after | Δ |
|---|---:|---:|---:|
| `.text` | 17,113,154 | 13,400,914 | −3,712,240 |
| `.rodata` | 2,247,416 | 2,097,304 | −150,112 |
| `.eh_frame` | 1,265,576 | 1,024,512 | −241,064 |
| `.rela.dyn` | 849,912 | 652,848 | −197,064 |
| `.data.rel.ro` | 672,896 | 474,504 | −198,392 |
| `.gcc_except_table` | 591,044 | 504,868 | −86,176 |
| `.eh_frame_hdr` | 151,228 | 115,620 | −35,608 |
| sum of deltas | | | −4,620,656 |
| file | 22,971,144 | 18,326,960 | −4,644,184 |

## What was applied

### `turso = { default-features = false }` — −4.43 MiB, shipped

turso 0.7.2 defaults to `["mimalloc", "fts"]`. oslo asks for neither.

* `fts` is `tantivy`, a full-text search engine. `grep -rn 'MATCH\|fts\|FTS' src/track/ src/startup/history_db.rs` returns two hits, both test-function names about path matching. There is no FTS table in the schema (`src/track/db.rs:63-95`).
* `mimalloc` is a C allocator, and it drags `zstd-sys` with it.

Measured on this tree:

```
binary        22,971,144 -> 18,326,960 bytes     (-4,644,184, -20.2%)
cargo tree -e normal, unique crates   287 -> 236
strings target/release/oslo | grep -ci {mimalloc,zstd,tantivy,stemmer}   ->  0 0 0 0
```

`make verify` exits 0; `cargo test`, `cargo test --test differential_tests` and
`cargo test --test posix_stays_on_the_byte_path` all pass. Startup, corpus, arithmetic, parsing
and glob are all unchanged, which is expected — this deletes code that was never reached.

**But it does not get the C toolchain out, and that is the correction worth recording.** The
manifest comment above `full_moon` says `mlua` was dropped because it "vendored and compiled 30k
lines of C into the binary, so a static musl build needed a C toolchain", and the comment above
`sha2` says `sha2` was chosen because it is "Pure Rust, so it does not cost the static musl build a
C toolchain". turso was added two lines below and silently reintroduced exactly that. Turning the
default features off removes the `mimalloc` and `zstd` C — but not all of it:

```
$ cargo tree -i simsimd -e normal
simsimd v6.5.16
└── turso_core v0.7.2 -> turso v0.7.2 -> oslo

$ grep -n -A2 'simsimd' .../turso_core-0.7.2/Cargo.toml
[target.'cfg(not(any(target_family = "wasm", all(target_os = "windows", target_arch = "aarch64"))))'.dependencies.simsimd]

$ head target/release/build/simsimd-*/output
CC = Some(gcc)
$ nm -g --defined-only target/release/build/simsimd-*/out/libsimsimd.a | head
0000000000000000 T simsimd_bf16_to_f32
...
$ ls -la target/release/build/simsimd-*/out/libsimsimd.a
275096 bytes
```

`simsimd` is a **non-optional** dependency of `turso_core` on every target except wasm and
Windows-on-ARM. It is a C SIMD distance-function library, it invokes `gcc`, and 275 KB of C
archive is linked into every oslo build. Note it is not named `*-sys`, which is why a grep for
sys crates misses it. **No feature flag can remove it. Only removing turso can.**

## Should turso be replaced?

Not yet — but the answer is a real "not yet", not a "no", and the reason is that the two big
arguments for it were both weaker than they look while the third is stronger.

### The contract the store must satisfy

Anything replacing turso has to do all of this. This is the specification, read off
`src/track/{db,write,query,prune,private}.rs` and `src/startup/history_db.rs` (3,550 lines total).

1. **Two ordered key ranges, at keystroke latency.** `run(dir_id, mode, argv)` for "what did I run
   here that starts like this", and `dir(base)` for "which remembered directory did that keyword
   mean". `src/track/query.rs` is explicit that these are half-open *range scans*, never `LIKE`,
   and that the resulting headroom is what makes a cache — and therefore a cache-coherence bug
   between terminals — unnecessary. Measured at 13 µs against 25,000 rows today.
2. **A secondary range on `run(mode, argv)` and a join through `dir.root`,** for the
   widen-to-worktree suggestion (`SUGGEST_IN_WORKSPACE`).
3. **Upsert-with-counters.** `write.rs` increments `runs`/`fails` and rewrites `last_at` on a
   unique key `(dir_id, mode, argv)`. This is what makes the store an aggregate rather than a log,
   which `src/track/mod.rs` argues for at length — repeats cost nothing after the first.
4. **Cascading delete.** `run.dir_id REFERENCES dir(id) ON DELETE CASCADE`; dropping a vanished
   directory takes its runs with it.
5. **A per-directory cap, applied as a sweep.** `RUNS_PER_DIR = 500`, enforced by `OVER_CAP` in
   `prune.rs` — a correlated subquery with `LIMIT MAX(0, COUNT(*) - ?2)`. Plus age-based
   expiry (`RUN_MAX_AGE` 90 d, `GONE_MAX_AGE` 30 d) and a daily stamp in `meta`.
6. **Multi-process concurrency.** Several terminals write the same file at once. turso opens WAL
   without being asked (`history_db.rs:123`) and that is what lets a second terminal read while a
   first writes.
7. **Crash safety.** A half-written store must not lose the file.
8. **0600 and a private sidecar, established before the engine sees the path.** `private.rs`
   documents that the `-wal` does *not* inherit the database's mode in turso 0.7.2, so oslo
   creates both files private first. Any replacement's sidecar files inherit this requirement.
9. **A ranking expression shared between SQL and Rust.** `score::score_sql` exists precisely so the
   ordering done in the database and the ordering done in Rust cannot drift (`query.rs:15-17`).
10. **A separate, simpler store for history:** `history_db.rs` is three columns, `INSERT`, and
    `ORDER BY id DESC LIMIT ?1`.
11. **A bounded file.** turso 0.7.2 has neither `VACUUM` nor `auto_vacuum`, so growth is a
    permanent high-water mark; the aggregate design is the workaround.
12. **Synchronous callers.** oslo's REPL is not async. Every call today goes through
    `runtime().block_on(...)` against a `OnceLock<tokio::runtime::Runtime>`
    (`track/db.rs:99`, `startup/history_db.rs:57`).

Nothing in this list is on the POSIX byte path. `tests/posix_stays_on_the_byte_path.rs` and the
differential corpus never touch the store, so a replacement cannot break byte-for-byte
compatibility with bash. That is the single most important property of this work item.

### Candidates, measured

One scratch project, one binary per candidate, identical workload: 25,000 rows keyed
`(dir_id, mode, argv)`, one half-open prefix range scan, profile `lto="thin"`,
`codegen-units=1`, `strip="symbols"`. `stat -c%s` and a median-of-200 scan.

| candidate | binary | Δ over empty | range scan | write 25k | pure Rust |
|---|---:|---:|---:|---:|---|
| empty binary (floor) | 0.35 MB | — | — | — | — |
| append-only log + `BTreeMap` | 0.40 MB | 0.05 MB | 0.1 µs | 2 ms | yes |
| `sanakirja` | 0.50 MB | 0.15 MB | 0.9 µs | 10 ms | yes |
| `heed` (LMDB) | 0.50 MB | 0.15 MB | 0.8 µs | 9 ms | **no** — `lmdb-master-sys` |
| `persy` | 0.97 MB | 0.62 MB | 18.5 µs | 517 ms | yes |
| **`redb` 3.1.3** | **1.02 MB** | **0.67 MB** | **4.9 µs** | **25 ms** | **yes** |
| `sled` 0.34 | 1.17 MB | 0.82 MB | 13.5 µs | 84 ms | yes, but see below |
| `native_db` 0.8 | 1.28 MB | 0.93 MB | 8.3 µs | 52 ms | yes (`redb` inside) |
| `fjall` 2.11 | 1.29 MB | 0.93 MB | 8.6 µs | 11 ms | yes |
| `rusqlite` bundled | 2.33 MB | 1.98 MB | 21.0 µs | 32 ms | **no** — `libsqlite3-sys` |
| turso, no defaults | 13.67 MB | 13.32 MB | 69.1 µs | 362 ms | **no** — `simsimd` |
| turso, as oslo shipped it | 18.14 MB | 17.79 MB | 66.1 µs | 325 ms | **no** — `+ mimalloc`, `zstd` |

(The SQL candidates report 1 hit because oslo's real query ends in `LIMIT 1`; the key-value ones
report the whole 100-row range.)

### Recommendation: `redb`, as a planned piece of work, not a cleanup

**Cost:** roughly 2,400 lines of `src/track/` plus `src/startup/history_db.rs` rewritten from SQL
into Rust. Call it a week, and the two places where hand-written Rust can silently diverge from
what the SQL did are `OVER_CAP`'s correlated subquery and the `score_sql` shared ranking — item 9
of the contract *dies* with the SQL, and something has to replace the guarantee it was making.

**Saving:** turso's standalone delta is 17.79 MB against `redb`'s 0.67 MB. Applied to today's
18.3 MB binary that lands oslo somewhere in **4–6 MB** — a range, not a point, because turso
shares `regex`, `serde_json`, `thiserror`, `memchr`, `smallvec` and `hashbrown` with oslo and thin
LTO already dedupes those. It also unlocks the `regex-lite` swap (a further 1.34 MB, see below),
which nothing else unlocks.

**Speed:** faster, on the exact query oslo runs. 4.9 µs against turso's 66.1 µs, 13×. The
`directories_ranked` scan that `query.rs` documents as "1.07 ms over 3000 directories" measures
52.2 µs written as a plain Rust scan — 20× faster than the SQL. And `redb` is synchronous, so
`tokio`, both `OnceLock<Runtime>`s and every `block_on` in the tracker go with it.

**Why `redb` over the two smaller ones.** `cargo tree -e normal` on `redb` prints exactly two
crates against turso's 236. `heed` is 0.5 MB smaller but is LMDB behind `lmdb-master-sys` — C,
which is the whole point of doing this. `sanakirja` is also 0.5 MB smaller but has an unsafe-heavy
API, which is a poor trade for 0.5 MB in a `/bin/sh`. `sled` should not be chosen at any size:
0.34 is the last real release and 1.0 has been unreleased for years. `rusqlite` with `bundled`
compiles SQLite's C amalgamation — reintroducing precisely what this project left behind — and at
2.33 MB it is not even the smallest option.

**Why not now.** It is the largest single item in the binary by an order of magnitude, and it is
also the only item on this page that can produce a wrong answer. It needs a plan, a schema
migration for existing `~/.local/share/oslo` stores, and its own test pass. It is not a cleanup and
was deliberately not attempted here.

**`history_db.rs` needs no store at all.** Three columns, `INSERT INTO history (line, mode, at)`,
`ORDER BY id DESC LIMIT ?1`. That is an append-only file read backwards, and it can be done
independently of and before the `redb` work.

## What else is worth doing, best ratio first

### 1. Dependency-only `opt-level = "s"` — −2.32 MiB for +14% parsing. **A decision, not a win.**

Built and measured on this tree. Not applied.

```toml
[profile.release.package."*"]   # "*" = every non-workspace package, i.e. exactly the deps
opt-level = "s"
```

```
binary        18,326,960 -> 15,893,936 bytes   (-2,433,024, -2.32 MiB, -13.3%)
.text         13,400,914 -> 10,663,250
```

Interleaved A/B, five alternating pairs each, both binaries kept side by side:

```
arith 200k    O3-deps 0.756 s (median)   Os-deps 0.777 s (median)    +2.8%  SLOWER
parse 3000x   O3-deps 0.050 s (median)   Os-deps 0.057 s (median)   +14.0%  SLOWER
corpus 408    O3-deps 4.194 s            Os-deps 4.208 s             +0.3%  noise
startup 200×  O3-deps 1.38 ms            Os-deps 1.39 ms             unchanged
glob 12k ×10  O3-deps 0.045 s            Os-deps 0.042 s             unchanged
```

The parse regression has a mechanism: `brush-parser` is a dependency, so it gets `-Os`, and thin
LTO inlines it into oslo's hot path. That is exactly the code the runtime analysis identified as
oslo's weakest area — parsing is already 8× bash and 70% of it is oslo's own brush→oslo adapter.
Making it 14% slower to save 2.3 MiB is the owner's call, so it is written down rather than taken.

Two claims from the earlier survey do **not** reproduce for the deps-only form and should not be
relied on: "startup −19%" and "corpus 3.4% faster" were properties of the *global* `-Os` build
against the 22.9 MB baseline, not of this one.

**The refinement that may make it free, untested for want of build budget:**

```toml
[profile.release.package."*"]
opt-level = "s"

[profile.release.package.brush-parser]
opt-level = 3
```

`brush_parser` is 198.1 KiB of `.text` at `-O3`, so the cost of the exemption is bounded at a
couple of hundred kilobytes against 2.3 MiB saved. If it recovers the parse loop this becomes an
unambiguous win and should ship. **One release build plus the two microbenchmarks above settles
it.** Global `opt-level = "s"` is separately known to cost 31% on the arithmetic loop and should
not ship in any form.

### 2. `panic = "abort"` + `-C force-unwind-tables=no` — up to −1.57 MiB, ~30 lines

Unwinding costs, in the *shipped* binary:

```
.eh_frame            1,024,512
.eh_frame_hdr          115,620
.gcc_except_table      504,868
                     ---------
                     1,645,000 B = 1.57 MiB = 9.0% of the binary
```

The `Cargo.toml` comment rejects `panic = "abort"` on the ground that `capturing()` in
`startup::environments` must restore stdout and stderr before a panic escapes. Reading the code,
that ground is thinner than it sounds. `grep -rn 'catch_unwind\|resume_unwind' src/ tests/` returns
exactly two lines, both inside that one function, and the second is
`std::panic::resume_unwind(panic)` with nothing above catching. **The shell dies either way.** The
unwinder is buying exactly one thing: that the panic message lands on the real stderr rather than
in the scratch tempfile the descriptors were `dup2`'d onto.

A panic hook buys that under `abort`. In `capturing()`, after the two `dup()` calls, install a
`std::panic::set_hook` closure holding `saved_err` and have it write the `PanicHookInfo` to that
raw descriptor; restore the previous hook on the way out.

Caveats not to skip:

* `set_hook`/`take_hook` are process-global and oslo runs two threads (`src/main.rs:94`). The hook
  is installed and removed around every `cd`, so the install/remove must be cheap and must not
  race the interpreter thread.
* `panic = "abort"` alone removes `.gcc_except_table` (505 KiB) and the landing pads inside
  `.text`. `-C force-unwind-tables=no` is what removes the remaining 1.14 MiB of
  `.eh_frame`/`.eh_frame_hdr`.
* The test profile keeps unwind, so `cargo test` is unaffected.

The user-visible behaviour to preserve is "a panic in a `.env.lua` prints a message rather than
dying mute", and the hook covers it.

### 3. `regex` → `regex-lite` — −1.34 MiB, but **blocked on turso**

`regex_automata` 374.7 + `regex_syntax` 187.5 + `aho_corasick` 142.7 + `regex` 9.9 = 714.8 KiB of
`.text`. A standalone A/B exercising every API oslo uses — `RegexBuilder` with
`dot_matches_new_line`/`multi_line`/`case_insensitive`, `captures`, `is_match`, `find_iter`,
`replace_all`, `replacen`, `split`, `escape` — measured 1,872,672 B against `regex-lite`'s
471,808 B: **1.34 MiB**.

The saving today is **zero**, and the reason is worth stating precisely because the earlier survey
got it slightly wrong. It attributed the block to `tantivy`; `tantivy` is now gone and `regex` is
still here:

```
$ cargo tree -i regex -e normal
regex v1.13.1
├── oslo
└── turso_core v0.7.2 -> turso -> oslo
```

`turso_core` depends on `regex` **directly**. So this unlocks when turso leaves entirely, not
before. Sequence it after the store work.

When the day comes it is not free: `regex-lite` has no Unicode character classes and its `\d`,
`\w`, `\s` are ASCII-only. For `[[ =~ ]]` that is arguably *closer* to bash (bash has no `\d` at
all), but any `oslo.re` Lua script using `\p{...}` breaks.

### 4. `interactive::spec::definitions::all` — −17 KiB, mentioned only so nobody re-derives it

875 lines of Rust building what could be a `&[(&str, &[&str])]` const. 21.8 KiB now, ~5 KiB as
data. This is the largest oslo-side size item in the entire binary and it is 0.1%. It is on this
list to close the question, not to open it.

## Measured and NOT worth doing

This section exists so the same ground is not re-walked. Each line is a real measurement.

| thing | number | why not |
|---|---|---|
| **Non-PIE** (`-C relocation-model=static`) | `.rela.dyn` 652,848 + `.got` 21,240 = **658 KiB**, plus part of `.data.rel.ro` | **Declined on principle.** This is going to be `/bin/sh` in a distribution — the single most-executed binary on the box. Shipping it without ASLR is a security regression a distro will reject. 658 KiB is not worth it. |
| **Global `opt-level = "s"`** | −7.61 MiB (measured against the 22.9 MB baseline) but **+31%** on the arithmetic loop | Hits oslo's own arithmetic evaluator and parser. The deps-only form above is the survivable version of this idea. |
| **Trimming `nix` features** | `nix` is **5.3 KiB** of `.text` total, and all seven features have call sites (`process` 14, `signal` 24, `term` 9, `fs` 9, `user` 12, `resource` 4, `hostname` 7) | Nothing there. Could not matter even if a feature were unused. |
| **Removing `which`** | below `cargo bloat -n 60`'s cutoff, **<4.4 KiB** | Invisible. |
| **Removing `tempfile`** | **7.3 KiB**, and turso depends on it anyway | Invisible, and not removable. |
| **Replacing `full_moon`** | **184.1 KiB** for a complete Lua 5.4 parser | Cheap for what it is. |
| **Replacing `rustyline`** | **142.3 KiB** | Cheap for what it is. |
| **Stripping `#[derive(Debug)]` from oslo's types** | oslo's own `Debug` code is **1 KiB**. The 668 KiB of `Debug` in the binary is 113 KiB named turso types plus ~547 KiB of `Vec<T>`/`Box<T>`/`Option<T>`/`Arc<T>` wrappers over turso types | It is not your `Debug`. |
| **Large oslo static tables** | There are none. Every large `.rodata` symbol in the *before* binary was zstd, crc32, ryu or Unicode from turso's subtree | Nothing to find. |
| **Cutting Lua for size** | `full_moon` 184 KiB + `oslo::lua` 536 KiB = **0.72 MB**, 3.3% of the before-binary | The premise was wrong. See "Where the 21.9 MB went". |
| **Cutting the structured pipeline for size** | `oslo::data` is **50 KiB**, 0.2% | Same. |
| **Removing the 16 MB interpreter stack / second thread** | `src/lib.rs:13`, spawned at `src/main.rs:94` | `main.rs:85-101` documents the signal-ordering bug that removing them brings back. It is deliberate. Do not touch it as part of size work. |
| **Chasing startup** | `strace -c` on `-c ':'`: oslo **186** syscalls, bash **169**. No regex compiled at startup; both tokio runtimes are lazy and are not built for `-c` | Already competitive. oslo starts in 1.21 ms against bash's 0.91 ms and the gap is mapping a larger binary, which is what the size work fixes. |
| **Chasing expansion** | Long `$@` re-expanded 50×: oslo **1.4× faster** than bash. String accumulation in a loop at n=8000: oslo **33× faster** — bash is quadratic there, oslo is linear. No O(n²) anywhere in `src/expand/` | Nothing to win. |
| **Chasing glob** | oslo 0.045 s against bash 0.075 s on 12,000 files × 10 | Already 1.7× ahead. |
| **Using the corpus total as a speed signal** | Timed per script on the shipping binary: **median 2 ms, p90 5 ms**, and the six slowest scripts are 3.270 s of the 4.17 s total (**78%**) — `job_jobs_running_format` 1006 ms, `job_fg_bg_without_job_control` 1006, `job_background_reaped` 421, `job_wait_jobspec` 413, `syntax_process_substitution` 216, `job_jobs_empty` 208. Everything else is startup-bound | The headline number is ~78% deliberate `sleep` plus the `kill %1` bug and cannot move whatever is optimised. Report median and p90 instead. |

### One runtime bug found while measuring, worth fixing on its own merits

`kill %1` is unimplemented. `src/env/builtins/process/kill.rs:47` only does
`operand.parse::<i32>()`, so every `%`-form falls through to the diagnostic on line 52. Reproduced
directly — `sleep 1 & ; kill %1; wait`:

```
oslo: oslo: kill: `%1': not a pid or valid job spec     1.005 s
bash: (silent)                                          0.002 s
```

It is invisible in the suite only because the corpus script that exercises it redirects stderr and
then blocks in `wait` — which is why `job_jobs_running_format.sh` takes **1006 ms** and is joint
slowest in the corpus. The resolver it needs already exists and is
already used by `wait`: `resolve_job()` in `src/env/builtins/jobs/wait.rs:177`. Note bash signals
the job's whole process *group* (negative pgid) for a jobspec, not just the leader. Small, low
risk, and a genuine bash-compat divergence — but it is a behaviour change, so it is written here
rather than done.

## Where the size actually is now, and what to do about it

```
18.3 MB today
 -2.3 MB   deps-only -Os, IF the brush-parser exemption holds        (1 build to confirm)
 -1.6 MB   panic=abort + no unwind tables                            (~30 lines)
-13   MB   redb instead of turso                                     (~1 week)
 -1.3 MB   regex-lite, unlocked only by the line above
 ------
 ~4 MB, which is roughly brush's 8.16 MB halved, with Lua in it
```

The first two are configuration and one function. The third is the whole game and is a decision,
not a chore. The fourth follows the third.

## Discipline notes

Two release builds were used, the limit. `df -h /home` read 224 G free before each and after both.
No scratch project was created in this tree; the earlier candidate benchmarks were built under
`/tmp` and deleted. Nothing was committed.

The stray top-level files `a`, `b`, `c`, `x`, `top`, `out`, `directory`, `adir` predate this work
and are gitignored, but they look exactly like the corpus littering `make corpus` warns about and
someone should sweep them.
