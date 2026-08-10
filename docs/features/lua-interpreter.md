# The Lua interpreter

`oslo-lua` is a Lua 5.4 evaluator written in Rust, with exactly one dependency: `full_moon`, the
lossless Lua parser behind StyLua and selene, vendored in `vendor/full_moon`. It exists because oslo
ships as a single statically linked musl binary and speaking Lua had to cost no C toolchain. The
crate it replaced, `mlua`, binds the reference interpreter: it compiles some thirty thousand lines of
C into the binary, and it keeps Lua values in a VM the shell can only pass strings across.

## How it works

`full_moon` parses and nothing else. Everything after the AST is oslo's own — a tree walker, not a
bytecode VM. That is the same arrangement `brush-parser` has to the shell side, and it is the point:
**one Rust core, two front ends.** A builtin is written once in Rust, and both `ls -la` typed as
shell and `sh.ls("-la")` written in Lua reach it.

```text
  source text
      │   full_moon::parse                       oslo_lua::parse / is_complete
      ▼
  full_moon::ast::Ast ─────────────────────────────────────────────┐
      │   Interp::run_ast                                          │ borrowed for
      ▼                                                            │ the whole run
  stmt::exec_block ──► stmt::exec_stmt ──► expr::eval / eval_multi  │
      │                     │                       │              │
      │ Flow::Normal        │ set_line(n)           │ ops::index   │
      │ Flow::Break         │                       │ ops::arith   │
      │ Flow::Return        ▼                       ▼              │
      ▼               Scope (locals) ─parent─► Scope ─► _G ────────┘
   Vec<Value>                                          │
                                                       ▼
                                            dyn Globals  (the shell)
```

Three decisions in that picture are load-bearing.

**Every `Interp` method takes `&self`.** The mutable state sits behind `Cell` and `RefCell`. That is
not style: a Lua script can call `oslo.proc.exec("build")`, the shell runs `build`, and `build` can
turn out to be a builtin that same script registered — so control has to re-enter an interpreter
that is still part-way through the outer call. With `&mut self` that second entry is unreachable.

**Scopes hold variables, not copies.** A closure captures an `Rc<Scope>`, so `local n = 0; local
function bump() n = n + 1 end` mutates the one `n`. Names resolve by walking the scope chain and
falling through to globals; real Lua assigns every local a register index while compiling, which is
much faster, and a hash lookup per variable is not what makes a shell slow.

**Function bodies are shared, keyed by the address of their AST node.** Creating a function *value*
used to deep-copy the whole body, which for a closure written inside a loop is once per iteration.
The cache is emptied at both ends of `run_ast`, which is what makes an address a sound key: an entry
can only live while the AST holding that node is borrowed by the call that made it.

The crate sits at the bottom of the graph and depends on nothing above it. Everything else in oslo
depends on *it*, because `oslo_lua::value::Value` is the shell's own interchange type — settings,
themes, structured rows and job reports are all built as Lua tables in Rust before any Lua runs.

```text
   oslo (bin + lib)
        │
   oslo-runtime ────────────┬───────────────┐
        │                   │               │
   oslo-shell ──────────► oslo-ui           │
        │                   │               │
        └────────┬──────────┘               │
             oslo-base ─────────────────► oslo-lua ──► full_moon (vendored)
```

### The crate, file by file

| File | Lines | What it holds |
| --- | --- | --- |
| `value.rs` | 485 | `Value`, `Number` (int/float subtypes), `Table` (array part, hash part, insertion order) |
| `scope.rs` | 157 | `Scope` chain and `Closure` |
| `expr.rs` | 502 | expressions; `eval` for one value, `eval_multi` for all of them |
| `stmt.rs` | 375 | statements, and `Flow` for `break`/`return` |
| `ops.rs` | 340 | operators and the metamethods behind them |
| `lib.rs` | 487 | `Interp`, `LuaError`, `Globals`, `current` |
| `stdlib/` | 2552 | `base`, `string`, `pattern`, `table`, `math`, `os`, `module`, `stub` |

`stdlib/pattern.rs` is a direct translation of the backtracking matcher in Lua's own `lstrlib.c`.
Lua patterns are not regular expressions — `%b()` matches balanced parentheses, `%f[%w]` is a
frontier assertion, there is no alternation — so substituting the `regex` crate would have been less
code and would have silently changed what every `string.gsub` in every script does.

### Crossing the boundary

Globals are one namespace with two spellings. The host implements a three-method `Globals` trait, so
an interpreter with no shell attached — a unit test, a `where` filter — behaves exactly as stock Lua.

```text
   reading `name`                        writing `name = v` from a script
        │                                        │
   in the scope chain? ──yes──► value      a local owns it? ──yes──► write the local
        │ no                                     │ no
   in _G? ─────────────yes──► value        v is a string? ──yes──► host.set(name, v)
        │ no                                     │                 and clear _G[name]
   host.get(name) ─────yes──► value              │ no
        │ no                              _G[name] = v, host.unset(name)
       nil
```

`_G` is consulted before the shell, and that ordering is the safety argument: a shell script that
sets `type=deploy` or `print=/usr/bin/lpr` must not break `type()` and `print()` in Lua. Each name
lives in exactly one of the two homes, so a name that changes type moves rather than leaving a stale
copy behind. In practice `name = "world"` in Lua is `$name` in shell on the next line, and a table
assigned to a global stays in `_G`, because a shell variable can only hold a string.

Going the other way, a Lua function registered with `oslo.register_builtin` is called with argv as a
table and its return value is turned into an exit status: `nil` and `true` are 0, `false` is 1, a
number is itself, a string is parsed.

## What makes it different

oslo's config is `config.lua`, and the values a config produces are the same `Value` the shell reads
— there is no serialisation step and no second config format, and no separate configuration
language to learn beside the one the prompt already speaks.

Against stock Lua, four differences are deliberate:

* **`pairs` iterates in insertion order.** Lua promises no order, but a table's hash part is walked
  by a `HashMap` whose order changes between runs, and a record printed as columns would print them
  shuffled. The hash part carries a `Vec<Key>` alongside it; lookup stays O(1) and only iteration
  and removal pay.
* **`package.path` has no `./?.lua`.** Stock Lua searches the working directory, which in a shell
  is a hijack: `cd` somewhere untrusted, run a tool, load a stranger's `utils.lua`.
* **`package.cpath` is empty.** A static binary cannot `dlopen` anything, and advertising a path
  would turn an honest "module not found" into a confusing loader error.
* **Everything unimplemented is present and erroring, never `nil`.** `coroutine` is a real table
  whose every member raises `coroutine.create is not implemented in oslo's Lua`, with the file and
  line. Left `nil`, the first use would be `attempt to index a nil value` and the reader would go
  looking for a typo.

`io.popen` and `os.execute` refuse by name rather than working, because in real Lua both run their
argument through `/bin/sh` — someone else's shell, from inside this one, and nothing at all on a
system where oslo is the only shell installed. Each error names its replacement.

## Configuration

The interpreter has no settings of its own. What it exposes is the module search path, assembled
from the environment at startup:

```lua
-- ~/.config/oslo/lua/?.lua, then ?/init.lua, then the system 5.4 directories.
-- Rooted at $XDG_CONFIG_HOME when that is set, otherwise $HOME/.config.
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

## Measurements

Timed with `os.clock()` inside the script, running `target/release/oslo` (fat LTO, `opt-level = 3`),
each figure the spread over four runs:

| What | Time |
| --- | --- |
| 1,000,000 iterations of `for i = 1, N do n = n + i end` | 0.218 – 0.246 s |
| 1,000,000 table stores, `x[i] = i * 2` | 0.334 – 0.337 s |
| 200,000 calls of a one-line Lua function | 0.072 – 0.078 s |
| 100 processes, each parsing and running an empty chunk, no config to load | 0.20 – 0.23 s in total |

That is roughly 220 ns per loop iteration and 380 ns per call — a tree walker's numbers, and far off
a bytecode VM's. It is the trade the crate exists to make. One further number is recorded in the
source rather than measured here: creating a function value used to deep-copy its body at **4.96 µs
for a ten-line closure**, paid on every evaluation of the same `function … end`, which is why
`Interp::bodies` exists. For size, `crates/oslo-lua` is 4,898 lines of Rust across 15 files and the
vendored parser is 14,290, plus 926 in its derive crate.

## What it cannot do

* **Coroutines.** Suspending mid-call needs either threads or a bytecode VM, and a tree walker
  running on the Rust stack is neither. `goto` and `::labels::` parse and then refuse at run time
  with `statement 'goto …' is not implemented in oslo's Lua`.
* **Collect cycles.** Tables are `Rc<RefCell<…>>`, so `t.self = t` leaks. There is no tracing GC,
  no weak tables, no `__gc` and no `__close`.
* **Hold arbitrary bytes in a string.** `Value::Str` is `Rc<str>`, so byte operations go through
  `String::from_utf8_lossy`: `string.char(255)` is three bytes long and reads back as 239, and
  `("héllo"):sub(1, 2)` returns a replacement character where real Lua returns half a codepoint.
  `#"héllo"` is still 6, and single-byte escapes such as `string.char(27)` are exact. The `utf8`
  library is one of the tables that refuses.
* **Enforce `<const>` and `<close>`.** The attributes parse and are ignored, so assigning to a
  `<const>` local succeeds here and is a compile error in real Lua.
* **File handles.** `io.open`, `io.lines` and friends need a userdata type and `__close`; the
  `oslo.fs` functions cover what a shell script does with a file. `io.write` and `io.read` are real,
  and `io.read` only does line formats.
* **Match C's `printf` exactly.** `%g` ignores its precision and never switches to exponent form —
  `string.format("%g", 1/3)` is `0.3333333333333333` — and `%a` is not a hex float.
* **Recurse deeply.** There is no tail-call optimisation, so `MAX_DEPTH` is 200 nested calls, above
  which the error is `stack overflow: too many nested calls`. A shell may not abort, and the ceiling
  holds because oslo runs the interpreter on a 16 MiB stack it reserves itself
  (`INTERPRETER_STACK`) rather than on whatever `ulimit -s` gave it.
* **Catch an exit.** `oslo.proc.exit(n)` travels as an error so that it unwinds, and `pcall`
  re-raises it — `pcall(oslo.proc.exit)` ends the shell, which is what "never returns" has to mean.
* **`load` from a reader function.** Only the string form; the callback form exists for chunks too
  large for memory, which a shell does not have.

## Where it lives

| Path | Key items |
| --- | --- |
| `crates/oslo-lua/src/lib.rs` | `Interp`, `LuaError`, `Flow`, `Globals`, `MAX_DEPTH`, `run`, `parse`, `is_complete`, `current::publish` |
| `crates/oslo-lua/src/value.rs` | `Value`, `Number`, `Key`, `Table`, `Function`, `NativeFn`, `parse_number` |
| `crates/oslo-lua/src/scope.rs` | `Scope::declare` / `get` / `set`, `Closure` |
| `crates/oslo-lua/src/expr.rs`, `stmt.rs` | `eval`, `eval_multi`, `lookup`, `unsupported`; `exec_block`, `exec_stmt` |
| `crates/oslo-lua/src/ops.rs` | `arith`, `compare`, `concat`, `index`, `set_index`, `metamethod`, `tostring` |
| `crates/oslo-lua/src/stdlib/` | `install`, `native`, `module`, and the eight library files |
| `crates/oslo-lua/src/stdlib/pattern.rs` | the `lstrlib.c` matcher, `Capture`, `Match` |
| `crates/oslo-runtime/src/lua/engine.rs` | `LuaEngine`, `ShellGlobals` (the `Globals` impl), `call_lua_builtin`, `status_from_lua` |
| `crates/oslo-runtime/src/lua/api/` | the `oslo.*` namespace; `api/run.rs` builds the `sh` table |
| `crates/oslo-runtime/src/lib.rs` | `INTERPRETER_STACK`, the 16 MiB stack the interpreter runs on |
| `crates/oslo-shell/src/data/tools/where_.rs` | a filter reaching the shell's interpreter through `oslo_lua::current` |
| `vendor/full_moon/` | the parser, `lua54` feature, `default-features = false` |
| `tests/lua_eval_tests.rs`, `tests/lua_corpus/` | the evaluator's tests and the hand-written corpus |
