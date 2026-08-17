# Plugins

Something you install once and then have, written in Lua, using only public `oslo.*` API. A plugin
adds commands to the shell you type at — and nothing else in oslo had to change for it to, because
`oslo.register_builtin` and the hooks were already there.

> ## The database and the veto are in every build; installing is not
>
> Two halves, and the line between them is where policy starts.
>
> | | `oslo` | `oslo-minimal` |
> |---|---|---|
> | `oslo.db` — a database a config owns | yes | **yes** |
> | a hook that can decline to record a line | yes | **yes** |
> | `oslo plugin`, manifests, loading | yes | no |
>
> A database and a hook that can decline to write something down are *capabilities*: small,
> self-contained, and useful to a config that will never install anything. Behind a switch they
> would give oslo two dialects, where a config has to ask whether `oslo.db` exists before using it.
>
> Installing is different in kind — it fetches somebody's code, decides whether to trust it, and
> reserves command names on their behalf. A `/bin/sh` on a distribution has no use for any of that.
> It is behind the **`plugin`** cargo feature and costs **108 KB**: 5,902,720 bytes without it
> against 6,013,312 with. In `oslo-minimal` the word `plugin` falls through to `$PATH`.

## What a plugin is

A directory with a manifest and some Lua:

```text
notes/
  plugin.lua      what it declares — a table, and nothing else
  init.lua        the plugin proper
```

```lua
-- plugin.lua
return {
  name     = "notes",
  version  = "0.1.0",
  entry    = "init.lua",
  builtins = { "note" },
  -- requires = ">= 0.2.29",   -- optional: the oldest oslo it will run on
  -- secrets  = { "gh-token" },-- optional: the secrets of yours it will read
}
```

```lua
-- init.lua
local db = oslo.db.open("notes")

oslo.register_builtin("note", function(argv)
  if argv[2] then
    db:set(os.date("%Y-%m-%dT%H:%M:%S"), argv[2])
  else
    for _, key in ipairs(db:keys()) do print(key .. "  " .. db:get(key)) end
  end
  return 0
end)
```

```sh
oslo plugin install ~/src/notes
note "remember this"
note
```

## How it works

```
oslo plugin install        reads plugin.lua, hashes the files, copies them in, writes the index
      ↓
$XDG_DATA_HOME/oslo/plugins/
  index.json               every plugin's declarations, flattened — the only file startup reads
  notes/                   the plugin
  notes.kv                 its database, if it opens one
      ↓
startup                    reads index.json; nothing else
      ↓
you type `note x`          the loop sees the name, checks the hash, runs init.lua
      ↓
`note` is now a builtin    and the line runs
```

### Loaded by the loop, not by a stub

The obvious design is a stub builtin per declared name, which loads the plugin when called. **It
cannot work**, and the shell says why:

```text
oslo: note: shell state is busy; an oslo.* call that reaches the shell cannot run from here.
```

A builtin runs *while the shell holds its state*, and `oslo.register_builtin` needs that state to
register anything — so a plugin loaded from inside a stub could never register the builtin it was
loaded to provide. The loading therefore happens one step earlier, in the read loop, where nothing
is held.

**Every word of the line is considered, not only the first.** `note x | wc -l` and `true && note y`
both need the plugin and neither has it at the front. The cost of being generous is a plugin loaded
because its name appeared as a filename — a plugin the user installed, doing its job slightly early.

### Why a generated index

A manifest is the right thing to *write* — it is Lua, in a Lua-first shell. It is the wrong thing to
read ten of at startup: each is a parse and an evaluation, to learn a list of names. `install`
flattens every manifest into one `index.json`, and a session reads that alone.

### The manifest cannot do anything

`plugin.lua` is evaluated in a **fresh interpreter with no `oslo` global**. A manifest that tries to
register a builtin, open a database or read a file finds nothing to do it with. That is what lets
`oslo plugin install` show you what a plugin claims *before* you decide to trust it.

### Saying which oslo it needs

```lua
requires = ">= 0.2.29"    -- or just "0.2.29"; they mean the same
```

A **minimum, and only a minimum**. A plugin knows what it needs — `oslo.db` arrived in a particular
release, and calling it in an older shell is a nil index — but it cannot know what a *later* oslo
will break, so an upper bound would be a guess that goes stale and locks working plugins out of new
releases.

Checked when it is installed *and* every time it is loaded, because the oslo a plugin was installed
against is not necessarily the one running now: downgrading, or copying a home to an older machine,
both leave a plugin recorded as fine and unable to work. Anything that is not a version — `^0.2`,
`> 0.2`, a typo — is a manifest error when it is read, rather than a plugin that silently never
loads.

The version it is compared against is the one `oslo --version` prints and `oslo.version` reports.
Those used to disagree — the binary said 0.2.29 while `oslo.version`, read from a different crate in
the workspace, said 0.2.21 — which would have made a requirement meaningless: an author reads one
number and the check compares another. There is one now.

## Trust

`install` records what the plugin's `.lua` files hashed to. Loading recomputes and compares; a
mismatch refuses and says so. This is the model `.envrc` already uses.

```sh
oslo plugin list          # notes   note   CHANGED — run `oslo plugin allow`
oslo plugin allow notes   # after looking at what changed
```

So `git pull` inside a plugin stops it loading until you say so, which is the point: an update is
somebody else's new code arriving on your machine. It is also why **`install` requires a revision**
for a git source — a branch would be a different plugin tomorrow, and a gate that refuses every
morning teaches people to allow without reading.

Only `.lua` is hashed. Editing a README is not a change to what a plugin will do, and hashing it
would make every documentation edit a refusal to run.

## The database

```lua
local db <close> = oslo.db.open("notes")
db:set("k", "v")          -- bytes, exactly: no trimming, no added newline
db:get("k")               -- "v", or nil
db:has("k")               -- an empty value is still present
db:delete("k")
db:keys("draft/")         -- every key under a prefix, in order
db:write(function(w)      -- one transaction; nothing lands if it raises
  w:set("a", "1")
  w:delete("b")
end)
```

**A handle is an object.** The verbs live behind `__index`, so `pairs(db)` walks nothing, a typo
(`db.nmae = 1`) is refused rather than quietly added, and `db.get("k")` with a dot is a message
rather than a read of the wrong key. `<close>` shuts the file at the end of the block and every verb
says so afterwards; a handle without it is released when it is collected, so what `<close>` buys is
the moment rather than the release. The same shape is what `oslo.spawn`, `oslo.after`/`oslo.every`
and `oslo.fs.mktempdir` answer with.

`open` takes a **name, never a path**. `oslo.db.open("../history")` is refused before anything is
opened, so a plugin cannot reach out of the directory these live in — oslo's own history and
tracking store included. One file per plugin, mode `0600`, which is also what makes uninstalling
an `rm`.

**It does not stop one plugin opening another's**, and it was described here as though it did.
`oslo.db.open("notes")` from a plugin called `weather` is accepted: the only check is on the shape
of the name. Every plugin runs on one interpreter with one `oslo` global and can read any file this
user can through `oslo.fs`, so a name check could never have been the thing that separated them.
What the databases buy is a file per plugin — findable, removable, and not a shared blob — and that
is worth having on its own.

## Secrets

A plugin can keep values encrypted, and can ask to read yours. Both are in
[secrets](secrets.md#what-a-plugin-may-reach); the short of it:

```lua
-- plugin.lua
secrets = { "gh-token" }    -- names, never a wildcard; shown at install, before you decide

-- init.lua
local mine = oslo.secret.mine()      -- its own store, encrypted, no name to pass
mine:set("cursor", "42")
oslo.secret.get("gh-token")          -- yours, and only what it declared
```

**A disclosure, not a sandbox**, for the reason [Trust](#trust) gives: a plugin that wants your
token can shell out to `oslo secret get` whatever its manifest says. The declaration makes a plugin
*catchable* — it is a claim, printed before trust is decided, that its behaviour can be held against.
And as above, a plugin's encrypted store is protected from the disk rather than from other plugins.

## A ceiling on the load

A plugin's entry file runs under a memory ceiling: whatever the interpreter is already using, plus
64 MB. A load that allocates without end is stopped, and you are told which plugin and why:

```
oslo: plugin greedy: it was stopped part-way through loading: it asked for more than 64 MB of memory
```

The shell answers the next command as usual. **This is not the sandbox the section above says does
not exist** — the plugin's hooks and callbacks run later with no ceiling at all, and any of them can
start a command. What it stops is the load that would otherwise take the session down with it, which
is a mistake far more likely than malice.

## Testing one

```lua
oslo.plugin.test("a fresh install has no notes", function(t)
  t.equal(#db:keys(), 0, "the database starts empty")
  t.ok(db, "the store opened")
  t.fail("a branch this should not have reached")
end)
```

```sh
oslo plugin test              # the plugin in the current directory
oslo plugin test ~/src/notes
```

**A temporary home, which is most of the point.** `$HOME`, `$XDG_DATA_HOME` and `$XDG_CONFIG_HOME`
point at a directory that is deleted afterwards, so the database really is empty. A test run against
the author's own home is a test that passes because there are already three notes in there, and the
failure a user hits on day one is exactly the one that cannot be reproduced.

The plugin is loaded straight out of the directory with **no trust check** — the same trust as
running a script you have just written, and the alternative would be installing a plugin before being
allowed to test it. A body that raises is one failure rather than the end of the run, so a report of
five tests is still worth reading when the second one is broken. A plugin with no tests is not a
failure: `oslo plugin test` over a directory of them should not stop at the first author who has not
written any yet.

Same shape as `oslo.plugin.health` on purpose. The difference is what they ask: a health check runs
against *your* machine and asks whether this plugin can work here; a test runs against a machine with
nothing on it and asks whether it works at all.

## The veto

A `pre-cmd` handler may decline to have a line written down:

```lua
oslo.on.pre_cmd(function(c)
  if c.text:match("PRIVATE") then
    return { record = false }
  end
end)
```

**It joins the leading-space convention rather than running beside it.** Every sink already asks one
flag, so a veto is that same condition by a second route — it cannot reach a sink the space does not,
or miss one it does. What is suppressed: editor history, `$HISTFILE`, the tracking database,
frecency, the terminal's semantic mark, the window title, the slow-command notice, and `set -x`.

The table is additive and every field optional — `{}` says nothing, `{ text = … }` rewrites,
`{ cancel = true }` refuses to run it. Only an explicit `record = false` suppresses, because reading
an absent field as a veto would make every table-returning handler hide its line by accident.

## Measurements

| | |
|---|---|
| the `plugin` feature | +108 KB on the static musl binary |
| an installed, unused plugin | one line in a JSON file; no Lua parsed, no Lua run |
| a plugin in use | its own Lua, once per session |

## What it cannot do

- **No `oslo <plugin>` subcommand.** `oslo -c` and scripts never read `init.lua` and never read
  the index; a plugin extends the shell you type at, and a script depending on one would break for
  anybody who had not installed it.
- **Reserve a name before something mentions it.** `type note` before `note` has ever been typed
  reports it as not found — because it is: the plugin providing it has not run. Nothing checks that
  a plugin delivered what its manifest promised, either.
- **Sandbox anything.** A plugin is Lua with the whole `oslo.*` API, which includes running
  commands. The hash gate decides *whether* you trust it, not what it may do once you have.
- **Survive its own bugs.** A plugin that raises while loading is reported and skipped for the rest
  of the session; one that hangs hangs the shell.
- **Resolve dependencies, or find plugins by name.** There is no registry. A plugin that needs
  another says so in prose.
- **Pick up a plugin installed mid-session.** The index is read once at startup, the same rule the
  config follows.

## Where it lives

| path | what |
|---|---|
| `crates/oslo-base/src/store.rs` | the database behind `oslo.db`: names, limits, where files go |
| `crates/oslo-runtime/src/lua/api/db.rs` | `oslo.db` itself — `open`, and the handle's verbs |
| `crates/oslo-runtime/src/startup/repl/precmd.rs` | what a `pre-cmd` answer means, and when the line is written down |
| `crates/oslo-base/src/quiet.rs` | the flag `set -x` reads, held while a hidden command runs |
| `crates/oslo-runtime/src/plugin/manifest.rs` | `plugin.lua`, read in an interpreter that can reach nothing |
| `crates/oslo-runtime/src/plugin/index.rs` | the generated index |
| `crates/oslo-runtime/src/plugin/trust.rs` | what a plugin hashes to, and whether it changed |
| `crates/oslo-runtime/src/plugin/mod.rs` | loading on first mention |
| `src/cli/plugin.rs` | `oslo plugin install / list / remove / allow` |
| `tests/plugin_tests.rs` | installing and loading, through the real binary |
