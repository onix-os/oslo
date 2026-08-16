# The Lua interpreter

oslo speaks Lua 5.4 through [`luna`](https://github.com/onix-os/luna), a stackless bytecode VM with
a tracing garbage collector, written in pure Rust and vendored in `vendor/luna`. Pure Rust is the
requirement everything else follows from: oslo ships as a single statically linked musl binary, and
speaking Lua had to cost no C toolchain. `mlua` — a binding to the reference interpreter — would
compile some thirty thousand lines of C into the binary and need a musl cross-compiler to link it.

**This replaced a tree walker.** Until recently the evaluator was oslo's own, walking a `full_moon`
AST. That crate and its parser are gone: 19,630 lines deleted, in exchange for coroutines, `goto`,
byte-exact strings, collected cycles, unbounded recursion and roughly an order of magnitude in
speed. What the trade cost is recorded under [What it cannot do yet](#what-it-cannot-do-yet).

<!-- demo:begin -->
[![lua-interpreter demo](https://asciinema.org/a/1262741.svg)](https://asciinema.org/a/1262741)
<!-- demo:end -->

## How it works

luna compiles a chunk to bytecode and runs it on its own stack inside a `gc-arena` heap. That heap
is the fact that shapes everything above it: a `luna::Value<'gc>` carries a garbage-collector
lifetime and exists **only** inside `lua.enter(|ctx| …)`. It cannot be returned, stored in a struct,
or built by code that has never heard of a VM.

oslo's own `Value` — in `oslo-base`, an `Rc` anyone can keep — stays the interchange currency, and
the two meet at exactly one place:

```text
  the shell, forty-odd files            the boundary                    the VM
  ────────────────────────────   ─────────────────────────────   ──────────────────────
  oslo_base::value::Value        oslo_luavm::convert             luna::Value<'gc>
  (owned, Rc, no lifetime)   ──►   into_lua / from_lua      ──►  (arena, 'gc lifetime)
                                          │
  structured pipeline,                    │  Engine  ── eval, load, call_function
  settings, theme, hooks,                 │  Host    ── what a native may ask
  the whole oslo.* API                    │  globals ── the shared namespace
```

`oslo-base` has no dependency on any engine at all. That is what lets the structured pipeline,
`settings`, `theme`, `hooks` and the ~200 registered callables be written, compiled and tested with
no VM in scope — and it is why swapping the engine underneath was a change to one crate rather than
to seventy files.

### The crate graph

```text
  vendor/luna ──► oslo-luavm ──► oslo-shell ──► oslo-runtime
                       ▲              ▲              │
                  oslo-base ──────────┴──────────────┘
```

### Crossing the boundary

Three things cross, and each is a rule worth knowing before touching `convert.rs`:

* **Tables are copied, not shared.** The tree walker's tables were the interpreter's own `Rc`, so
  Rust could fetch a global table and mutate it and Lua would see the change. A VM table cannot
  leave the collector, so what `Host::global` answers is a *snapshot*. Anything meaning to reach the
  live table goes through `Host::set_field`, or is done in Lua. This is how
  `package.preload` and `oslo.completion.for_command` are written.
* **Metatables cross both ways.** They did not at first, and three whole surfaces were silently
  dead — `sh.*`, `oslo.prompt` and `oslo.theme.styles` are each an *empty* table whose entire
  behaviour is its metatable, so copying only the entries delivered `{}`.
* **Identity is preserved.** A memo keyed on the `Rc` pointer means a value that was one thing stays
  one thing: `oslo.from_json == oslo.json.decode` answers true, and a table containing itself stays
  cyclic rather than being truncated — which is what lets `oslo.json.encode` refuse it by name.

Functions cross inside `Function::Held`, an `Rc<dyn Any>` the shell never looks inside. Two things
travel in it and only `convert.rs` knows which: a `Native` (a Rust closure going in, wrapped as a
luna callback) or a `StashedFunction` (a Lua function coming out, rooted so the collector keeps it
while a hook or a completer holds it).

**This is why the port was small.** All ~200 callables register through one helper, `util::native`,
whose signature is oslo's own — `Fn(&dyn Host, Vec<Value>) -> LuaResult<Vec<Value>>` — and 198 of
them ignore the `Host` entirely. Their bodies never changed.

## What makes it different

oslo's config is `config.lua`, and the values a config produces are the same `Value` the shell reads
— no serialisation step, no second config format, and no separate configuration language to learn
beside the one the prompt already speaks.

`io.popen` and `os.execute` are meant to refuse by name rather than work, because in real Lua both
run their argument through `/bin/sh` — someone else's shell, from inside this one, and nothing at
all on a system where oslo is the only shell installed.

## Configuration

The interpreter has no settings of its own. A single-file `~/.config/oslo/config.lua` is the
supported shape today; the module search path is part of the `package` support still to land.

## Measurements

`target/release/oslo` at fat LTO, best of three, wall clock including process start (3.6 ms, from
100 empty runs in 0.36 s). The tree-walker column is the figure the previous version of this
document recorded, measured the same way.

| What | tree walker | luna |
| --- | --- | --- |
| 1,000,000 iterations of `for i = 1, N do n = n + i end` | 0.281 s | **0.02 s** |
| 1,000,000 table stores, `x[i] = i * 2` | 0.444 s | **0.05 s** |
| 200,000 calls of a one-line Lua function | 0.093 s | **0.02 s** |
| 100 processes, each running an empty chunk | 0.61 s | **0.36 s** |

The benchmarks live in `bench/lua/`. The `_noos` variants exist because they cannot time themselves
with `os.clock()` yet — see below.

## What it cannot do yet

These are gaps in luna rather than in the binding, and each is written up with a minimal
reproduction in luna's own `plans/oslo_requirement.md`. **The shell itself is unaffected by all of
them** — commands, pipes, job control, globbing, completion, history and oslo's own structured
tools are Rust and never touch the VM.

* **`os`, `io`, `package`/`require`/`dofile`, `_G`, `xpcall`, `utf8`, `debug` are absent.** In
  practice: no clock in a Lua prompt, no splitting a config across files, and no `io.open` — though
  `oslo.fs.read` / `write` / `exists` cover what a shell script does with a file. A prompt function
  that reaches for a missing name fails *gracefully*: the error is reported and the built-in prompt
  is drawn.
* **`pairs` does not iterate in insertion order.** luna's hash part is `ahash`, seeded per process,
  so a row built **in Lua** prints its columns in a different order every run. oslo's own tools
  build their rows in Rust and are unaffected — `ls | to json` is stable.
* **A native cannot always call back into Lua.** Stepping a nested executor works, and a
  Lua-registered tool typed at the prompt runs correctly. But `oslo.proc.exec("<a tool written in
  Lua>")` *called from Lua* panics when that tool's body closes over a local, because reading an
  upvalue of a still-running thread hits a borrow conflict inside the VM.
* **Recursion is unbounded**, so a runaway recursive function in a config hangs rather than raising
  a catchable error.
* **`error(msg)` does not prepend `chunk:line:`**, so the `msg:match(":(%d+): ")` idiom finds
  nothing. `error(msg, 0)` and `assert` are already correct.
* **Floats render as integers.** `tostring(3.0)` is `3` and `10/2` prints `5`, where Lua 5.4 gives
  `3.0` and `5.0` — the subtype is tracked correctly, only the formatting is wrong.
* **Unimplemented names are `nil` rather than present and erroring.** oslo's rule is that a name it
  does not implement should raise `… is not implemented in oslo's Lua` with a file and a line, so
  the reader gets a sentence instead of `attempt to index a nil value`.

## Where it lives

| Path | Key items |
| --- | --- |
| `crates/oslo-luavm/src/lib.rs` | `Engine`, `eval`, `load`, `call_function`, `is_complete`, `current::publish` |
| `crates/oslo-luavm/src/convert.rs` | `into_lua`, `from_lua`, `raise` — values, metatables, functions, identity |
| `crates/oslo-luavm/src/host.rs` | `Host`, `Native`, `CallbackHost`, `run_nested`, the re-entrancy slot |
| `crates/oslo-luavm/src/globals.rs` | the shell's variables as Lua's global namespace |
| `crates/oslo-base/src/value.rs` | `Value`, `Number`, `Key`, `Table`, `Function` — the shell's own, engine-free |
| `crates/oslo-base/src/value/error.rs` | `LuaError`, `LuaResult` |
| `crates/oslo-runtime/src/lua/engine.rs` | `LuaEngine`, `ShellGlobals`, `call_lua_builtin`, `status_from_lua` |
| `crates/oslo-runtime/src/lua/api/` | the `oslo.*` namespace; `api/util.rs` registers every callable; `api/run.rs` builds `sh` |
| `crates/oslo-shell/src/data/tools/where_.rs` | `where` and `each`, compiling one expression and running it per row |
| `vendor/luna/` | the VM; `plans/oslo_requirement.md` is what oslo still needs from it |
| `tests/lua_eval_tests.rs`, `tests/lua_corpus/` | the language tests and the hand-written corpus |
