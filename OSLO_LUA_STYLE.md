# The oslo Lua config style

One config style across oslo, hexe, lule and pixy. **oslo is where it comes from** — this file
records it so the other three can match, and so oslo itself stays consistent as it grows. The same
document sits in each of the four repositories, each with its own examples.

The whole of it: **assign the settings, register the behaviour, return nothing.** A description —
a plugin, preset or theme — stays a table; it is just handed to something instead of returned.

```lua
local oslo = require "oslo"

oslo.vi.enabled = true
oslo.on.key(function(k) ... end)
oslo.keys["f4"] = function(line) ... end
```

## Where oslo stands today

oslo follows all five rules. `~/.config/oslo/init.lua` has no `return` in it, settings assigned onto
namespaces, behaviour registered. One thing is worth writing down anyway: a *third* pattern oslo
invented that belongs in the style rather than being treated as a deviation — keyed registration,
in rule 2.

**The naming slip this file used to record is fixed.** `oslo.on.on_key` and `oslo.on.on_report`
stuttered because the namespace already says `on`, and twenty of oslo's hooks are named
`on-something`. Every one of them now also answers to the name without the prefix —
`oslo.on.key`, `oslo.on.report`, `oslo.on.variable_change` — as one rule over the table rather than
the single hand-written alias that had made `oslo.on.key` work and `oslo.on.report` not. The
canonical name is untouched: `on-key` is still what `oslo hook list` prints and what a plugin
declares. The doubled spelling still resolves, and nothing in the tree writes it any more.

## The five rules

### 1. Settings are assigned, not declared in a table

```lua
oslo.misc.welcome = false          -- yes
oslo.vi.cursor_insert = "underscore"
oslo.suggest.sh_sources = { "predict", "history", "path" }

return tool.setup({ settings = { welcome = false } })   -- no
```

A setting the config never mentions is left alone — the environment or a flag decides it. There is
no "defaults table" to keep in sync, and no way to blank a setting by forgetting to list it.

**Nested data stays nested.** The rule is about how the setting is *delivered*, not about
flattening its value. A structured setting is a table on the right-hand side:

```lua
oslo.prompt.left = {
  command = "pixy",
  args = { "render", "prompt.left", "--target=ansi", ... },
  timeout_ms = 10,
  async = true,
}
```

That is rule 1 done right, and it is the answer to "but my settings are a big tree": assign the
tree. Turning a deep table into fifty assignment statements is worse than the table, and the rule
never asked for it.

Namespace where there is genuinely more than one subsystem — oslo has `vi`, `suggest`, `builtin`,
`misc`, `prompt`, `lua`, `term`. lule has one subject and keeps its settings flat. A namespace with
one member is a directory with one file in it.

### 2. Behaviour is registered, and registration repeats

```lua
oslo.on.key(function(k) ... end)     -- as often as you like
```

This is the rule the others follow from. If a hook is one field in a returned table there is
exactly one place to put anything, so everything a config does piles into one function. If it is a
call, the config is as many small named functions as it wants:

```lua
local function write_cache(c) ... end
local function recolour_terminals(c) ... end

lule.on.colors(write_cache)
lule.on.colors(recolour_terminals)
```

Registrations apply in the order they were made. **One that raises is reported and the rest still
run** — a mistake in the third handler is not a reason to skip the fourth, which has nothing to do
with it.

**Keyed registration is the second form, and it is oslo's.** Where a registration is identified by
something — a key name, a zone name, a command — assigning into a map is better than appending to
a list, because it is idempotent: registering `f4` twice replaces rather than fires twice.

```lua
oslo.keys["f4"] = function(line) ... end

for c in ("abcdefghijklmnopqrstuvwxyz"):gmatch(".") do
  oslo.keys["alt-" .. c] = function(line) ... end
end
```

This is not a deviation from the rule, it is the half of it that solves the append-only form's one
real weakness. A list registrar re-run duplicates its entries; a keyed one is safe to run twice.
Use a list where entries genuinely accumulate (handlers for one event), a map where each entry has
an identity that can be replaced (keys, zones, named templates).

### 3. The file returns nothing

No `return tool.setup({...})`, no `return M`. The config is a list of statements, so it can compute
freely between them — the 26 `alt-<letter>` bindings above are a loop, and this is an `if`:

```lua
if oslo.term.kitty_keyboard() then
  oslo.lua.enter = "newline"
end

if on_path("pixy") then
  oslo.prompt.left = { ... }
end
```

That last shape is worth noticing: the config **degrades on the machine it is running on**. The
same file works where pixy is not installed, because it asked. A returned table cannot ask.

The host reads its settings off the module table after the chunk has run, so there is nothing to
hand back.

This is about the **config** file. A plugin, preset or theme is a different kind of file — rule 4.

### 4. A table is an argument, never a fragment somebody has to merge

Registration is for behaviour. A plugin, preset or theme is a *description* — a pile of values —
and a table is the right shape for one. The two coexist perfectly well, as long as the table is
handed **to** something:

```lua
make.recipe{ name = "smoke", deps = { "build" }, run = function() ... end }   -- an argument
```

`.make.lua` already gets this right: a recipe is a table, and it is passed to a registrar rather
than collected into a list of recipes that something returns.

What goes wrong is the table that is only returned, leaving the caller to assemble it. hexe pays
this today — `layout.lua` returns a fragment, and `init.lua` carries the `or {}` defaults, a
`__hexe_type` shape check and a two-branch merge to get the pieces back out. Every further fragment
file re-implements the same dance.

Two ways to keep it an argument instead:

- **registered** — the fragment calls the registrar itself and returns nothing; the config just
  requires it.
- **discovered** — the host scans a directory and merges the returned tables itself, the way
  lazy.nvim does. Then `return {...}` is fine: the merge exists once, in the host.

nvim splits along exactly this line — `vim.o.background = "dark"` and sixty-odd `vim.keymap.set(...)`
in the config, `return { "author/plugin", opts = {...} }` in a plugin spec that lazy discovers and
merges.

### 5. A handler's return value means something, or nothing

Where a handler can influence what happens next, `nil` means "not mine, carry on" and a table means
"here is what to do instead". oslo is the exemplar:

```lua
oslo.on.key(function(k)
  if k.language ~= "sh" then return end          -- not mine
  if k.name == "enter" and k.text == "" then
    return { text = "la --git-ignore", submit = true }
  end
end)
```

Where a handler is purely a side effect — lule's `on.colors` — the return value is ignored and the
handler returns nothing. Both are fine; what matters is that a config author can tell which kind
they are writing without checking.

## What the host implements

Small enough to be worth stating exactly:

- The module table is created, the host's functions are added to it, and it is parked before the
  config chunk runs. `package.loaded['<tool>'] = tool` so `require` never touches the filesystem.
- Registrars append to a plain Lua list, or assign into a plain Lua map, on that table. Keeping
  them in Lua rather than in the host means they can also be assigned outright, and can be read
  back by the config.
- After the chunk runs, the host reads what it needs off the module table. Missing key means unset,
  not zero.
- A syntax error or a raise **at load time** stops the run and names file and line — carrying on
  with defaults silently applies something the user did not ask for. A raise **inside a handler**
  is reported and the run stands.
- Handlers are called one at a time under `pcall`, in registration order.

## Naming

- `tool.on.<noun>` for events — the thing that happened, not when. `on.colors`, `on.attach`,
  `on.key`. Not `after`, `post`, `hook`: those say when, and a config full of `after` tells you
  nothing about what any of it does.
- **A hook reached through `on` does not repeat it.** `oslo.on.key`, not `oslo.on.on_key`; the
  namespace already said it. oslo's canonical names keep the `on-` prefix — they are read outside
  the namespace too, in `oslo hook list` and in a plugin's declaration — and the field spelling
  drops it. `on.pre_cmd` is fine as it is: a before-hook that can veto genuinely has to say it runs
  before.
- Settings take the same name as the equivalent command-line flag.
- A registrar that takes a name uses it so a warning can say *which* one is wrong — and, where the
  host addresses one by name later (pixy's `prompt.left.ssh` selector, oslo's `keys["f4"]`), so it
  can be replaced rather than duplicated.
