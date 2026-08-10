# Prediction and repair

Two features from one library. `vista` predicts what you are about to type and rebuilds what you
got wrong, and both read the same history oslo already keeps.

## Status

- **State:** Phase 1 core landed — the model exists, learns and is tested; not yet reachable from
  the shell. Phases 2–4 planned.
- **Branch:** `feat/predict-and-repair`, off `develop` at `b54823c` (0.2.27)
- **Library:** `vista` 0.1.0, `~/data/code/tools/vista` — MIT, edition 2024, **zero dependencies**
- **Measured cost:** **+340 KB** on the static musl binary, with the two features taken; every
  feature measured separately in the ledger below.
- **MSRV: settled.** vista declared 1.97.1 and did not need it; it now declares 1.89, oslo's own.
- **Dependency form: vendored** at `vendor/vista`, provisionally; see Phase 1.

The previous contents of this file — the Tagdata history plan, marked implemented and audited on
2026-08-09 — are in git at `git show HEAD:PLAN.md`.

## What this buys

**Prediction.** A variable-order PPM model over the commands you have run, blended with a
recent-history cache and adjusted by context, outcome and what you have typed so far. It is the
fourth ghost-suggestion source, beside history, completion and path — and unlike the first, it can
offer a line you have not typed *here* before because it learns the order things happen in.

**Repair.** The same model, asked a different question. `predict_aligned(&query, &failed)` rebuilds
a failed command from the structure of commands history already contains:

```text
you typed:  apt install ripgrep
history:    sudo apt install fd
result:     sudo apt install ripgrep
```

This is what `thefuck` does, without `thefuck`'s method. thefuck carries ~200 hand-written rules,
one per tool, each a Python function that recognises a failure and rewrites it; the rules are the
product and they rot as the tools change. vista has no rules and no dictionary: shared tokens are
structure, tokens only history has are the repair, and differing tokens are decided by what you
have actually typed before. That means it corrects *your* commands — including private tooling no
rule author has ever seen — and it means it can only ever suggest something built from commands you
have really run, which is a much better safety property than a rule engine has.

## MSRV — settled, and it was nothing

vista declared `rust-version = "1.97.1"` against oslo's `1.89`, which looked like a wall: oslo's
MSRV is enforced by a job in `.github/workflows/tests.yml`, and a shell meant to be `/bin/sh` on a
distribution cannot require a compiler released last month — 1.97.1 landed 2026-07-14.

The declared number was not a floor. It was the toolchain vista happened to be written on. Built
against every toolchain to hand, `--all-features`:

| toolchain | result |
|---|---|
| 1.85.0 — edition 2024's own minimum | builds |
| 1.88.0 | builds |
| **1.89.0** | **builds; all 80 tests pass** |
| 1.91 / 1.93 / 1.94 / 1.95 | build |

vista now declares **1.89**, matching oslo exactly. 1.85 was available and was not taken: matching
oslo means a future change that breaks the floor fails in oslo's existing MSRV job rather than at
integration time, which is the only moment anybody would notice.

**oslo does not move.** Nothing was gained by raising it and real portability would have been lost.

## How vista is depended on — decided: vendored

oslo has three precedents and they disagree, so it was picked deliberately:

| form | precedent | fits here? |
|---|---|---|
| `vendor/` | `brush-parser`, `full_moon` | yes if vista is to be frozen and read |
| git dependency | `maki` (the SSH fork) | yes if vista stays a live project |
| path dependency | none in a release build | **no** — a path outside the repo cannot be built by CI |

`vista` is `publish = false`, so crates.io is not an option today. The measurement above used a
path dependency, which works on this machine and would fail in GitHub Actions immediately.

**Taken: `vendor/`.** A git dependency is the better end state and remains the recommendation once
vista's MSRV change is pushed — it is under active development by the same author, and vendoring
means every improvement is a manual copy. It could not be taken *now*: a git dependency would point
at a vista that still declares 1.97.1, and the measurement above used a path dependency that CI
cannot resolve. Vendoring builds today and is two lines to reverse.

## Size

Binary size is an acceptance criterion for oslo, not a cleanup task. The 344 KB above is
**with vista's default features**, which include things this integration may not need:

```
default = ["explanations", "recent-cache", "snapshot", "surface-indexes"]
```

- `explanations` — why a prediction was made. Useful for a debugging tool, not for a ghost.
- `research`, `evaluation` — off by default already; must stay off.
- `snapshot` — needed, unless the model is rebuilt from history at every start (see Phase 2).
- `surface-indexes`, `recent-cache` — measured both ways; see the ledger.

### Ledger

Exact bytes from `target/x86_64-unknown-linux-musl/release/oslo`, not the rounded MB.

All measured with a probe reachable from `main`, vendored, one feature at a time:

| Checkpoint | Bytes | Delta | That feature costs |
|---|---:|---:|---:|
| Baseline (`b54823c`) | 6,327,232 | 0 | |
| vista, no features | 6,630,336 | +303,104 | the crate itself, 296 KB |
| `+ snapshot` | 6,638,528 | +311,296 | 8 KB |
| `+ recent-cache` | 6,675,392 | +348,160 | 36 KB |
| `+ surface-indexes` | 6,777,792 | +450,560 | **100 KB** |
| `+ explanations` (all four) | 6,781,888 | +454,656 | 4 KB |
| **Chosen: `snapshot`, `recent-cache`** | **6,675,392** | **+340 KB** | |
| After Phase 1 core | 6,327,232 | 0 | model unreachable from `main`; LTO drops it |
| After Phase 2 | 6,844,608 | +517,376 | first checkpoint where it is reachable |
| After Phases 3–4 | 6,852,800 | +525,568 | repair and `oslo.predict`, 8 KB |
| After the inline hint | 6,861,024 | +533,792 | drawn correction and `oslo.repair`, 8 KB |

`surface-indexes` is not taken, and Phase 3 is where that was settled rather than assumed. Built
both ways against the case it exists for — a repair whose candidate is not a substring of anything
in history — it cost the measured 100 KB and **did not change the answer**: still empty. What
actually decides whether repair can answer is whether the mistyped line is itself in the model, and
the fix for that is [`predict::ran`], which costs nothing. The feature stays out.

**A dead-code probe measures nothing.** The first attempt read +0 bytes because LTO removed the
unreachable call, and the Phase 1 row above reads 0 for the same reason and honestly: the module
exists and is tested, but nothing in the binary reaches it yet. Every future row must come from a
build where the code is genuinely called.

## What vista offers

```rust
Predictor::new(Config) | ::builder(config) | ::with_components(config, normalizer, tokenizer, matcher)
predictor.observe(Observation) / .replay(iter)
predictor.predict(&Query) -> Vec<Prediction>
predictor.predict_aligned(&Query, &Item) -> Vec<Prediction>   // repair
predictor.probability_of(&Query, &Item) -> f64
predictor.forget(&dyn ItemMatcher) / .clear() / .break_stream(StreamId) / .stats()
predictor.write_snapshot(W) / ::read_snapshot(R, ...)
```

```rust
Observation { item: Item, stream: StreamId, position: Position, timestamp: i64,
              context: Vec<Feature>, outcome: Vec<Feature> }
Query { stream, position, context, partial: Option<String>, limit }
Feature::categorical(name, value) | ::numeric(name, f32)
```

`Config` carries ~25 hard bounds (`max_order`, `max_contexts`, `max_candidates`,
`max_snapshot_bytes`, …). Everything is bounded by construction, which is the property that makes
this safe to run in a shell.

## The mapping

oslo already records exactly what an `Observation` wants. From `track::sync::HistoryEvent`:

| vista | oslo | note |
|---|---|---|
| `item` | `Item::new("command", line)` | the line as typed |
| `stream` | hash of `session` | one shell is one stream; ordering only means something within one |
| `position` | `seq` | already a per-session counter |
| `timestamp` | `recorded_at` | |
| `context` | `cwd`, `root`, `host`, `mode` | categorical; `root` is the git worktree, which is the strongest signal a shell has |
| `outcome` | `status`, `duration_ms` | a failed command must not be predicted as confidently as a successful one |

Nothing new needs recording. **This is the whole reason the feature is cheap.**

Two things to decide while implementing:

- **Does the model learn failed commands?** It must *see* them — repair needs to know what failure
  looks like — but a failure should not be offered as a prediction. `outcome` is how vista is told;
  verify the weighting does what we want rather than assuming.
- **Secret lines.** `Input::Command { secret }` already suppresses history. It must suppress this
  too, and that is a test, not a comment.

## Phase 1 — the model, fed and persisted

No user-visible behaviour. A model that learns and can be inspected.

- [x] **Dependency form: vendored**, `vendor/vista`, beside `brush-parser` and `full_moon`.
      A git dependency was the recommendation and remains the better end state, but it would have
      to point at a vista whose MSRV fix is pushed, and nothing can be built against a path outside
      the repo in CI. Vendoring is buildable today, carries the fix, and is two lines to reverse.
      Examples were dropped from the vendored manifest; `[[test]]` targets and `src/` are intact.
- [x] **Feature-trimmed and measured**, one at a time, into the ledger above.
- [x] **`crates/oslo-base/src/predict/`** — `Model`, mapping `Entry` to `Observation`, with
      `next()` for prediction and `repair()` for the aligned rebuild. Eight tests.
- [x] Snapshot beside the history database, `0600`, same as the frecency and dev-shell caches. It
      holds command text, so it is exactly as sensitive as history itself. Written once on the way
      out; `oslo history clear` deletes it, and a session with no history never writes one.
- [x] Load: `read_snapshot` on a background thread from `Tracker::start`, behind the same
      `keeps_a_record` gate the history store is behind. Absent until it lands; nothing waits.
- [x] `Model::learned()` and `Model::corrections()` — what it was shown, and what it made of it.

### What the mapping turned out to be

Better than the plan assumed. `track::log::Entry` already carries `session` and `seq`, so the
stream and the position are fields rather than derivations:

| vista | oslo | |
|---|---|---|
| `stream` | `Entry::session` | a `u32` ordinal, already "same shell or not" |
| `position` | `Entry::seq` | per-session, **and it skips** |
| `item` | `Entry::line` | |
| `context` | `Entry::mode` | `cwd`/`root`/`host` want the outcome join; not yet wired |

The skip is the good part. A secret command is never appended, so `seq` jumping 4 → 6 is the log
saying *something happened here you cannot see* — handing that gap over unchanged is what stops the
model learning a transition that never happened. Nothing had to be added for that; it was already
designed in.

### What had to be measured rather than assumed

**A stream orders candidates; it does not fence them.** The first test asserted that a command from
another terminal could not appear in this shell's predictions. It failed. vista's recent cache is
global, so any command in history is a candidate — what the stream decides is the *ranking*, and
with a pattern established the foreign command sinks to last place at a third of the top
probability. That is right for a shell, since the history source has always offered other shells'
lines, but it is not what "the session is the stream" implies. Written down and pinned by a test.

**The open question this phase must answer: what does it cost at startup?** oslo starts in 3.5 ms
and beats bash. Replaying 10,000 events at every start is not acceptable; a snapshot read may not
be either. Measure both, and if neither is free, the model loads on a background thread the way
`command_index::warm` already does — with prediction simply absent until it is ready.

## Phase 2 — prediction as a suggestion source

1. `settings::Source::Prediction`, joining `History`, `Completion`, `Path`. Configurable in
   `oslo.suggest.sources`, and **not in the default order until it has earned it.**
2. `OsloHelper::suggest` calls it with `Query { partial: Some(typed), context: cwd/root/host }`.
3. Extend `bench/keystroke.rs` — it exists and already measures `paint` and `hint` per keystroke.
   The ghost path currently costs ~2.3 µs. A prediction that costs 500 µs is not shippable however
   good it is.

**Acceptance:** measurably better than the history source on a real corpus, at a cost the keystroke
bench can live with. If it is not better, it does not go in the default order — the honest outcome
of a measurement is sometimes "no".

## Phase 3 — repair

The `thefuck` feature, and the one that needs the most care because it proposes running something.

1. **Trigger.** Three candidates, and they are not exclusive:
   - explicit: `oslo fix`, or a key binding — always safe;
   - `on-command-not-found`, which already fires from `exec/simple.rs` — the typo'd-command case;
   - after any non-zero exit, offering rather than acting.
2. **Never run anything without confirmation.** oslo has `ask` and the dropdown/finder widgets; the
   candidates are a list to choose from, with the chosen line placed *in the editor* rather than
   executed. A shell that silently runs a different command than the one typed is a shell nobody
   should trust, and vista returning a probability does not change that.
3. **The chosen line is put on the input line**, so Enter is yours. This also makes the feature
   teachable: you see the correction before it happens.
4. Feed the accepted correction back with `observe`, so the model learns the repair was right.
   `predictor.corrections()` exposes the pairs it has learned.

**Acceptance:** on a corpus of real failures, the top candidate is the intended command often
enough to be worth a keystroke — and *never* silently runs.

### What this phase turned out to be

**The trigger did not need to be built.** `oslo.keys[k] = function(line) … end` already replaces the
line being edited with what a Lua function returns, so the whole interaction is four lines of
config — and it lands in the editor by construction, which is the property point 3 asks for.
Building a native binding beside it would have been a second way to say the same thing, with a key
chosen by oslo rather than by the user.

**A failure has to be reported, or nothing is learned from it.** vista forms a correction pair from
an observation marked failed followed by the retyping. oslo was sending `outcome: Vec::new()` for
every command, so that machinery was inert — silently, since prediction still worked. It reports the
status now, which meant moving the observation from where the line is *logged* (before it runs, so
there is no status) to the command boundary, where `record` holds and `settle` learns.

**Then the measurement that changed the design.** Once failures were learned, repair for the line
that had just failed answered *nothing* — the typo is in the model as a command like any other, so
there is nothing to align it to. This is the case the whole feature exists for. Two candidate fixes:

| | binary | fixes it |
|---|---:|---|
| `surface-indexes` | +100 KB | **no** — still empty |
| don't learn what never ran (`predict::ran`) | +0 | yes |

So a line that exits 127 or 126, or never reached execution, is not learned.

### The rule was too narrow, and the trigger was still missing

Both found by using it. A key on the input line only ever fixes what you are *typing* — but the
repair anybody actually reaches for is of the command they have already run and watched fail, and by
then it is not on the line to correct. That needs two things:

1. **`oslo.repair()` with no argument**, answering for the last line that failed. One line is kept,
   set at the command boundary and cleared by the next success, so it always means "the thing you
   just watched go wrong". A secret line never reaches it, because it is never logged and so never
   held.
2. **Learning only what succeeded**, because the first rule was measured against the wrong case:

   | model | `repair("git stauts --short")` |
   |---|---|
   | `git status --short` ×3 | `git status --short`, p = 0.98 |
   | the same, plus the typo learned as a failed command | nothing |

   Excluding 126, 127 and never-ran covered a wrong *command word*; it left the far commoner case —
   a real command with a mistyped argument, `sudo apt updare` — learned at exit 1 and therefore
   unrepairable at the exact prompt where the repair was wanted.

The cost is stated rather than hidden: `cargo build` that failed to compile is not learned, so it is
not offered until a run of it succeeds; and vista's correction pairs, which need a failed
observation to form, are given up along with the outcome feature that fed them. They only reordered
candidates that already existed. This decides whether any exist at all.

### The trigger that was actually wanted

The key binding works, and it is still not what a person reaching for this wants: you have to *know*
the line is wrong and *remember* the key. So the correction is drawn after the line as it is typed —
reversed, because it is the shell disagreeing with text you already have rather than offering text
you might be about to have — and Right takes it under the rule that already accepts a suggestion.
Never both at once: a continuation exists when what was typed *starts* something, a correction when
it *near-misses* something.

**And `$PATH` had to join the model.** `lsvlk` is the case everybody tries first, and the model
could not answer it — it had never seen `lsblk` run. `command_index::nearest` already knew, because
it is what writes *did you mean lsblk?* after the failure. `oslo_ui::repair` asks both: the spelling
of the command word first, since it works on a shell with no history, then the model for the whole
line. `oslo.repair(line)` is the composed answer; `oslo.predict.repair` stays the model alone.

**A likeness gate on the model's half.** `predict_aligned` will answer a well-formed line with a
different command, which is a fine prediction and a terrible repair — drawn under every line anyone
types, it would be a permanent second opinion. A proposal has to be within a bounded edit distance
of what is on the line. Two edits by ten characters, not one, because the commonest typo of all is a
transposition and Levenshtein charges two for it.

**It sits on the keystroke path, so it was measured.** `nearest` walks every name on `$PATH`:

| | µs/keystroke |
|---|---:|
| repaint, for scale | 2.1 |
| ghost suggestion | 2.2 |
| repair, first attempt | **30.4** |
| repair, once a prefix of a real command is not a mistake | **0.22** |

A word that begins something runnable is unfinished rather than wrong, and a binary search says so
before the edit distance is reached. Typing `cargo build` now scans nothing at all; only a genuine
near-miss pays the 30 µs, which is a keystroke that is about to show you something.

## Phase 4 — Lua

Only after the shapes have settled, and only what a config genuinely cannot do otherwise:

```lua
oslo.predict.next(partial, n)  -- what the model thinks comes next
oslo.predict.repair(line, n)   -- candidates for a failed line
oslo.predict.ready()           -- whether there is a model to ask yet
```

Both answer a list of `{ line, probability }`, best first and possibly empty. `ready()` is there
because "no model yet" and "nothing matched" are both an empty list, and only one of them is worth
saying anything about.

The pieces a config needs to *use* these already landed: `c.commands` gives the parsed line as
data, and `oslo.quote` writes a word back safely.

## Verification

- `make verify` green at every phase: fmt, check-loc, check-readme, check, test, clippy `-D
  warnings`, rustdoc `-D warnings`.
- The differential corpus stays green. Neither feature may change what a script does — both are
  interactive-only, like `pre-cmd`.
- `bench/keystroke.rs` before and after Phase 2, interleaved, on a quiet machine.
- Exact binary bytes into the ledger at every phase.
- A secret line reaches neither the model nor the snapshot — a test, not a comment.
- Startup timing (`hyperfine`, n≥300) before and after Phase 1. The floor is `/bin/true`; losing to
  bash at startup to gain a suggestion is not a trade worth making.

## Out of scope

- Any LLM, network call, or telemetry. vista is deterministic and local, and that is the point.
- Sharing a model between machines. History already syncs; a model is a cache of it and can be
  rebuilt. Revisit only if rebuilding proves expensive.
- Prediction in scripts or `sh -c`. There is no prompt, no config, and nobody to offer anything to.
- Replacing the existing suggestion sources. This is a fourth source, and it competes on merit.
- Rule-based correction of any kind. If vista cannot repair something, the answer is a better model
  or nothing — not the beginning of a rules directory.

## Risks

- **A path dependency cannot be built by CI.** The measurement above used one; a release build
  needs the git dependency (or `vendor/`) before anything lands.
- **Startup cost.** oslo's startup is at the process floor today and that is a property worth more
  than this feature. If the model cannot be loaded cheaply, it loads late or not at all.
- **Keystroke cost.** The ghost path is measured in microseconds. Prediction must fit or be
  deferred to a thread.
- **A wrong repair is worse than none.** Hence confirmation, always, and the line going to the
  editor rather than to the shell.
- **Privacy.** The model is a distillation of every command you have run, on disk. `0600`, beside
  the history database, and cleared by whatever clears history — `oslo history clear` must clear it
  too, or the shell keeps what the user asked it to forget.
