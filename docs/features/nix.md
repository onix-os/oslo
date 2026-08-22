# nix, as data

Everything `nix` can answer as JSON, reachable from Lua as ordinary tables. One function in Rust,
every name above it in Lua — so what this grows into is decided in a config or a plugin rather than
in the shell.

> ## This is in `oslo`, not in `oslo-minimal`
>
> Everything on this page is behind the **`nix`** cargo feature, which is off by default.
>
> | | |
> |---|---|
> | `oslo` | `oslo.nix`, and `use flake` in a directory environment |
> | `oslo-minimal` | neither |
>
> ```sh
> scripts/build.sh              # the full binary, every feature
> scripts/build.sh --minimal    # no nix integration at all
> ```
>
> It costs **48 KB** — 6,271,776 bytes without it against 6,320,928 with — the second-smallest of
> the nine optional features. It is off because a `/bin/sh` on a distribution has no business
> knowing what a flake is, not because of the size.
>
> Without it there is no `oslo.nix` at all, and a config asks before using it the way it asks about
> any other optional surface:
>
> ```lua
> if oslo.nix then … end
> ```
>
> The feature is independent of `direnv`. With both, a
> `.env.lua` can call `oslo.direnv.nix_develop()`; with `nix` alone there is no directory file to
> ask, and `oslo.nix` is the whole of it.

<!-- demo:begin -->
[![nix demo](https://asciinema.org/a/1263435.svg)](https://asciinema.org/a/1263435)
<!-- demo:end -->

## There is no `oslo nix` command

Deliberately, and it is not coming. The feature is a Lua table; anything that wants to be typed is a
tool a config registers, in three lines:

```lua
oslo.register_tool{
  name = "stale",
  rows = function() return oslo.nix.inputs() end,
}
```

`register_tool` takes a **table** — `name` and `rows` at least; see
[your own tools](your-own-tools.md). Returning the inputs rather than printing them is what makes
`stale | where 'days > 180'` work, because the rows are rows.

## How it works

```
oslo.nix.inputs()          Lua        crates/oslo-runtime/src/lua/api/nix.lua
      ↓
oslo.nix.run{…}            Rust       lua/api/nix.rs        argument reading
      ↓
nix_shell::json::run       Rust       nix_shell/json.rs     Command::new("nix")
      ↓
nix --json …
      ↓
oslo.json's decoder        Rust       lua/api/json.rs       one decoder, shared
      ↓
a Lua table
```

`--json` is appended unless the caller already wrote it, and every invocation carries
`--extra-experimental-features 'nix-command flakes'` so a user who has not enabled them in
`nix.conf` gets an answer rather than a lecture.

### Why one generic call and not a function per command

Twenty-three subcommands advertise `--json` on nix 2.34.6, and nix's own `--help` opens by saying
the interface is subject to change. That alone argues against hand-writing wrappers. Two things
settle it:

**The help text lies.** `nix registry list --help` documents `--json`. Running it:

```
$ nix registry list --json
error: unrecognised flag '--json'
```

A wrapper generated from the help would have shipped a function that cannot work, and a reader would
have blamed oslo. Here it is a message the caller can branch on.

**Names in Rust are names only oslo can add.** With the primitive generic, `oslo.nix.metadata` is
Lua — so a plugin that wants `closure_size` writes a Lua file instead of patching the shell, and any
helper below can be replaced by assigning over it.

### Argv, never a shell

`nix_shell::command` builds a *string*, because the one fixed `print-dev-env` call it serves is
handed to `eval_command_substitution`. Arguments arriving from a config are a different problem —
quoting them back into a string for a shell to take apart again is where the injections live — so
this uses `Command` and passes the list through untouched. `oslo.nix.run{"eval", "--option",
"warn-dirty false"}` reaches nix as three arguments, not four.

### The two failures that shaped the runner

**A document can be bigger than a pipe.** `nix config show --json` is 76 KB against a 64 KB pipe
buffer: a parent that waits on the child before draining its output deadlocks — the child blocks in
`write`, the parent in `try_wait`, and neither moves again. Both pipes are read on their own threads
before anything waits.

**Killing nix does not always close the pipe.** If nix left a child holding it, joining the reader
threads after the kill waits for the *grandchild*. A 150 ms deadline spent 30 seconds that way in
the test that found it. On a timeout the readers are abandoned rather than joined, because the
output has already been decided to be worthless.

## The names

Every one takes an optional table: `flake` to ask about an installable other than the current
directory, `cache` to keep the answer until the flake changes, `timeout` in seconds.

| | |
|---|---|
| `oslo.nix.run{…}` | any nix command, JSON in, table out — the only one written in Rust |
| `oslo.nix.available()` | is there a `nix` on `$PATH` to ask |
| `oslo.nix.metadata()` | description, revision, inputs, store path |
| `oslo.nix.inputs()` | every input with its pin date and age in days, oldest first |
| `oslo.nix.outputs()` | `flake show` — every output, by system |
| `oslo.nix.shells()` | the dev shell names this machine can enter |
| `oslo.nix.config()` | nix's settings |
| `oslo.nix.system()` | `x86_64-linux` |
| `oslo.nix.dirty()` | has the flake uncommitted changes |
| `oslo.nix.complete(prior, word)` | flake outputs, for the completion hook |

`inputs` is the one that says something you cannot already see. It costs no evaluation — the lock
file is read, not the flake — and it is the only thing in the shell that will tell you this:

```
systems                github    1220 days
flake-utils            github     636 days
nanopb-src             github     618 days
nixpkgs                github     125 days
```

## Configuration

Nothing here runs on its own. Arriving in a flake directory with this feature on and nothing
configured behaves exactly as it did before.

**Completion for the real `nix` binary** is one line:

```lua
oslo.completion.for_command.nix = oslo.nix.complete
```

`nix build .#<TAB>`, `nix develop .#<TAB>` and `nix run .#<TAB>` then complete from the flake's own
outputs, each subcommand offered the outputs it can take. Every other word falls through to oslo's
ordinary completion, and so does a named flake — `nix build nixpkgs#<TAB>` would evaluate the whole
of nixpkgs, and nothing that runs on a keystroke may risk that.

**A prompt fact**, if you want one:

```lua
oslo.prompt.left = function()
  local shell = os.getenv("IN_NIX_SHELL") and oslo.nix.dirty() and "flake*" or nil
  …
end
```

**Replacing a helper** is assignment, because they are Lua:

```lua
local was = oslo.nix.inputs
oslo.nix.inputs = function(opts)
  local found = was(opts) or {}
  …                                   -- your own ordering, filtering, whatever
  return found
end
```

## Measurements

Against a real flake, warm store, nix 2.34.6:

| | cold | warm | cached by oslo |
|---|---|---|---|
| `flake metadata` | 264 ms | 27 ms | |
| `flake show` | 455 ms | 34 ms | **0.1 ms** |
| `config show` | | 22 ms | |
| `search nixpkgs ripgrep` | **46 s** | | |

Two things follow. **nix caches evaluation itself, and does it well** — which is why oslo's cache is
opt-in rather than on: a warm `flake show` at 34 ms needs no help, and a caller asking `store info`
wants the store's answer rather than last week's. And **the 46-second measurement is why there is a
timeout at all**; 60 s by default, and nothing may exceed it without being asked.

The cache is keyed on the flake, not on a clock: `flake.nix`, `flake.lock`, `shell.nix` and
`default.nix`, by length and mtime. Editing any of them re-evaluates on the next call and nothing
else does.

Loading `nix.lua` at startup is **8 µs against 1,810** in a debug build — noise, and the reason the
helpers are simply parsed at startup rather than behind lazy-loading machinery.

## What it cannot do

- **Evaluate the Nix language itself.** Flake outputs need *evaluating*, not parsing — a flake that
  computes `devShells.x86_64-linux.default` in a `let … in` cannot be answered by reading the file.
  Only nix can, which is why this shells out rather than embedding a parser.
- **Work without nix installed.** `oslo.nix.available()` is false and every call answers
  `nil, "nix is not installed"`. The feature being compiled in says nothing about nix being present.
- **Complete anything for a flake you are not standing in.** `nixpkgs#<TAB>` falls through, on
  purpose.
- **Answer a command that does not speak JSON.** `nix fmt --json` accepts the flag and prints
  whatever the formatter printed; that comes back as a parse error naming the command, not a table.
- **Know that nix itself changed.** The cache watches the flake files. A new nix, a changed
  `nix.conf` or a garbage-collected store are invisible to it — delete
  `$XDG_CACHE_HOME/oslo/nix/` or simply do not pass `cache = true`.
- **Show you anything by itself.** No prompt segment, no completion, no cache is populated unless a
  config asks for it.

## Where it lives

| path | what |
|---|---|
| `crates/oslo-shell/src/nix_shell/json.rs` | `run`, `available`, the timeout and the two pipes |
| `crates/oslo-shell/src/nix_shell/cache.rs` | `document`, `keep`, `key`, and the dev-env cache |
| `crates/oslo-runtime/src/lua/api/nix.rs` | `oslo.nix.run`, argument reading, loading the helpers |
| `crates/oslo-runtime/src/lua/api/nix.lua` | every named helper, and `complete` |
| `crates/oslo-runtime/src/lua/api/json.rs` | `from_json`, the one decoder both halves share |
| `tests/nix_api_tests.rs` | the helpers, driven with `run` replaced — no nix needed |
