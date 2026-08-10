# Features you can turn off

Eight parts of the shell can be switched off and on by name while it runs. The mechanism is
deliberately not a second way to configure them: **a feature is a mask over your configuration and
never an assignment to it**, so turning one back on gives you back whatever the config said rather
than a hardcoded default.

## How it works

The whole state is one process-wide `AtomicU32` in `oslo_base::feature`, holding which features are
**off**. Inverted on purpose: the default — nothing set, everything working — is then zero and needs
no initialisation, and a bit nothing ever writes cannot turn anything off.

Every place that uses a feature asks two questions and ANDs them:

```
  your configuration                        the mask
  read once, from your .lua                 an AtomicU32; bit n set = FEATURES[n] is OFF
                                            written at run time, from anywhere

     oslo.vi.enabled = true                    bit 1 (VI) = 0
              │                                      │
              └──────────────► AND ◄─────────────────┘
                                │
                                ▼
                       vi::enabled() == true      ← the only thing the editor asks

  Nothing ever flows leftwards. oslo.feature.set("vi", true) clears a bit; it does not
  write oslo.vi.enabled — which is why a shell configured for emacs cannot acquire vi
  mode by having the `vi` feature turned on, and why nothing has to remember a value in
  order to put it back.
```

That property is the reason this is a separate mechanism rather than a setter on `Settings`. The
hard part of "disable on the way in, restore on the way out" is the restore, and there is no state
here to restore or to leak.

### The table

The order is load-bearing, because the bitset indexes into it. `feature::at::*` names the indices so
that code on the keystroke and per-command paths does not walk eight strings, and a test checks
those constants against the table rather than trusting them.

| # | name | where the bit is read | ANDed with |
|---|---|---|---|
| 0 | `direnv` | `Direnv::arrive` | the files on disk: `.env.lua`, `.envrc` |
| 1 | `vi` | `vi::enabled()` | `oslo.vi.enabled`, which is `false` by default |
| 2 | `suggest` | `OsloHelper::suggest` | `oslo.suggest.sources` |
| 3 | `abbr` | the editor's expand-on-space path | `oslo.abbr` |
| 4 | `notify` | `slow_command_notice` | `oslo.notify.after`, 10 seconds by default |
| 5 | `marks` | `marks::enabled()` | whether the terminal can take marks, decided at startup |
| 6 | `finder` | `open_finder` | `oslo.finder.enabled`, which is `true` by default |
| 7 | `rm` | the builtin registry, and nowhere else | — |

Every row has a real gate site, and that is a rule rather than an observation: a name that could be
turned off with nothing reading the bit is indistinguishable from a typo in the name.

Two of the rows do something more than decline. `direnv` off is read as *there is no file here*, so
it **unloads** what is currently loaded instead of merely refusing to load the next thing —
otherwise a config that turned directory environments off on the way into a project would keep the
last project's variables for the rest of the session. `marks` covers terminal metadata and the
window title as well as the semantic boundaries, because they are written from the same place.

### Turning off a builtin

Three features name a builtin in `provides`: `direnv`, `abbr` and `rm`. Turning one off hands the
name back to `$PATH`.

```
  Registry::entry(name)
        │
        ├─ builtin_is_off(name)?     one relaxed load; disabled == 0 returns false
        │      │                     immediately, which is what makes this affordable
        │      │                     on every command
        │      └─ yes ──► None       the word falls through to $PATH like any other
        │
        └─ self.table.get(name)      nothing was ever removed from the table
```

This is **the one place a builtin is turned off**, so a disabled one is invisible to the dispatcher,
to `type`, to `command -v` and to completion at once rather than in four places that can disagree.
Because the table is untouched, turning the feature back on restores the builtin exactly as it was
registered — there is no re-registration to get wrong.

```lua
oslo.run{"type", "rm"}            -- rm is a shell builtin
oslo.feature.set("rm", false)
oslo.run{"type", "rm"}            -- /usr/bin/rm
oslo.feature.set("rm", true)
oslo.run{"type", "rm"}            -- rm is a shell builtin
```

### Predicates, and why they are re-asked

`oslo.feature.when(name, f)` attaches a predicate instead of a value. It is re-evaluated for every
directory the shell arrives in, before anything reads a feature bit:

```
  cd somewhere                    the shell starts, config already loaded
       │                                │
       └────────────► environments::arrive(dir) ◄──────────┘
                          │
                          ├─ feature::decide(dir)      FIRST, for every feature
                          │     │                      that has a predicate
                          │     ├─ returned nothing, or nil ──► leave the bit alone
                          │     ├─ raised ──► report on stderr, leave the bit alone
                          │     └─ anything else ──► set the bit to Lua truthiness
                          │
                          └─ load / unload this directory's environment
                                (which reads the `direnv` bit)
```

`decide` lives inside `arrive` rather than at its three call sites so that a fourth cannot forget
it, and it runs before the load rather than after, because the first thing a predicate is for is
deciding whether this directory's `.env.lua` should be loaded at all. The directory a shell opens in
gets the same treatment: it is the one arrival no hook has fired for yet.

A predicate that returns nothing has not answered, and is not read as `false` — otherwise a handler
with a missing `return` on one branch would turn a feature off, which is the failure hardest to
attribute to the config that caused it. A predicate that raises is reported by name and skipped, and
does not stop the others being asked.

`set` and `when` are alternatives, not layers. `set` on a feature that a predicate owns is refused
with a message pointing at the predicate, because the write would appear to work and then be undone
by the next `cd`.

### What is deliberately not a feature

**History and the tracking store, and they must never become features.** They are what the command
log and the frecency table are built from, and something downstream is entitled to assume the log is
complete rather than "complete except where a config had an opinion". A gap nobody can see is worse
than no data. `redact` and `--profile` are the controls that exist for this, and both leave a record
that is honestly shaped: redaction drops a risky line's arguments and keeps the command, a profile
separates two chronologies without truncating either.

There is a test that refuses the names `history`, `tracking`, `track`, `log`, `frecency` and
`record` outright, so the next person to want this meets an argument rather than an absence.

## What makes it different

bash's `enable -n <name>` disables one of its own builtins and hands the name to `$PATH`, which is
the shape the `rm` row here has — though bash has no `rm` builtin to disable in the first place.
What that has no equivalent of is doing the same to behaviour that is not a builtin at all, such as
key bindings or the suggestion. `shopt -u` and zsh's `unsetopt` write the option itself, so putting
one back means having remembered what it was.

oslo's answer is that the configured value is never written, so there is nothing to remember: a
predicate is simply asked again in the new directory and its answer is the entire state.

The consequence worth knowing is that `oslo.feature.get` answers about the mask alone. On a stock
shell `oslo.feature.get("vi")` is `true` while vi mode is off, because the config never asked for
it. There is no call that answers "is vi mode actually happening" — two questions stay two questions.

## Configuration

`oslo.feature` is a namespace of functions rather than a settings table, because a feature is not
configuration and the two must not look alike.

```lua
oslo.feature.set("finder", false)      -- returns the value it set
oslo.feature.set("finder", true)
oslo.feature.set("finder")             -- with no second argument, on
oslo.feature.get("finder")             -- the mask bit

for _, f in ipairs(oslo.feature.list()) do
  print(f.name, f.on, f.about)         -- all eight, in table order
end
```

Anything but `false` and `nil` is on, which is Lua's own truthiness and what somebody writing
`oslo.feature.set("vi", want_vi)` means.

```lua
oslo.feature.when("direnv", function(dir)
  return not oslo.fs.exists(dir .. "/.envrc")
end)
```

The names are `direnv`, `vi`, `suggest`, `abbr`, `notify`, `marks`, `finder` and `rm`. Anything else
raises, listing the real ones — a config that turns off `direnvv` and is quietly obeyed looks
exactly like a config that is not being read at all. `when` refuses anything that is not a function
for the same reason: a string would be stored and then fail on every directory change, a long way
from the line that caused it.

There is no shell-level spelling. No builtin, no flag, no environment variable — the whole surface
is these four Lua functions.

## What it cannot do

- **Mask anything not in the table.** Eight rows, fixed at compile time, and a config cannot add
  one. The bitset is a `u32`, so a thirty-third feature would need a wider one; a test says so.
- **Turn recording off.** See above. This is refused by name.
- **Scope itself.** One atomic for the whole process, so a `set` inside a function or a hook applies
  to the whole shell until something sets it back. A `when` predicate is the only form that
  un-applies itself, and only on arrival in a directory.
- **Reach a child process.** Nothing serialises the bitset, so a script, an `oslo -c` or a nested
  shell starts with everything on.
- **Reload on re-enabling `direnv`.** Turning the feature off unloads immediately, because the
  unload happens on the arrival path that is already running. Turning it back on does nothing until
  the next arrival: `cd` somewhere and back, or `direnv reload`.
- **Make something work that was not working.** Enabling `marks` in a script or on a terminal that
  cannot take them changes nothing, because the other half of the AND is still false. The mask can
  only ever subtract.
- **Answer whether a feature is in effect.** `get` reports the bit, not the conjunction.

## Where it lives

| path | what |
|---|---|
| `crates/oslo-base/src/feature.rs` | `FEATURES`, `at::*`, the bitset, `on`/`set`/`resolve`/`builtin_is_off`/`listing` |
| `crates/oslo-runtime/src/lua/api/feature.rs` | `oslo.feature.set/get/list/when`, the predicate registry, `decide` |
| `crates/oslo-runtime/src/startup/environments/mod.rs` | `arrive` calls `decide` before anything reads a bit |
| `crates/oslo-shell/src/env/scope/registry.rs` | `Registry::entry` and `names` — the single builtin gate |
| `crates/oslo-shell/src/direnv/mod.rs` | the `direnv` bit, read as "there is no file here" |
| `crates/oslo-ui/src/vi.rs`, `crates/oslo-ui/src/marks.rs` | `enabled()` in both — the config AND the mask |
| `crates/oslo-ui/src/lib.rs` | the `suggest` gate, before the sources are walked |
| `crates/oslo-runtime/src/startup/native.rs` | the `finder` and `abbr` gates |
| `crates/oslo-runtime/src/startup/notify.rs` | the `notify` gate |
| `tests/feature_tests.rs` | the builtin gate through a spawned binary; predicates against `decide` |
