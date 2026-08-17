# The Lua interpreter

oslo speaks Lua 5.4 through [`luna`](https://github.com/onix-os/luna), a stackless bytecode VM with
a tracing garbage collector, written in pure Rust. Pure Rust is the requirement everything else
follows from: oslo ships as a single statically linked musl binary, and speaking Lua had to cost no
C toolchain. `mlua` — a binding to the reference interpreter — would compile some thirty thousand
lines of C into the binary and need a musl cross-compiler to link it.

It is a dependency **pinned to a tag** — `v0.5.0` — rather than vendored or tracked on a branch. A
branch would make a build depend on the day it ran; a tag moves only when somebody moves it, and
`cargo update` cannot move it at all. `Cargo.lock` records the commit behind the tag either way, so
what the tag buys is a name a human can read in a diff. See `crates/oslo-luavm/Cargo.toml`.

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
  luna (pinned) ──► oslo-luavm ──► oslo-shell ──► oslo-runtime
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

oslo's config is `init.lua`, and the values a config produces are the same `Value` the shell reads
— no serialisation step, no second config format, and no separate configuration language to learn
beside the one the prompt already speaks.

`io.popen` and `os.execute` are meant to refuse by name rather than work, because in real Lua both
run their argument through `/bin/sh` — someone else's shell, from inside this one, and nothing at
all on a system where oslo is the only shell installed.

### Handles, and things that stream

**Everything oslo hands out that owns something is an object with a metatable**, built by
`api/handle.rs`. The verbs live behind `__index`, so `pairs` over a handle shows nothing to get
wrong and internals stay internal; `__newindex` refuses a typo rather than adding a field; and
`<close>` releases at the end of the block, after which every verb says so.

```lua
local db <close> = oslo.db.open("notes")     -- the file is shut here
local tmp <close> = oslo.fs.mktempdir()      -- the directory is removed here
```

`oslo.db.open`, `oslo.spawn`, `oslo.after`/`oslo.every` and `oslo.fs.mktempdir` are the four.
`h:close()` is the same call as leaving a `<close>` scope, and answers whether it was the one that
did the closing.

**No handle sets `__gc`**, and the reason differs by kind. For a database, a file or a pipe there is
nothing a finalizer would do that collection does not already: the verbs hold Rust values, and
collecting the handle drops them. `<close>` buys the *moment*, not the release. For a spawn, a timer
or a temporary directory a finalizer would be wrong — those handles are normally written for the
effect and thrown away, so `__gc` would cancel the callback, stop the timer, and remove a directory
whose path had been copied out.

**Anything that iterates is lazy, and the iterator is a handle too.** A generic `for` needs
something callable and a scope-bound release needs a metatable, so these carry `__call` — one value
that works in both positions, rather than two returns a caller has to remember to keep together:

```lua
for line in oslo.lines{"cargo", "build"} do oslo.ui.log(line) end
for path in oslo.fs.walk("/etc") do print(path) end
for line in oslo.fs.lines("/proc/mounts") do … end

local out <close> = oslo.lines{"journalctl", "-f"}   -- reaped when the block ends
for line in out do if line:find("error") then break end end
```

A loop that runs out cleans up on its own. A loop that `break`s does not, because luna does not
close a `for`'s closing value — which is what `<close>` and `:close()` are for.

### A failure carries facts, and is still the message

`nil, message` is the convention everywhere in `oslo.*`, and the second value is now an object:

```lua
local text, err = oslo.fs.read("/nope")
print(err)                       -- /nope: No such file or directory (os error 2)
print(err.kind, err.code)        -- not-found  2
if err.kind == "permission" then … end
```

`err.kind` is one of `not-found`, `permission`, `exists`, `invalid`, `truncated`, `timeout`,
`interrupted`, `other`; `err.code` is the errno; `err.path` is what the call was about, and
`err.to` as well for `rename` and `copy`. `oslo.db` adds `name` and its own `kind`.

**Everything that read the message still reads it.** `tostring(err)` and `print(err)` give the same
sentence as before, `"oops: " .. err` concatenates, and `err:find(…)`, `err:match(…)` and
`err:upper()` work because `__index` falls through to the string library. The only observable
change is that `type(err)` is now `"table"`.

The point is that matching English was the only way to ask what went wrong, and a translated C
library or a reworded message broke it. The kind and the errno are the facts; the sentence is a
rendering of them.

## Configuration

**The configuration is a Lua program, and it is one program even when it is several files.**

```lua
-- init.lua
local oslo = require "oslo"      -- the same table the global names, not a copy of it
require "aliases"                -- aliases.lua, beside this file
```

`require "oslo"` answers with the table `oslo` *is*. That is worth stating because the obvious
implementation gets it wrong: values crossing the shell↔VM boundary are converted, so a `preload`
that handed back a shell-side value produced a fresh table per call — and
`require("oslo").completion.max_rows = 42` was written into a copy nothing would ever read. The
registration is done in Lua, against the global, so identity holds and a setting lands.
`tests/config_modules_tests.rs` holds that to it.

The search path is set by `lua::api::policy` from the environment at startup:

```lua
-- ~/.config/oslo/?.lua and ?/init.lua — the config's own directory, so a second file
-- beside init.lua is `require "aliases"` rather than an absolute path spelled out.
-- Then ~/.config/oslo/lua/, for a library kept apart from the config that uses it.
-- Then the system 5.4 directories. Rooted at $XDG_CONFIG_HOME, else $HOME/.config.
print(package.path)

-- So a library of your own lives beside the config that requires it:
--   ~/.config/oslo/lua/mine.lua
local mine = require("mine")

-- Both are ordinary Lua values, so a config can extend the path, or register a
-- module that has no file at all. `preload` wins over the filesystem, so a
-- host-provided module shadows a file of the same name rather than racing it.
package.path = "/opt/team/lua/?.lua;" .. package.path
package.preload["team.colours"] = function() return { accent = "#89b4fa" } end
```

`package.cpath` is the empty string, not unset: a static binary cannot `dlopen` anything, and
advertising a C path would turn an honest "module not found" into a confusing loader error — but a
`cpath` that is absent breaks `package.cpath == ""`, which is how a script asks.

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

The benchmarks live in `bench/lua/`. The `_noos` variants time themselves from outside rather than
with `os.clock()`, which is how these figures include process start.

## What it cannot do yet

The standard library is complete enough that oslo's own tests no longer notice its edges: `os`,
`io`, `package`/`require`, `utf8`, `debug`, `coroutine`, `string.pack` and `_G` are all there,
`pairs` iterates in insertion order, recursion is bounded by a catchable error, and floats print as
Lua 5.4 prints them.

Four things remain, and one of them is oslo's own:

* **A generic `for` does not close its fourth value.** Lua 5.4 closes the loop's closing value when
  the loop ends, `break` and error included; luna does not. That is what would otherwise have made
  `for line in oslo.lines{…} do … break … end` reap its child by itself, and it is why the
  iterators oslo hands out are `__call`able handles you can also `<close>`.

* **A runtime error is `userdata`, where Lua 5.4 raises a string.** `tostring(err)` reaches the
  message, so nothing is lost — but `err:find("…")`, the idiom for inspecting one, cannot index a
  userdata. `error("…")` and `require`'s "module not found" are already strings; it is the errors
  the VM itself raises that are not.
* **`require` does not detect a module that requires itself.** Reference Lua marks a module as in
  progress and reports `loop or previous error loading module`; here the recursion runs to the call
  depth limit and reports a stack overflow instead. It stops, catchably — it just says the wrong
  thing about why.
* **`os.setlocale` is absent**, the last of the standard names. Nothing in a shell reaches for it.

Neither of the first two is reachable from ordinary shell use.

**The third is `_ENV`, and it belongs here rather than to the VM.** A name assigned a table and then
a string stays in `_G` instead of moving to the shell, because the globals metatable's `__newindex`
does not fire for a name `_G` already has — see [`globals`](#what-makes-it-different). Expressing
the rule exactly needs an always-empty `_ENV` proxy with the real names in a backing table, which
`Closure::load_with_env` makes possible; it costs a metamethod on every global access, which is why
it has not been done for one test.

**Three names oslo refuses on purpose**, and those are not gaps — see
`crates/oslo-runtime/src/lua/api/policy.rs`. `os.execute` and `io.popen` would run their argument
through `/bin/sh`, another shell started from inside this one; `os.tmpname` names a file without
creating it, leaving a window for somebody else to take the name. Each refusal names its
replacement, and `package.path` is set there too, without stock Lua's `./?.lua` — in a shell,
searching the working directory is a script hijack.

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
| `crates/oslo-runtime/src/lua/api/policy.rs` | the names oslo replaces, and where `require` looks |
| `tests/lua_eval_tests.rs`, `tests/lua_corpus/` | the language tests and the hand-written corpus |
