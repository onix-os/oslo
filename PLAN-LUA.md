# PLAN-LUA.md — the Lua-first scripting layer

Every decision below was made by the maintainer, in conversation, not chosen here. Where a
trade-off was close the reasoning is recorded with it, so a future reader can tell what was weighed
rather than having to re-derive it.

What this file does **not** yet contain is the implementation sequence — the rounds, the files, the
"done means". That comes once the last open questions at the bottom are closed.

Source material: a ten-agent survey of Hilbish, lush, the Lua system-scripting ecosystem
(luaposix, Penlight, luafilesystem) and the structured shells (Nushell, Elvish, xonsh), plus an
audit of oslo's own Lua surface. Every claim in "Confirmed defects" was verified against the tree.

---

## The architecture

**One core, two interfaces.** Both languages are front-ends over the same Rust core. Neither
language owns any behaviour; both dispatch into the same builtins, executor, expansion and job
control.

```
                        ┌────────────────────────────────┐
  shell source ────────►│                                │
   brush-parser → AST   │        oslo core (Rust)        │
                        │  ┌──────────────────────────┐  │
  lua source ──────────►│  │   one builtin registry   │  │
   lua parser → AST     │  └──────────────────────────┘  │
                        │   exec · expand · jobs · fs    │
                        └────────────────────────────────┘
```

**No C in the binary.** mlua is removed. It is a *binding* to the vendored Lua 5.4 C source, which
is why the static musl build currently needs a C compiler at all. Lua becomes what brush-parser
already is for the shell: a pure-Rust parser producing an AST, with oslo evaluating it.

The asymmetry this fixes, measured before the decision:

| | external dependency does | ours |
|---|---|---|
| shell | parsing only | 26,650 lines: exec, expand, 53 builtins |
| Lua (today) | parses **and executes** the whole language | 477 lines of bridge |

---

## Decided

| Area | Decision |
|---|---|
| Lua runtime | **Pure Rust, no C.** A pure-Rust Lua parser for the front end; oslo evaluates the AST. mlua and vendored Lua 5.4 are deleted. |
| Lua semantics | **Glue subset plus metatables.** Locals/globals, tables, functions, closures, `if`/`while`/`for`, operators, strings, metatables, and calls into the core. Out: coroutines, weak tables, `__gc`, `__close`. |
| Unimplemented surface | **Present and erroring, never absent.** `coroutine` exists as a table whose functions raise `coroutine.create is not implemented in oslo, at file:line`. If it were simply `nil`, the user gets `attempt to index a nil value` and has to work out why. A partial implementation that fails *legibly* is far easier to live with than one that fails quietly or cryptically — and it doubles as the to-do list for what to implement next. |
| Library compatibility | A pure-Lua library runs if its code stays inside the subset. That is a per-library question answered by trying it, and the loud errors above are what make trying it cheap. Accepted knowingly. |
| Built-in commands | Start with ~12 (`ls cat cp mv rm mkdir touch stat grep head tail wc`), grow later. One implementation, dispatched from shell syntax and callable from Lua. |
| Built-in vs `$PATH` | Ours wins by default, and it is configurable. |
| Built-in design | **Deferred.** Each tool gets designed on its own terms later; the aim is to be better than a POSIX clone, not a coreutils reimplementation. Architecture first, tools after. |
| Prompt modes | Shift+Tab toggles sh ⇄ Lua. The two are never mixed on one line. |
| Keybinding | Shift+Tab is the *default*; must be configurable. `BackTab` is the only Tab-family key terminals deliver distinctly — Ctrl+Tab is indistinguishable from Tab in the legacy encoding, so it would silently do nothing on a plain tty. |
| Default mode | sh, configurable. |
| One-line escape | `!ls -la` runs one shell line from Lua mode; `=print(1)` runs one Lua line from sh mode. |
| Variable sharing | Lua globals *are* shell variables when they hold strings. `local` stays private to Lua. Non-string globals stay in `_G`. Reads check `_G` first, then fall through to shell variables, so Lua's stdlib always wins and `type=deploy` in sh cannot break `type()` in Lua. |
| Command calls | `sh.grep("-n", p, f)` — sugar forwarding to an argv call `oslo.run{...}`. Argv end to end, so there is no escaping step that can be buggy. |
| Rejected: `os.grep` | `os` already holds `date`/`time`/`rename`/`remove` which collide with real binaries; `os.rg` would be `nil` on a machine without ripgrep; and Lua cannot spell `apt-get` (parses as subtraction), `7z`, `g++` or `[`. |
| Pipelines | `oslo.pipe({"grep",…}, {"wc","-l"})`. A method chain would need the sugar to return a lazy object rather than a result — two command models to keep in sync, for notation. |
| Failure | Commands return `{status, ok, out, err, signal}`. Never raise. `ok` is `status == 0`. |
| Captured output | `out`/`err` are `nil` when not captured, never `""`, so "captured nothing" and "did not capture" stay distinguishable. |
| Streaming | Output goes to the terminal by default; capture is opt-in. Plus `for line in oslo.lines{…}` for live consumption. Buffering by default is how you hold 200 MB of `cargo build` in memory. |
| Namespaces | `oslo.fs`, `oslo.proc`, `oslo.job`, `oslo.re`, `oslo.path`, plus a short global `sh` for command sugar. One namespace = one Rust file, which is the seam the 600-line limit needs. |
| Lua builtins | Keep `oslo.register_builtin`. Fix its reentrancy bug. It works in pipelines already because oslo forks — Hilbish needs its entire `sinks` subsystem only because Go cannot fork. |
| Hooks | Named setters, `oslo.on.precmd(fn)`, returning a handle. Not a global event bus: Hilbish's `bait` needs the identical function reference to remove a handler, so anonymous handlers are unremovable. |
| Multi-line at prompt | Yes. `load()` the input and check whether the error mentions `<eof>`; that means incomplete. Same trick the reference Lua REPL and Hilbish both use. |
| Bare names | `foo()` is Lua and only Lua. Commands need `sh.foo()`. Falling through to `$PATH` means your Lua function stops being called the day someone installs a binary with that name. |
| History | One history, each entry tagged with the mode it was typed in, so recall runs under the right interpreter. |
| Config file | `~/.config/oslo/init.lua`, interactive shells only. POSIX rc files keep working untouched — oslo still has to be a real `/bin/sh`. |
| Scope | Full system-scripting stack, not just a config layer. |
| Lua parser | **`full_moon` v1 with the `lua54` feature.** Pure Rust, maintained, the parser behind StyLua and selene — and already a production dependency in the maintainer's `os-tools`, where it walks the AST with its `Visitor` trait. Same relationship brush-parser has to the shell. |
| Modules | `require` and `dofile` are ours to write: read file → `full_moon` parse → evaluate → cache in `package.loaded`. Nothing about this needed a VM. `package.path` drops `./?.lua`; `cpath` is emptied, because a static binary cannot `dlopen` a `.so` and advertising the path turns "not found" into a loader error. |
| Batteries in Rust | `oslo.http` and `oslo.json` are core capabilities, not libraries. C modules cannot load in a static binary — which rules out luasocket, cqueues, lua-cjson and every Lua HTTP client built on them — so the things people reach for most have to come from us. Faster and safer there anyway. |
| Lua version | 5.4 syntax, including the integer/float distinction. |
| Regex | Expose the `regex` crate we already carry for `[[ =~ ]]`, as `oslo.re`. Lua patterns stay available because they are part of Lua. |
| JSON | Yes, `serde_json`. Works on static musl; `lua-cjson` cannot. |
| Structured returns | Our own functions return tables (`oslo.fs.ls()` gives entries with name/size/type/mtime), not text. |
| Filesystem API | Everything a shell script does: stat, read/write, mkdir -p, mktemp, rename, remove, chmod, symlink, realpath, glob, walk. |
| External output | Explicit converters (`oslo.from_json`, `from_lines`, `from_columns`) **plus** per-command parsers for common tools, so `sh.df()` can return a table. Accepted cost: we maintain parsers for other people's output formats. |
| Jobs and signals | Both reachable from Lua. Job tables with `:stop()`/`:background()`/`:foreground()`; `oslo.on.signal("INT", fn)`. Traps dispatch at command boundaries, never from the handler. |
| Timers/threads | No. An event loop is a large structural change for something a shell rarely needs. |
| Utility surface (v1) | Path manipulation, PATH helpers, introspection (`oslo.interactive`, `.login`, `.version`, `.user`, `.host`, `.exit_code`). `oslo.read` deferred — needs line-editor plumbing. |
| Interactive extras | None in v1: no editor API, no Lua completions, no abbreviations, no `@modifier` prefixes. Exception: a minimal `oslo.opts` table, because the config needs knobs for the toggle keybinding and default mode anyway. |
| Line editor / completions | Stay in Rust. Revisit once the scripting layer is solid. |
| Prompt | A Lua function, not a format string. |
| History backend | Rust owns it; Lua can read it (`oslo.history.all()`, `.search()`). |
| Lua errors | Message plus traceback, exit 1. |
| Cross-calls | Both directions: Lua can call shell functions, shell can call Lua-registered commands. |
| Testing | Hand-written expectation corpus, extending the existing 10 cases. **Recorded risk:** this encodes our own reading of Lua, which is the exact blind spot that produced most of this project's shell bugs. Diffing against a real `lua5.4` binary would be the stronger oracle and the harness shape already exists — cheap to add later if the corpus starts feeling thin. |

---

## Confirmed defects in the current Lua layer

Found by audit, each verified against the tree.

* `borrow_env` uses `try_lock` (`src/lua/engine.rs:34`), so a builtin registered with
  `oslo.register_builtin` cannot call *any* `oslo.*` function. `tests/lua_builtin_tests.rs:121`
  asserts that failure is correct behaviour.
* `precmd_fn`, `postcmd_fn` and `cd_fn` on `LuaEngine` (`src/lua/engine.rs:90-92`) have zero
  readers anywhere in the tree. They look like a feature in the type.
* `package.path` ends in `./?.lua` — the shell `require`s from the working directory.
* `package.cpath` points at `.so` files a static musl build can never `dlopen`.
* `os.execute` and `io.popen` route through `/bin/sh`, so an oslo Lua script runs someone else's
  shell, and fails entirely on a system where oslo is the only one.
* `oslo.exec` returns a number and `oslo.capture` returns a table — two result shapes.
* No argv call at all, so every command is a string: `oslo.exec("rm " .. name)` is one space away
  from the injection hole the whole Lua ecosystem is criticised for, and there is no quoting helper.

---

## Still open

Short list. Everything else above is settled.

1. **Incomplete-input detection.** The `<eof>` trick above is Lua's own `load()` talking. With our
   own evaluator the parser tells us directly — easier, but it has to be designed in rather than
   borrowed.
2. **Depth limit** for a Lua-registered builtin that invokes itself.
3. **The built-in tools themselves** — deliberately deferred. Each gets designed on its own terms,
   aiming past POSIX rather than cloning it.
4. **Migration of the existing corpus.** The 10 Lua cases and the API tests are written against
   `oslo.exec`/`oslo.capture`, which are being replaced.
