# Build recipes

A `.make.lua` in a project, and `make build` runs a recipe out of it. Dependencies, parameters, and
a rule for when work can be skipped — a `justfile` or a `Makefile`, except that it is not a small
language invented to avoid writing a program. oslo already has a program: the config is Lua, the
directory environment is Lua, and the shell can run a command without a `/bin/sh` between it and the
argv. So the third file is the same language, pointed at builds.

```lua
-- .make.lua
local make = oslo.make

make.recipe{
  name    = "build",
  desc    = "the static release binary",
  deps    = { "fmt-check" },
  inputs  = { "src/**/*.rs", "Cargo.toml" },
  outputs = { "target/release/app" },
  stale   = "content",
  params  = { { "--type", desc = "minimal | full", default = "full" } },
  run = function(a)
    local features = a.type == "minimal" and {} or { "--all-features" }
    sh.cargo("build", "--release", table.unpack(features))
  end,
}

make.alias("b", "build")
```

```console
$ make                       # no recipe: list them
build  the static release binary
b      → build

$ make build --type minimal
→ fmt-check
→ build

$ make build                 # nothing changed
· fmt-check
· build  up to date
```

> ## This is in `oslo`, not in `oslo-minimal`
>
> Behind the **`make`** cargo feature, which is off by default. Without it there is no `oslo.make`,
> no `oslo make` tool and no `make` builtin — so the word falls through to `$PATH` and GNU make
> answers, which is what it does on every other shell.
>
> It reads a file in the working directory and runs what it finds. That it only does so **when you
> ask** is what makes it safer than [directory environments](directory-environments.md), not what
> makes it free.

## How it works

```text
 make build              at a prompt: the builtin — env/builtins/make.rs
   ├── not interactive, or no .make.lua here ──► /usr/bin/make, unchanged
   ▼
 oslo make build         a child process — src/cli/make.rs
   ▼
 make::governing(cwd)    walk up, nearest ancestor holding .make.lua
   ▼
 chdir to its directory  make's rule: a recipe resolves `src/` against the project
   ▼
 engine + init.lua + bindings        so oslo.make exists before the file mentions it
   ▼
 load .make.lua          declarations only — every body is a function, nothing runs
   ▼
 oslo.make.__main()      parse argv, plan the graph, run it, set the exit status
```

### Why it is a child process, and not a builtin

This is the decision the whole feature is shaped around, and it was forced by a measurement.

A builtin registered from Lua runs **while the shell holds its own state**, so every call that
reaches the shell fails from inside one:

```text
run RAISED   -> shell state is busy; an oslo.* call that reaches the shell cannot run from here.
sh RAISED    -> shell state is busy; …
lines RAISED -> shell state is busy; …
```

A build runner that cannot run a command is not a build runner. [Directory
environments](directory-environments.md) get around the same lock by taking it, snapshotting,
releasing it and running the file — but `direnv::arrive` is called from `startup/repl.rs`
*between commands*, at the one moment the shell holds nothing. A builtin has no such moment: it is
called from inside dispatch, in the middle of the state it would have to release.

So `oslo make` is a separate process, which is what `oslo macros`, `oslo hook` and `oslo secret`
already are: a fresh `oslo` with its own engine, holding no interactive state, where the whole API
simply works.

What that costs is what `make` and `just` already cost — **a recipe cannot `cd` the shell that
called it, or set a variable in it.** That is the semantics of a recipe, not a limitation of this.

### The `make` builtin gets out of the way

`make` is a real program that a great many projects are built with, and shadowing it would be the
mistake this codebase already refuses for `test`. The precedent is `which`, which answers about
*this* shell at a prompt and hands the word to `/usr/bin/which` everywhere else. Three conditions,
and all must hold:

1. **The shell is interactive.** In a script, `make` is the program the script was written for.
2. **A `.make.lua` governs the working directory.** No file, no claim on the name.
3. `\make` and `command make` reach the program, as they do for every builtin.

A project with a `Makefile` *and* a `.make.lua` gets the Lua one at a prompt — the same rule
`direnv::find` applies to `.env.lua` over `.envrc`, and for the same reason: a repository holding
both usually has one of them for everybody else.

`type make` answers *make is a shell builtin*, and `command -v make` exits 0. That is not
decoration. `your-own-tools.md` lists *"tools are invisible to everything that answers questions
about names"* as a limitation of `register_tool`; there was no reason to ship that hole twice.

### Nearest ancestor, and nothing merged

The walk is direnv's, verbatim: up from the working directory, take the first `.make.lua`. Nearest
wins outright — two files on one path means the inner one governs and the outer one does not, so
what `make build` does never depends on how deep in the tree you were standing.

### Strict `sh`, and only inside a recipe

`oslo.run` deliberately never raises: *a command that fails is not an exceptional event in a shell,
it is Tuesday.* That is right at a prompt and wrong in a build, where make's rule — stop at the
first non-zero — is the only safe default.

So the runner swaps `sh` for a strict one while it runs, the way direnv's stdlib exists only while
an `.envrc` is being read. Inside a recipe `sh.cargo(…)` raises on a non-zero status and the message
names the command; `oslo.run{…}` keeps its ordinary manners for the caller who wants to read
`r.status` themselves.

```lua
sh.cargo("build")                              -- raises: cargo exited 101
local r = oslo.run{ "cargo", "build" }         -- answers: r.ok is false
```

**A command oslo answers in rows has no status to check.** `sh.ls(…)` and `sh.df(…)` give a list of
rows rather than a result — see [structured pipelines](structured-pipelines.md) — so strict `sh`
cannot fail on them, and a recipe that needs the status of one writes `oslo.run{…}`.

## Staleness — the part `just` does not have

A recipe with no `outputs` is **phony**: it always runs, which is the default, and is the inverse of
make's `.PHONY`. One that declares `outputs` is skipped when it is up to date.

| `stale` | the question | cost |
|---|---|---|
| `"mtime"` *(default)* | is any input newer than any output? | one `stat` per file |
| `"content"` | do the inputs hash to what they hashed to last time? | one read per file |

Mtime is compared to the **nanosecond**, and an output must be **strictly newer** than every input.
Both halves are needed and each was a wrong build on its own. At second resolution an input edited
and an output written inside the same second compare equal, so a recipe that had not been rebuilt
reported `up to date` — and the fast inner loop is exactly where a build finishes inside one second.
Nanoseconds alone do not fix it either, because the *filesystem* decides the resolution: tmpfs
stamps to the jiffy, a few milliseconds, so two writes a fraction of a millisecond apart come back
byte-identical. Equal therefore counts as stale. That costs a spurious rebuild when a build
genuinely finishes inside one tick and buys never shipping a stale artifact.

`oslo.fs.stat` gained `mtime_ns` for this — a whole timestamp, not a sub-second remainder, so two
files compare with one `<`. `mtime` stays seconds, which is what a person prints.

`"content"` is the reason to have this at all. A `git checkout` moves every mtime in the tree, so
mtime staleness rebuilds the world after switching branches and back; a content hash does not.
The fingerprint is kept in [`oslo.db`](plugins.md), under a key covering the project, the recipe and
its arguments — so `make build --type minimal` after `make build` is not mistaken for a no-op.

**The stamp is written after the body returns, never during the check.** The first version recorded
the inputs up front, so a build that *failed* reported "up to date" on the next run — which is the
one answer a build tool must never give. `--force` runs a recipe regardless.

A recipe declaring `outputs` and no `inputs` is refused when the file is read: it could never be up
to date, and saying so beats a build that silently reruns for ever.

## Configuration

Everything is `oslo.make`, and a `.make.lua` is the only file that calls it.

```lua
make.settings{ quiet = false, keep_going = false, stale = "mtime" }

make.recipe{
  name    = "build",       -- required; a leading `_` keeps it out of the listing
  desc    = "…",           -- the second column of `make` with no argument
  deps    = { "a", "b" },  -- run first, each once per invocation
  inputs  = { "src/**" },  -- globs; `**` walks
  outputs = { "out.bin" }, -- declaring any of these makes the recipe skippable
  stale   = "content",     -- or "mtime"
  quiet   = true,          -- no `→ name` line of its own
  params  = { { "--who", desc = "…", default = "world" } },
  run     = function(args) … end,
}

make.alias("b", "build")
make.import("tools/.make.lua")     -- another file's recipes, in this graph
make.run("check-static")           -- one recipe from inside another, memoised
make.names()                       -- the declared names, as data
```

`run` is handed one table: declared parameters with their defaults filled in, any `--flag value`,
`--flag=value` or bare `--flag` from the command line, and `args.rest` for everything that was not a
flag. An undeclared flag arrives too — a recipe passing its arguments through should not have to
declare them first.

```
  -l, --list        the recipes and what they say they do
  -n, --dry-run     name every recipe that would run, and run none
  -f, --force       run even a recipe that is up to date
  -k, --keep-going  carry on after a recipe fails
  -q, --quiet       no progress lines, only what the recipes print
  -h, --help        this text
```

## What makes it different

`just` invented a language, and everything a language has to grow is grafted on: `{{interpolation}}`
because there are no expressions, `set shell := […]` because there is no way to run a command
directly, a `[private]` attribute because there is no scope. Here a variable is a `local`, a
condition is an `if`, and a list of exclusions is a list.

`make` has the one thing `just` gave up — knowing whether work can be skipped — and it decides with
mtimes, which is why a fresh checkout rebuilds the world. `stale = "content"` is the same idea with
the failure mode removed.

Both shell out through `/bin/sh`, so every recipe is one careless value away from word-splitting.
`sh.rm(name)` is argv end to end: there is no quoting step, so there is no quoting bug.

The reference comparison is oslo's own build. `Makefile` and `.make.lua` in this repository produce
the same binary; the second has no `$$`, no backslash continuations, no `.PHONY` list, and no
hand-maintained `help:` target repeating what the recipes already say.

## What it cannot do

- **No `-j`.** `oslo.spawn` delivers its callback at a safe point — a command boundary or an idle
  prompt — and `oslo make` has no read loop, so nothing ever drains the queue. Parallelism needs a
  blocking join, which a child process can offer safely and which is not written yet.
- **No pattern rules.** `%.o: %.c` is expressible as a recipe that declares recipes, and generating
  them at load time works, but there is nothing built in.
- **A recipe cannot change the shell that called it.** It is a child. `cd`, `export` and setting a
  shell variable affect the recipe and nothing else — make's semantics and just's.
- **No completion of recipe names yet.** Reading the names means evaluating the file, and doing that
  on a Tab press is arbitrary execution on a keystroke. The declared names are data
  (`make.names()`), so the cache this needs is possible; it is not built.
- **`make` is a builtin only in an interactive shell.** In a script, in `oslo -c`, and in a
  `#!/bin/oslo` file the word is the program. `oslo make` works everywhere.
- **A row-answering command cannot fail a recipe.** Strict `sh` checks a status, and `sh.ls(…)`
  answers with rows instead. `oslo.run{…}` is the way to check one.
- **No `--fmt`, no `--dump`.** It is Lua.
- **The `Makefile` still works, and there is no converter.** Two build files in one tree is a
  choice, not a migration.

## Where it lives

| path | what |
|---|---|
| `crates/oslo-shell/src/make.rs` | `governing`, `root_of`, `NAME` — which file, and nothing else |
| `crates/oslo-shell/src/env/builtins/make.rs` | the `make` builtin, and the handover to the program |
| `crates/oslo-runtime/src/lua/api/make.rs` | `__argv`, `__file`, `__root`, `__status`, `__emit`, `__relative` |
| `crates/oslo-runtime/src/lua/api/make.lua` | recipes, the graph, staleness, parameters, the listing |
| `src/cli/make.rs` | `oslo make`: find, chdir, boot the engine, run |
| `src/cli/tools.rs` | the row that makes `oslo make` reachable |
| `tests/make_tests.rs` | the end-to-end suite, one temporary project per case |
| `.make.lua` | oslo's own build, as recipes |
| `plans/PLAN_MAKE.md` | the inventory this was designed from |
