# Plugins

Something you install once and then have, written in Lua, using only public `oslo.*` API. Not a new
extension mechanism — oslo already has most of one — but the three things missing from it, plus a
way to install and trust the result.

**The test this design has to pass**: the secrets feature, deleted from this tree on 2026-08-12,
must be expressible as a plugin with no Rust change. It is the honest test because it needed a
database, a command, and the ability to stop a command being written down — one of which oslo could
not do at all.

## What already exists

More than it looks, and none of it should be rebuilt.

| | |
|---|---|
| `oslo.register_builtin(name, f)` | a Lua function *is* a shell builtin, ahead of `$PATH`, with its return value as the exit status |
| `oslo.register_tool{…}` | a command that produces rows, so `foo \| where …` works |
| `oslo.on.*` | 19 hook points, including `pre_cmd` and `post_cmd` |
| `oslo.completion.for_command` | per-command completion |
| `oslo.keys`, `oslo.prompt`, `oslo.ui.*` | keybindings, prompt segments, the widget set |
| `package.path` | already prefers `~/.config/oslo/lua`, so `require "foo"` works today |
| `conf.d/*.lua` | already documented as the place "a plugin, a package manager or a dotfile repo" adds a line without editing a file it does not own |
| `kv::Store::open(path)` | a database at an arbitrary path, private-mode, with transactions — **Rust only** |

A plugin that wants to add a command and a keybinding needs nothing new. That is why this plan is
short.

## The three gaps

**1. The database has no Lua surface.** `kv::Store` exists and is used by history and tracking;
nothing in `oslo.*` reaches it.

**2. No hook can stop a command being recorded.** `pre_cmd` may observe or rewrite; it cannot say
"run this, and write nothing down". This is why a privacy plugin is impossible today, and it is the
gap that decides whether this design passes its own test.

**3. Nothing installs anything.** `require "foo"` works if you put the file there yourself. There is
no notion of a plugin as a unit with a name, a version, a home, and a decision about whether you
trust it.

## Decisions taken

Settled with the user before writing this, and each closes off a design that would otherwise be
rediscovered later:

1. **A plugin's command is a builtin inside oslo. There is no `oslo <plugin>` subcommand.** This is
   the decision the rest of the plan rests on: `oslo -c` and the tool dispatcher do not read
   `config.lua`, and making them would put plugin loading on the startup path of every
   non-interactive shell in every script.
2. **One database file per plugin.** Uninstall is `rm`; no key collisions; no plugin can read
   another's data by guessing a prefix.
3. **Key→value with transactions**, a thin wrapper over the existing store. Values are opaque
   strings; a plugin wanting structure uses `oslo.json`.
4. **A manifest names the builtins; the plugin's Lua runs on first use.** Ten plugins must not cost
   ten chunks of Lua on every prompt.
5. **Installed from a local path or a git revision.** No registry, no name resolution service.
6. **Trust is a hash gate**, the one `.envrc` already uses: allow is keyed on the contents, so an
   update is a different hash and asks again.
7. **A veto suppresses everything oslo writes down** — history, `$HISTFILE`, tracking, frecency,
   terminal marks, notices and `set -x`. Not a subset: a credential that leaks into frecency is
   still leaked.

## The design

### Where a plugin lives

```text
$XDG_DATA_HOME/oslo/plugins/
  index.json              generated; the only file startup reads
  <name>/
    plugin.lua            the manifest: a table, no side effects
    init.lua              the plugin proper, run on first use
    …
$XDG_DATA_HOME/oslo/plugins/<name>.kv    its database, if it opens one
```

### The manifest, and why there is also an index

`plugin.lua` returns a table and does nothing else:

```lua
return {
  name     = "secrets",
  version  = "0.1.0",
  entry    = "init.lua",
  builtins = { "secret" },        -- names to reserve; the file runs when one is called
  tools    = { "stale" },         -- same, for row-producing tools
  requires = ">= 0.2.29",
}
```

**The index is generated because reading manifests is not free.** A Lua manifest is the right thing
to *write* in a Lua-first shell, but reading ten of them at startup means ten parses to learn ten
lists of names. `oslo plugin install` therefore writes every manifest's declarations into one
`index.json`, and startup reads that single file — `serde_json` is already a dependency, and the
index is generated, so nobody hand-writes JSON.

If the index is missing or older than a plugin's manifest, it is rebuilt. A stale index is a
performance bug, never a correctness one.

### Loading

**The stub design in the first draft of this plan does not work, and the shell says why:**

```text
oslo: note: shell state is busy; an oslo.* call that reaches the shell cannot run from here.
```

A builtin runs *while the shell holds its state*, and `oslo.register_builtin` needs that state to
register anything — so a plugin loaded from inside a stub builtin can never register the builtin it
was loaded to provide. That is not a fixable stub; it is the wrong place to load from.

So the loop does it. One step before a line runs, with nothing held, any plugin whose declared name
appears as a word in the line is loaded: trust hash checked, entry file run, and by the time the
line executes the real builtin is registered and dispatch finds it like any other.

**Every word, not only the first.** `note x | wc -l` and `true && note y` both need the plugin and
neither has it at the front. The cost of being generous is a plugin loaded because its name happened
to appear as a filename — which is a plugin the user installed, doing its job slightly early.

The consequence worth knowing: **a name is not reserved until something mentions it.** `type note`
before `note` has ever been typed reports it as not found, because it is — the plugin has not run.
Nothing checks that a plugin delivered what its manifest promised, either; the shell looks the word
up as usual and "command not found" is the honest answer when it did not.

**Nothing outside the interactive shell loads plugins**, per decision 1. `oslo -c 'note x'` does not
work and is not meant to.

### `oslo.db`

```lua
local db = oslo.db.open("secrets")        -- $XDG_DATA_HOME/oslo/plugins/secrets.kv, mode 0600
db:set("k", "v")
local v = db:get("k")                      -- string, or nil
db:delete("k")
for k in db:keys("prefix/") do … end
db:write(function(w)                       -- one transaction; nothing lands if it errors
  w:set("a", "1")
  w:set("b", "2")
end)
```

`open` takes a *name*, not a path: a plugin cannot open another plugin's database or point one at
`/etc`. The name is validated the way a plugin name is.

### The veto

`pre_cmd` may return a table instead of a string:

```lua
oslo.on.pre_cmd(function(c)
  if looks_like_a_credential(c.text) then
    return { record = false }              -- runs; nothing is written down
  end
end)
```

Returning a string still means "run this instead", unchanged. `{ text = …, record = false }` does
both. `record = false` suppresses every sink listed in decision 7, which is the list
`Sensitivity::is_private` already gates in the REPL — the seam exists, and this hands it to Lua.

**The risk, stated plainly**: a plugin with a wrong predicate silently stops recording anything, and
the symptom is an empty history with no error. The mitigation is that the veto is per command and
never sticky, and that `oslo plugin list` says which plugins can use it.

### `oslo plugin`

```text
oslo plugin install <path|github:user/repo@rev>
oslo plugin list
oslo plugin remove <name>
oslo plugin allow <name>          # after an update changes the hash
```

`install` copies or clones to the plugin directory, reads the manifest, refuses a name that collides
with an installed plugin or a shell keyword, writes the index, and asks for trust once — showing the
builtins it will reserve. It never runs the plugin's code.

`remove` deletes the directory and rewrites the index. **It leaves the database**, and says so: that
is the user's data, and a plugin manager that deletes your password vault because you reinstalled it
is a plugin manager nobody should run.

## Order

Each step ends with `make verify` green and is its own commit.

1. **`oslo.db`** — the Lua surface over `kv::Store`. Self-contained, useful on its own, no policy.
2. **The veto** — `pre_cmd` returning a table, and the REPL honouring `record = false`. Tests must
   cover every sink in decision 7, because the failure mode is silent.
3. **The manifest and the index** — reading, validating, rebuilding when stale. No loading yet.
4. **Stub registration and load-on-first-use**, including the trust check at load.
5. **`oslo plugin`** — install, list, remove, allow.
6. **Documentation**, and a worked plugin: the deleted secrets feature, rebuilt as one, as proof the
   design passes its own test.

## Verification

- `make verify` after every step; `make build TYPE=minimal` still builds.
- **Startup cost, measured against `develop`.** Loading eight Lua chunks at startup was measured at
  8 µs against 1,810 µs in a debug build when `nix.lua` was added — the index must stay in that
  class, and step 4 is where it could stop being.
- A plugin that vetoes must be shown, by test, to keep the command out of *each* of: editor history,
  `$HISTFILE`, the tracking store, frecency, terminal marks, notices, `set -x`.
- A plugin whose files changed must refuse to load until `oslo plugin allow`.

## What this does not do

- **No `oslo <plugin>` subcommand**, now or later. Decision 1.
- **No registry and no dependency resolution.** A plugin that needs another one says so in prose.
- **No sandbox.** A plugin is Lua with the full `oslo.*` API, which includes running commands. The
  hash gate decides *whether* you trust it, not what it may do once you have.
- **No isolation of failure beyond what hooks already give.** A handler that errors is reported and
  the rest still run; a plugin that hangs hangs the shell.

## The feature line, decided

**`oslo.db` and the veto are in every build. Installing and loading plugins is behind the `plugin`
cargo feature.**

The line is drawn where the *policy* starts. A database and a hook that can decline to write
something down are capabilities — small, self-contained, and useful to a config that will never
install anything. Putting them behind a switch would give oslo two dialects, where a config file has
to ask whether `oslo.db` exists before using it.

Installing is different in kind: it fetches somebody's code, decides whether to trust it, and
reserves builtin names on their behalf. A `/bin/sh` on a distribution has no use for any of that,
and it is exactly the sort of thing that should be absent rather than merely unused.

So `oslo-minimal` has `oslo.db`, has the veto, and has no `oslo plugin` and no plugin loading —
steps 1 and 2 carry no `#[cfg]`, steps 3 to 5 are entirely behind one.
