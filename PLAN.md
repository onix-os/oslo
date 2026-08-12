# Six more, after composition

The last plan closed the gaps that stopped a plugin *composing* — with the pipeline, with another
plugin, with "later". These are the ones left over: work that happens off the prompt, a pipeline that
can summarise rather than only select, three ways to find out what a session actually did, and the
one extension point a shell has that an editor does not.

**Work on a new branch off `feat/compose`**, not `develop`: item 1 lands its callbacks at the safe
point the timers introduced, and item 2's verbs are the reason a Lua verb was worth allowing.

## 1. Nothing can run in the background and call you back

`oslo.job` lists, foregrounds, backgrounds and signals — all of them things done to jobs that already
exist. Lua cannot start work and be told when it is finished, so anything a config wants to *know*
must be fetched on the spot, blocking whatever asked.

**The cost is already being paid.** The `nix` prompt segment shells out on every draw — measured at
6 ms — because there is no other way to have an answer ready.

The machinery exists and is walled in. `lua/api/external.rs` already spawns a thread, waits with a
deadline, and delivers through a channel; it does that for *prompt commands only*, and nothing else
can reach it.

```lua
oslo.spawn{ "git", "status", "--porcelain",
  on_exit = function(out, status) oslo.state.set("git.dirty", out ~= "") end }
```

**Delivered where timers are.** `startup::timers::fire` already runs Lua at the two moments the shell
holds nothing; a finished job's callback joins that queue rather than inventing a second one. The
same honest limitation applies and must be documented: **a callback arrives between commands**, not
the instant the process exits.

What this is not: a general async runtime. One process, one callback, no promises and no scheduler.

## 2. The pipeline can select but not summarise

Every verb oslo has — `where cols get sort-by first last length each`, plus `to from lines parse` —
is **selection**. There is no way to reduce a stream to a smaller one.

That is what makes rows worth having over text. `ps | group-by user | count` is the query `ps | grep`
cannot express, and without it the structured pipeline is a nicer `awk` rather than a different tool.

Add four, in Rust:

| | |
|---|---|
| `group-by FIELD` | rows in, one row per distinct value, each carrying its group |
| `count` | how many rows, or how many per group |
| `uniq [FIELD]` | distinct rows, or distinct by one field |
| `stats FIELD` | min, max, sum, mean over a numeric column |

**Rust rather than Lua, even though Lua verbs now work.** These are the ones every pipeline reaches
for; they must be present in `oslo-minimal`, and they must be fast enough that nobody drops back to
`sort | uniq -c`.

**`join` is deliberately excluded.** It needs a *second* input stream, and oslo's pipeline is a line —
there is no shape for "and also read this". Adding one is a pipeline change, not a verb.

## 3. No way to find out why the shell got slow

There is no startup profiling of any kind. A session now loads `conf.d/*.lua`, `config.lua`, every
installed plugin, prompt segments and timers — five suspects and no instrument.

`oslo config timing` runs the same load the shell does and reports what each file and each plugin
cost. Neovim's `--startuptime` for the same reason: "my shell got slow" is otherwise answered by
commenting lines out until it stops.

The measurement must be of the *real* load, so this belongs beside `oslo config which`, which already
reproduces it.

## 4. No record of what a session did

Nothing answers "what loaded, what fired, what failed". A plugin that could not load said so once,
twenty commands ago, and the line is gone.

`oslo messages` — the diagnostics this session produced, newest last, each with what produced it: a
plugin, a config file, a hook. neovim keeps `:messages` for exactly this, and the plugin work made it
matter more: a refused trust hash, a shadowed name and a plugin that registered nothing are all
single lines that scroll.

**A ring buffer in memory, not a log on disk.** What a *session* said is a session-lived fact, and a
file would be one more thing to rotate, permission and eventually leak something into.

## 5. A plugin author cannot test a plugin

There is no harness. Every plugin written for this tree so far was tested by hand, in a pty, with a
temporary home — which is not a thing to ask of anybody else.

`oslo plugin test <directory>` loads the plugin into a session with a temporary home and database,
runs the assertions it declares, and reports. A plugin declares them the way it declares health
checks, which is a shape that already exists:

```lua
oslo.plugin.test("notes", function(t)
  t.equal(note_count(), 0, "a new database is empty")
end)
```

This is what turns "I wrote a plugin" into "I maintain a plugin", and it is the difference between a
plugin ecosystem and a directory of one-offs.

## 6. Completion cannot be declared, only computed

The highest ceiling on this list, and the most work.

oslo has a real completion spec system — `crates/oslo-ui/src/spec/definitions/` ships hand-written
specs for `git`, `cargo`, `docker` and `npm`, with subcommands, flags and descriptions. **A config
cannot add one**, and the reason is structural:

```rust
pub struct CommandSpec { pub names: Vec<&'static str>, pub description: &'static str, … }
```

`&'static str`. A spec built at runtime from Lua cannot be stored in it at all.

So a plugin's only route is `for_command`: a function that must re-implement subcommand matching,
flag parsing and descriptions by hand, for every command it wants to complete. What it wants to write
is what fish lets you write:

```lua
oslo.completion.spec{ command = "notes",
  subcommands = { { name = "new", desc = "start one" }, { name = "list", desc = "every note" } },
  flags = { { name = "--since", desc = "only newer than", takes = "duration" } } }
```

**The work is making `CommandSpec` own its strings**, then a Lua reader that builds one. The four
built-in definitions change with it, mechanically. Measure first: the registry is consulted on every
Tab, and `String` where `&'static str` was is an allocation at build time and a pointer chase at read
time — expected to be nothing against a `HashMap` lookup and a fuzzy match, but expected is not
measured.

## Order

Each step ends with `make verify` green and is its own commit.

1. **`oslo.spawn`**, delivered at the timers' safe point.
2. **`group-by`, `count`, `uniq`, `stats`.**
3. **`oslo config timing`.**
4. **`oslo messages`.**
5. **`oslo plugin test`.**
6. **Declarative completion specs**, last: the largest change, and the only one that touches code
   every keystroke goes through.

Steps 1, 2 and 6 are core and must work in `oslo-minimal`; 5 is behind `plugin`; 3 and 4 are tools.

## Verification

- `make verify` after every step, and `cargo test` with no features.
- **Step 1 needs a test that the callback does not arrive early**: a spawn whose process is still
  running must not have called back, and one that finished must call back exactly once.
- **Step 2 belongs in the corpus**, piped from a built-in producer and into a built-in consumer, with
  the empty stream and the single-row stream as their own cases — an aggregation over nothing is
  where these usually get it wrong.
- **Step 6 is measured before and after**: completion latency on a spec-carrying command, min-of-N,
  against `feat/compose`. If `String` costs anything visible, the change stops there.
- The 600-line rule will bite `data/tools/mod.rs` again at step 2; split by subject, not by order.

## What this does not do

- **No `join`, and no second input stream.** See item 2.
- **No async runtime.** Item 1 is one process and one callback; there is no promise, no coroutine
  scheduler, and no way for two callbacks to interleave.
- **No log on disk.** Item 4 is a session's own memory.
- **No sandbox.** Unchanged: the trust gate decides whether you run somebody's code.

## Open, and worth deciding rather than omitting

**An external door into a running session.** Neovim's ecosystem exists because *any* program can
drive it over RPC, and oslo has no equivalent — an editor that wants the shell's working directory,
or to run something in its context, has nothing to talk to. The precedent is in-tree: the scratch
daemon already has a socket and a wire protocol.

It is left out of this plan on purpose, because it is larger than the other six together and because
it may be the wrong shape for a shell: a shell's integration story has always been "it is a process,
pipe to it", and an RPC surface is a second, permanent interface to keep compatible. Worth an
explicit decision before anybody starts.
