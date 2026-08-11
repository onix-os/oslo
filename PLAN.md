# oslo.nix

Everything `nix` can answer as JSON, reachable from Lua as ordinary tables. Behind the `nix` cargo
feature, which already exists and today has no way in.

**No subcommand.** `oslo nix …` is not part of this and will not be. The feature's whole surface is
a Lua table, so what it grows into is decided in a config or a plugin rather than in Rust.

## The shape, and why it is generic

The obvious design is a function per useful nix command — `oslo.nix.metadata()`, `oslo.nix.show()`,
`oslo.nix.config()`. Three measurements say not to.

**The `--json` surface is 23 commands wide and version-shaped.** On nix 2.34.6, grepping every
subcommand's own help for `--json` finds these:

```
build            derivation show  flake metadata   fmt              path-info
config show      eval             flake prefetch   formatter run    print-dev-env
                 flake archive    flake show       help-stores      profile list
nar ls           realisation info search           store info       store ls
store make-content-addressed      store prefetch-file               flake info
```

Twenty-three hand-written wrappers is a lot of Rust to maintain against a tool that is explicit
about its interface being unstable, and every nix release moves the list.

**The help text lies.** `nix registry list --help` advertises `--json`; `nix registry list --json`
answers `error: unrecognised flag '--json'` and exits 1. A generated wrapper list would have shipped
a function that cannot work. A generic call reports that as a value and the config decides.

**The conversion is already written.** `from_json` in `crates/oslo-runtime/src/lua/api/json.rs:58`
turns any `serde_json::Value` into Lua — arrays and objects tagged with `__json` so they round-trip,
`null` decoded as `false` rather than `nil` so a null field does not vanish. It is private; making
it `pub(super)` is the entire bridge. There is no second JSON path to get wrong.

So: **one call in Rust, names in Lua.**

```lua
oslo.nix.run{"flake", "metadata"}     -- the primitive: any nix command, JSON in, table out
oslo.nix.metadata()                   -- written in Lua, on top of run
```

The named helpers are Lua because that is what makes them extensible. A plugin adding
`oslo.nix.closure_size()` is a Lua file, not a patch to the shell.

## What the primitive does

```lua
local doc, err = oslo.nix.run{"flake", "metadata", "--flake", "nixpkgs", cache = true, timeout = 30}
```

| | |
|---|---|
| argv | the positional entries, passed to `nix` verbatim — never through a shell |
| `--json` | appended once, unless the caller already wrote it |
| experimental features | `--extra-experimental-features 'nix-command flakes'`, as `nix_shell::command` already does |
| returns | the decoded document, or `nil, message` |

**`Command::new("nix")`, not the shell.** `nix_shell::apply_with` runs its one fixed command through
`eval_command_substitution`, which needs an `Environment` and a quoting function
(`nix_shell.rs:133`). This takes argv from a Lua list, where quoting is a hazard and not a
convenience, and it must work from any Lua context.

**Exit status is a value, not an error.** `registry list --json` fails; `flake metadata` in a
directory with no flake fails. Both are answers a config wants to branch on, so a non-zero exit
returns `nil, stderr`.

### timeout

Required, because of one measurement: **`nix search nixpkgs ripgrep --json` took 46 seconds** on a
cold eval cache. Anything that can block a prompt for 46 seconds needs a ceiling it cannot exceed by
default. Default 60 s, `timeout = n` to change it, and exceeding it is `nil, "timed out"`.

## Caching, and how little of it is ours

The cold/warm gap is real but nix mostly closes it itself:

| command | cold | warm |
|---|---|---|
| `flake metadata` | 264 ms | **27 ms** |
| `flake show` | 455 ms | **34 ms** |
| `config show` | — | 22 ms |
| `search nixpkgs ripgrep` | 46 s | (evaluation cache) |

An earlier read of this said `flake show` costs 455 ms and needed caching on the flake's
`fingerprint`. That was the cold number. Warm it is 34 ms, because nix keeps its own evaluation
cache — so the fingerprint scheme would have been machinery bolted on top of a cache that already
works.

What is left for oslo is the cold case and `search`. `cache = true` opts in, reusing the key
`nix_shell::key` already computes — argv plus length-and-mtime of `flake.nix`, `flake.lock`,
`shell.nix`, `default.nix` (`nix_shell.rs:174`). Editing the flake re-evaluates immediately, nothing
else does. Off by default: a config asking for `store info` wants the store's answer, not last
week's.

**Cached documents are not secrets, and `print-dev-env` is.** `nix_shell::remember` writes `0o600`
because a dev shell's environment holds tokens (`nix_shell.rs:224`). This cache inherits that mode
rather than reasoning about which documents deserve it.

## What Lua gets, layer by layer

**Rust — one function.** `oslo.nix.run`. Plus `oslo.nix.available()`, which answers whether the
`nix` binary is on `$PATH` at all, so a config can be written once and run on a machine without nix.

**Lua — the named helpers**, shipped as defaults and replaceable:

```lua
oslo.nix.metadata(flake)   -- flake metadata:  description, dirty, path, url
oslo.nix.inputs(flake)     -- the lock's nodes, each with its pin date and age in days
oslo.nix.outputs(flake)    -- flake show, flattened: devShells, packages, apps per system
oslo.nix.shells(flake)     -- just the devShell names for the current system
oslo.nix.config()          -- config show
oslo.nix.dirty()           -- true when metadata has a dirtyRevision
```

`inputs` is the one worth having. The lock alone — no evaluation, 27 ms — knows exactly how stale a
project is, and nothing in the shell tells you today:

```
flake-utils    pinned 2024-11-13    636 days
nanopb-src     pinned 2024-12-01    618 days
nixpkgs        pinned 2026-04-09    125 days
systems        pinned 2023-04-09   1220 days
```

That is `locks.nodes[*].locked.lastModified`, arithmetic against now, and nothing else.

## The two things built on it

Both are the doors discussed instead of a subcommand. Both are thin once `run` exists.

**Completion for the real `nix` binary.** `oslo.completion.for_command` is already the supported
hook (`crates/oslo-ui/src/completion.rs:110`, the same mechanism the docs describe for `git`), so
this is a shipped Lua file, not new Rust: on `nix build .#<TAB>`, `nix develop .#<TAB>`,
`nix run .#<TAB>`, complete from `oslo.nix.outputs()`. 34 ms warm is inside a keystroke's budget;
cold it is 455 ms once, which is what `cache = true` is for here.

**A prompt fact.** `oslo.git` is a native provider a config turns into a segment
(`crates/oslo-runtime/src/lua/api/prompt.rs:54`). Here the provider is `oslo.nix.metadata` and
`oslo.nix.inputs`, and the config decides whether a 1220-day pin is worth a character on screen.
Nothing is shown by default.

## Order

Each step ends with `make verify` green and is its own commit.

1. **`from_json` becomes `pub(super)`.** One word, no behaviour.
2. **`oslo.nix` with `run` and `available`**, gated on `#[cfg(feature = "nix")]` at
   `lua/api/mod.rs`, beside the `direnv` table it will sit next to (`mod.rs:179`). Tests for: argv
   passthrough, `--json` not doubled, non-zero exit as `nil, msg`, timeout, nix absent.
3. **The cache**, `cache = true`, reusing `nix_shell::key`. Needs that function to be `pub(crate)`
   and `nix_shell.rs` is at 545 of 600 lines, so the key and cache functions move to
   `nix_shell/cache.rs` in this step.
4. **The Lua helpers** — `metadata`, `inputs`, `outputs`, `shells`, `config`, `dirty`.
5. **Completion for `nix`**, as a Lua file on `for_command`.
6. **Documentation** — `docs/features/` gets a page, and `README.md` the feature row.

## Verification

- `make verify` after every step. `make test` runs `--all-features`, so these tests do run.
- **`make build TYPE=minimal` must still build**, and `oslo-minimal` must have no `oslo.nix` — the
  same contract `scratch` and `direnv` have.
- **The tests cannot require nix.** CI has no nix, so `run` is tested against a fake `nix` on
  `$PATH`, and `available()` is what the helpers check. A test that shells out to real nix is a test
  that fails on someone else's machine.
- Binary-size delta measured against `develop`, as the other three features were.

## What this does not do

- **No subcommand, now or later.** If something wants to be typed, it is a Lua tool via
  `register_tool`, in a config.
- **No evaluation of the Nix language.** `rnix` parses the language, but flake outputs need
  *evaluating* — melodi's flake computes `devShells.x86_64-linux.default` in a `let … in`. Only nix
  can answer that, which is why this shells out.
- **No crate.** `nix_rs` is 205 crates and brings back `tokio`, to replace roughly a hundred lines
  that already work.
- **Nothing runs on its own.** No prompt segment, no completion, no cache is populated unless a
  config asks. Arriving in a flake directory with this feature on and nothing configured behaves
  exactly as it does today.
- **It does not make nix fast.** A cold `search` is 46 seconds and this cannot change that; it can
  only refuse to wait forever.
