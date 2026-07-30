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
| Batteries in Rust | `oslo.json` is a core capability, not a library: C modules cannot load in a static binary, which rules out lua-cjson and every Lua JSON parser built on it. **`oslo.http` was originally in this row and has since been dropped — see below.** |
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

## Built

| Decision | Where |
|---|---|
| Pure-Rust runtime, mlua deleted | `src/lua/eval/` — values, scopes, operators, expressions, statements, stdlib. Nothing in the dependency tree compiles C; a static musl binary builds with no C toolchain present. |
| `full_moon` front end | `Cargo.toml`; `eval::parse` |
| Glue subset plus metatables | `eval/ops.rs` — `__index`, `__newindex`, `__call`, `__eq`, `__lt`, `__concat`, `__tostring`, `__pairs` |
| Present and erroring | `eval/stdlib/stub.rs` — `coroutine`, `utf8`, most of `debug` and `io` |
| Lua patterns | `eval/stdlib/pattern.rs`, a translation of `lstrlib.c`'s matcher |
| Argv call model | `oslo.run{…}`, `oslo.pipe(…)`, `sh.*` — `src/exec/argv.rs`, `lua/api/run.rs` |
| Result tables, never raising | `{status, ok, out, err, signal}`; `out`/`err` absent rather than empty when uncaptured |
| Streaming | `oslo.lines{…}` |
| Namespaces | `oslo.fs`, `.path`, `.re`, `.json`, `.proc`, `.job`, plus `sh` |
| Batteries in Rust | `oslo.json` on `serde_json`; `oslo.re` on the `regex` crate already carried for `[[ =~ ]]` |
| Modules | `require`, `dofile`, `loadfile`, `load`, `package.*`; no `./?.lua`, empty `cpath` |
| Variable sharing | `Interp::set_script_global` and `engine::ShellGlobals`; `_G` first, shell second |
| Prompt modes | `src/startup/mode.rs`; Shift+Tab (configurable via `$OSLO_TOGGLE_KEY`), `$OSLO_DEFAULT_MODE`, `!`/`=` one-line escapes, `$OSLO_MODE` published |
| Incomplete input at the prompt | `eval::is_complete`, asking `full_moon` where the error is rather than string-matching `<eof>` |
| Hooks | `oslo.on.precmd`/`.postcmd`/`.cd`, each returning a removable handle |
| Introspection and options | `oslo.version`, `.user()`, `.host()`, `.interactive()`, `.login()`, `.exit_code()`, `.pid()`, `.ppid()`, `oslo.opts` |
| Converters | `oslo.from_lines`, `.from_columns`, `.from_pairs`, `.from_json` |
| `os.execute` / `io.popen` | Refused by name, pointing at `oslo.run` — they route through `/bin/sh`, which is not this shell |

The evaluator recursion limit needed a decision it did not have: a tree-walker puts Lua recursion
on the Rust stack, so against the repo's 1 MiB convention the honest limit was about *fifty*. oslo
now reserves its own 16 MiB stack (`crate::INTERPRETER_STACK`) and the limit is 200 — real Lua's
own ceiling on nested C calls — verified against that exact stack.

## `oslo.http`: built, then dropped

**Decision: there is no `oslo.http`. `sh.curl(…)` is the answer.** It was written, it worked, and
it came back out. The investigation is kept here because the finding is worth having and the
question will come round again.

HTTPS means TLS, and the providers everyone uses — `ring`, `aws-lc-rs`, `boring` — all compile C
or assembly, which would have put a C toolchain straight back into the build the Lua round had
just removed. That looked like a dead end.

It is not one. **`rustls-graviola`** is a pure-Rust provider by rustls's own author, and with it
`cargo tree -e build --target x86_64-unknown-linux-musl` reports no build dependencies at all for
the whole of oslo. A real HTTPS request worked. So "batteries in Rust" and "no C in the binary"
*can* both hold.

The price was **rustc 1.89**: `graviola` needs it, oslo's MSRV is 1.88, and CI's MSRV job caught
it. One release is not much — but it is a permanent floor raised so that a shell can do something
`curl` already does, on a machine that in practice has `curl` on it. The argv model makes the
alternative a single line with no quoting hazard:

```lua
local r = sh.curl("-fsSL", url)          -- or oslo.run{"curl", "-fsSL", url, capture = true}
if r.ok then print(r.out) end
```

That also inherits curl's certificate handling, its proxy support, its redirect rules and its
security updates, none of which oslo would then have to track. The reasons to revisit would be a
machine with no `curl`, or wanting a request that never shells out at all.

Two design answers from the removed version, kept because they would apply again: certificates
followed curl exactly (`cacert`/`capath`, then `$CURL_CA_BUNDLE`, `$SSL_CERT_FILE`,
`$SSL_CERT_DIR`, then the distribution paths, with nothing bundled), and a 404 was an *answer*
(`status = 404`, `ok = false`, body intact) rather than a failure to get one.

## Still open

1. **History is not tagged with its mode.** The mode a line was typed in is not recorded, so
   recalling a Lua line while in shell mode runs it as shell. rustyline's history holds plain
   strings, so tagging needs either a side table or a marker in the stored text — a visible
   design choice rather than an implementation detail.
2. **Depth limit** for a Lua-registered builtin that invokes itself. The evaluator's own limit
   does not see a round trip that leaves through the shell and comes back.
3. **The built-in tools themselves** — deliberately deferred. Each gets designed on its own terms,
   aiming past POSIX rather than cloning it.
4. **Migration of the existing corpus.** The 10 Lua cases still exercise `oslo.exec`/`oslo.capture`
   rather than the argv model. Both still work, so this is tidying rather than a gap.
5. **Per-command output parsers** (`sh.df()` returning a table). The generic converters are in;
   the per-tool ones are not.
