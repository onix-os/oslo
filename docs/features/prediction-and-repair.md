# Prediction and repair

One model of the commands you actually run, answering two questions: *what are you about to type*
(a ghost suggestion) and *what did you mean* (the correction, which is what `thefuck` does with two
hundred hand-written rules and this does with none).

> ## This is in `oslo`, not in `oslo-minimal`
>
> Everything on this page is behind the **`vista`** cargo feature, which is off by default. A
> release publishes two binaries per architecture and they differ in exactly this:
>
> | | has the model | `oslo.repair`, `oslo.predict` | ghost from |
> |---|---|---|---|
> | `oslo` | yes | yes | the model, then history, completions, `$PATH` |
> | `oslo-minimal` | no | **absent** | history, completions, `$PATH` |
>
> ```sh
> scripts/build.sh              # the full binary, every feature
> scripts/build.sh --minimal    # the floor: no model, nothing to learn from
> ```
>
> It costs 297 KB — 6,016,512 bytes without it against 6,320,928 with — on a binary meant to be
> `/bin/sh`, which is the whole argument for a distribution shipping `oslo-minimal` as the system
> shell. Without it the shell learns nothing, writes no model and reads none; a config that has to
> work under both asks before using the names, `if oslo.repair then … end`.

<!-- demo:begin -->
[![prediction-and-repair demo](https://asciinema.org/a/1262745.svg)](https://asciinema.org/a/1262745)
<!-- demo:end -->

## How it works

The model is a [vista](https://github.com/bresilla/vista) predictor — the `vista-recall` crate, a
git dependency pinned to a commit. It
is fed from the command log oslo already keeps, so nothing new is written to make prediction work.

A command's exit status does not exist when the command is logged, and that single fact shapes the
whole write path:

```
you press Enter
  │
  ├─ log::append_entry()            the line is written to history BEFORE it runs,
  │    │                            so another terminal can see a long command running
  │    └─ predict::record()         holds the entry — does NOT learn it yet
  │         └─ note_position()      where the next prompt will query from; known now
  │
  ├─ ……… the command runs ………
  │
  └─ tracking::record_outcome()     the command boundary: the status finally exists
       └─ predict::settle(status)
            ├─ FAILED = line        if status ≠ 0   (cleared again by the next success)
            └─ learn()              only if status == 0
```

`record` holds and `settle` learns. What is *not* deferred is the position: the next prompt asks
from it and cannot wait for a command that is still running.

### Five gates decide what gets in

| gate | why |
|---|---|
| `session == 0` | pre-dates session ordinals; filing it under a shared stream would teach transitions between unrelated shells |
| `seq == 0` | a row written before positions existed; there is no place in a stream to file it |
| secret (leading space) | never appended to the log, so `record` is never called |
| blank line | nothing to learn |
| **did not succeed** | see below — this one was forced by a measurement |

A secret command leaves **no gap**, and that is a known hole rather than a design. `seq` is advanced
only when a row is actually appended, so a hidden line consumes no position and the model sees the
commands either side of it as consecutive — learning a transition that never occurred. The comments
in the source claimed the opposite until writing this document forced the check.

### What is actually stored

vista is built here with `IdentityNormalizer`, which means **every distinct command line is its own
template** — there is no slotting. The model does not learn `apt install <thing>` as a pattern; it
learns whole lines.

```
     ┌───────────────────────────────────────────────────────────┐
     │ dictionary   one template per distinct command line       │
     │              + count, last seen                           │
     ├───────────────────────────────────────────────────────────┤
     │ PPM          per stream (= session):                      │
     │              "after these ≤8 templates, these followed,   │
     │               this many times"  — backs off to shorter    │
     │               contexts when the long one has no evidence  │
     ├───────────────────────────────────────────────────────────┤
     │ recent cache 256 items, weight 0.20, half-life 32         │
     │              one global list AND one per stream           │
     └───────────────────────────────────────────────────────────┘
```

Candidates are drawn from the PPM contexts, from the recent cache's *global* list, and from the
dictionary as a whole — which is why a command from another terminal *is* offered here. The stream
decides the *ranking*, not the membership. The first test written for this asserted the opposite and
failed; it now pins the real behaviour — a foreign command sinks to last place at under a third of
the top probability once this shell has any pattern to go on.

### Reading it back

```
  next(session, seq, partial)     PPM + recent cache + dictionary → candidates
                                  → filtered by `partial` (ContainsMatcher)
                                  → ranked                        → ghost suggestion

  repair(session, seq, failed)    predict_aligned: retrieve candidates, token-align each
                                  against the failed line, keep shared tokens as structure
                                  and differing ones from whichever side they belong to,
                                  drop anything that reproduces the input (≤3 iterations)
```

On top of `repair`, oslo adds a **likeness gate** that vista does not have: a proposal must be
within a bounded edit distance of what was typed. Without it `predict_aligned` will answer a
well-formed line with a *different* command — a fine prediction and a terrible repair, and drawn
under every correct line you ever type it would be a permanent second opinion. The budget is two
edits by ten characters rather than one, because the commonest typo is a transposition (`buidl`,
`tset`, `stauts`) and Levenshtein charges two for those.

### `$PATH` is the other half, and it is not learning at all

`lsvlk` is a misspelling of a real program whether or not it has ever been run here.
`command_index::nearest` — the same function that writes *did you mean lsblk?* after a failure —
answers that from `$PATH` alone, on a shell with no history. `oslo.repair()` asks it first and falls
back to the model:

```
oslo.repair(line)
   │
   ├─ 1. is the command word a real command?  (builtin / alias / function / $PATH)
   │        yes → skip to 2
   │        no, but a PREFIX of one → not a mistake, it is unfinished → nothing
   │        no → nearest() over $PATH, bounded edit distance      → "lsblk …"
   │
   └─ 2. the model, for the whole line, gated on likeness         → "git status --short"
```

### Where it is drawn

The correction appears after the line as you type, with **only the words that changed** bracketed:

```
$ systemclt status -> [systemctl] status
  ^^^^^^^^^^^^^^^^    ^^^^^^^^^^^  ^^^^^^
  what you typed      reversed     ghost grey
```

The arrow and the already-correct words are the ghost's ordinary grey; the changed word and its
brackets are that same colour reversed. A ghost suggestion and a correction are never drawn at once
— a continuation exists when what you typed *starts* something, a correction when it *near-misses*
something — so one key accepts whichever is showing.

The other half is the command that has **already run**:

```
$ git stauts --short          ← runs, fails
$ <F4>
$ git status --short          ← in the editor. Enter is still yours.
```

`oslo.repair()` with no argument answers for the last failed line. One line is kept, set at the
command boundary and cleared by the next success, so it always means "the thing you just watched go
wrong" rather than something that has scrolled off.

**Nothing here ever runs anything.** The correction lands on the input line and Enter is yours,
which is the property that makes a wrong guess cost a keystroke instead of a command.

### Persistence

Read at startup on a background thread, written once on the way out, never rebuilt from history:

```
start ── Tracker::start ── keeps_a_record? ──no──→ no model at all
                                │yes
                                └─ predict::warm(path) ─ thread ─→ read_snapshot (0.1 ms)
exit  ── settle_stores ─────────── save_shared(path)    0600, write-then-rename
```

`HISTFILE=""` reads nothing and writes nothing, and `oslo history clear` deletes the snapshot — a
model is a distillation of the history, so the switch that means "no trace" has to cover it too.

## What makes it different

`thefuck` ships roughly two hundred rules, one per tool, each encoding what that tool's error output
means. oslo has none: a repair can only ever be built out of `$PATH` and commands you have really
run, which is a safety property a rule engine cannot offer — it cannot propose something you have
never done. The cost is the mirror image: it cannot fix a command you have never got right.

Both `thefuck` and `sudo !!`-style aliases *run* the corrected command. oslo puts it in the editor.

## Configuration

```lua
oslo.suggest.sh_sources = { "predict", "history", "path" }   -- not in the default order

oslo.keys["f4"] = function(line)
  if line.text == "" then return oslo.repair() or "" end   -- the command that just failed
  return oslo.repair(line.text) or line.text               -- the one being typed
end

oslo.theme = { syntax = { repair = { fg = "yellow", reverse = true } } }
```

`repair` inherits the `autosuggestion` colour reversed unless you name it, so recolouring the ghost
drags the correction with it.

The Lua surface:

| call | answers |
|---|---|
| `oslo.repair(line)` | the corrected line, or nil — `$PATH` **and** the model |
| `oslo.repair()` | the same, for the last line that failed |
| `oslo.last_failed()` | that line itself, or nil |
| `oslo.predict.next(partial, n)` | `{ {line, probability}, … }` from the model alone |
| `oslo.predict.repair(line, n)` | the same shape, model alone |
| `oslo.predict.ready()` | whether there is a model to ask yet |

## Measurements

`cargo bench --bench predict`, on a synthetic history:

| history | replay | snapshot | save | load | predict | repair |
|---|---:|---:|---:|---:|---:|---:|
| 1,000 | 1.8 ms | 30.9 KB | 0.0 ms | 0.1 ms | 1.6 µs | 16.8 µs |
| 10,000 | 9.8 ms | 31.3 KB | 0.0 ms | 0.1 ms | 4.2 µs | 36.2 µs |
| 50,000 | 47.5 ms | 31.3 KB | 0.0 ms | 0.1 ms | 4.2 µs | 36.5 µs |

oslo starts in about 3.5 ms. Replaying ten thousand commands would cost three times the entire
startup to produce what a file already holds in 0.1 ms, which is the whole argument for the
snapshot. The snapshot is flat at ~31 KB because the model is bounded and evicts.

`cargo bench --bench keystroke`, per keystroke:

| | µs |
|---|---:|
| repaint, for scale | 2.1 |
| ghost suggestion | 2.2 |
| repair — ordinary typing | 0.22 |
| repair — an actual near-miss | 30.4 |

The gap between the last two is a design decision, not luck. `nearest` walks every name on `$PATH`;
a word that is a *prefix* of something runnable is unfinished rather than wrong, and a binary search
says so before the edit distance is reached. Typing `cargo build` scans nothing at all.

Binary cost, from the ledger in `PLAN.md`: 533,792 bytes over the pre-vista baseline — 521 KB, of
which the vista crate itself is 296 KB.

## What it cannot do

- **Generalise.** It has seen `sudo apt install jq`; it cannot invent `sudo apt install ripgrep`.
  Whole lines, not patterns.
- **Fix a command you have never got right**, except through `$PATH` spelling — which only ever
  corrects the command word, not an argument.
- **Learn from failures.** Given up deliberately: vista forms a correction pair from a failed
  observation followed by a retyping, but that observation is exactly what breaks repair. Measured:

  | model | `repair("git stauts --short")` |
  |---|---|
  | `git status --short` ×3 | `git status --short` |
  | the same, plus the typo learned as a failed command | nothing |

  A mistyped line inside the model is a command like any other, so there is nothing to align it to
  — and the repair anybody actually wants is asked *after* the failure. The cost of the rule is that
  `cargo build` which failed to compile is not learned until a run of it succeeds.
- **Remember forever.** The bounds are vista's defaults: 64 KB for any one command line, 64 MB of
  retained text, 16,384 templates, 256 streams, PPM order 8. It evicts by recency.
- **Work in a script or `sh -c`.** There is no prompt, no config and nobody to offer anything to.
- **Cross machines.** The model is a cache of history, which already syncs, and can be rebuilt.

## Where it lives

| path | what |
|---|---|
| `crates/oslo-base/src/predict/mod.rs` | `Model`, the gates, `record`/`settle`, snapshot I/O, `last_failed` |
| `crates/oslo-ui/src/repair.rs` | `$PATH` spelling + model, the likeness gate, `annotate` |
| `crates/oslo-ui/src/command_index.rs` | `nearest`, `has_prefix`, the cached `$PATH` index |
| `crates/oslo-ui/src/edit/session/frame.rs` | where the correction is drawn |
| `crates/oslo-ui/src/edit/session/accept.rs` | `take_hint` and `take_repair` |
| `crates/oslo-runtime/src/lua/api/predict.rs` | `oslo.repair`, `oslo.last_failed`, `oslo.predict.*` |
| `crates/oslo-runtime/src/startup/tracking.rs` | `Tracker::start` warms it; `record_outcome` settles it |
| `vista-recall` (git, pinned) | the predictor |
| `bench/predict.rs`, `bench/keystroke.rs` | the numbers above |
