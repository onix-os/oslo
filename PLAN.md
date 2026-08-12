# What a plugin still cannot do

The plugin system works: a directory of Lua, installed, trusted, loaded on first mention. What it
cannot do is *compose* — with the pipeline, or with another plugin. This plan is the six things
missing, in the order they are worth building.

Measured against neovim, which is the reference for "extensible in Lua" and whose model was read
rather than remembered: 139 autocommand events plus a `User` event plugins fire themselves, `vim.uv`
timers and `vim.schedule`, `nvim_create_user_command` with `nargs`/`complete`/`desc`, `vim.health`,
`vim.notify` with levels, and `vim.g`/`b`/`w`/`t` scopes.

**Work on a new branch off `develop`.**

## What oslo already has, and should not rebuild

| | |
|---|---|
| 14 namespaces | `db direnv env feature fs job json nix path predict proc re sys ui` |
| `oslo.ui` | 30 functions — widgets, tables, pagers, colour, `ask`, `filter`, `spin` |
| 22 hooks | the real moments of a shell, not a buffer/window vocabulary |
| `register_builtin`, `register_tool` | a Lua function *is* a builtin, ahead of `$PATH` |
| a settings namespace per plugin | `oslo.notes = {…}` already passes without complaint |
| `oslo.db`, the `pre-cmd` veto | a database and the right to decline being recorded |

## 1. A Lua tool cannot consume rows — only produce them

**The largest gap, and the one that undercuts oslo's flagship feature.**

```rust
// crates/oslo-shell/src/data/custom.rs:38
pub fn rows_of(name: &str, argv: &[String]) -> Option<Result<Vec<Record>, String>>
```

`run_tool` has the previous stage's rows in hand and drops them on the floor. `register_tool`
accepts an `accepts = "rows"` declaration, the planner reads it to decide the edge — and then the
input never arrives. So **nobody can write `where` in Lua**: structured pipelines are extensible at
the source and nowhere else, which is the half that matters least. A plugin can offer `notes` but
never `notes-since`, `redact` or `group-by`.

`docs/features/your-own-tools.md` states this outright — "a Lua tool is therefore always a
producer" — so the fix is closing a documented hole rather than changing a promise.

**The work**: widen `Handler` to take the input rows, hand them to the Lua function as a second
argument, and convert `Record` into a Lua table on the way in. The reverse conversion already exists
(`lua/api/tool.rs::records_of`), the planner already respects `accepts`, and `run_tool` already holds
what is needed.

```lua
oslo.register_tool{ name = "redact", accepts = "rows", produces = "rows",
  rows = function(argv, input)
    for _, row in ipairs(input) do row.token = "…" end
    return input
  end }
```

**The one behaviour change**: a config that already declares `accepts = "rows"` gets its input, where
before it got nothing. That is the bug being fixed, and no test asserts the old behaviour.

## 2. Plugins cannot talk to each other

Every one of the 22 hooks is a name oslo chose. A plugin cannot announce anything, and nothing can
listen for what another plugin did — so two plugins compose only through the filesystem.

nvim's answer is one event, `User`, fired with `:doautocmd`; it is what lazy.nvim, telescope and the
rest coordinate through without depending on each other's internals.

```lua
oslo.on.emit("notes:saved", { key = k })     -- from the plugin that did something
oslo.on.user("notes:saved", function(e) … end)  -- from anyone who cares
```

**The work**: a second registry beside the fixed `at::` indices, keyed by name. Handlers already
report-and-continue when one raises, which is the behaviour a plugin bus needs. Names are validated
like a rule id — a typo must be a refusal, not a silent subscription to an event nobody fires.

**Synchronous, like `doautocmd`.** An asynchronous bus needs an ordering story, and the one thing
worse than no events is events that arrive in an order nobody can predict.

## 3. Nothing means "later"

There is no `after`, no `every`, no "when the prompt comes back". A prompt segment that wants fresh
data must block the draw or shell out — which is exactly what the `nix` prompt segment does today,
at 6 ms per draw.

```lua
local t = oslo.after(500, function() … end)
local u = oslo.every(30000, function() … end)
u:stop()
```

**Fired from the read loop, not from an event loop.** `startup::repl` already drains deferred hooks
at a point where nothing is held (`run_deferred_hooks`), and that is where these belong. The
limitation is honest and must be documented: **a timer does not fire while a command is running.**
The alternative is a real event loop, which means bringing `tokio` back into a shell that
deliberately deleted it.

## 4. A command cannot describe itself

```lua
oslo.register_builtin("note", f)   -- a name, a function, and nothing else
```

No description, no argument spec, no completion — a plugin must separately reach
`oslo.completion.for_command`, and nothing can ever print what `note` is for. nvim's user commands
carry `nargs`, `complete` and `desc`.

```lua
oslo.register_builtin{ name = "note", run = f,
  desc = "write a note down",
  complete = function(prior, word) … end }
```

The two-argument form keeps working; the table form is additive. `type`, the completion dropdown and
`oslo plugin info` all read the same declaration.

## 5. "It is installed and nothing happens"

The commonest plugin question, and the plugin system added three new ways to reach it: the trust hash
refused, the name was already claimed by a config, the plugin registered nothing. Each is currently a
line on stderr the user has to catch as it goes past.

`oslo plugin doctor` checks, per plugin: the index parses, the directory is there, the hash matches,
the entry file exists, `requires` is satisfied, and no declared name is already taken. A plugin may
add its own check the way `vim.health` lets one — is `age` installed, is the database writable — by
registering a function the doctor calls.

## 6. The smaller ones, worth doing together

- **`oslo.state`** — session-lived, structured, not exported. Between an environment variable
  (a string, inherited by children) and `oslo.db` (durable, on disk) there is nothing, and most
  plugins want the middle.
- **Setting provenance.** `oslo config which <key>` — which file set this. Config, `conf.d` and
  plugins all write now, and "why is my keybinding not working" has no answer.
- **A description on a keybinding.** `oslo.keys["alt-n"] = { run = f, desc = … }`, so something can
  list what is bound.
- **Lazy-load by hook.** A plugin that only matters in a git repository could declare
  `load_on = "post-change-dir"` instead of being loaded because its name appeared in a line.

## Order

Each step ends with `make verify` green and is its own commit.

1. **Rows into a Lua tool** — the widened handler and the `Record` → Lua conversion.
2. **User events** — `emit` and `on.user`, with name validation.
3. **Timers** — `after`, `every`, `stop`, drained from the read loop.
4. **`register_builtin` in table form** — `desc` and `complete` beside `run`.
5. **`oslo plugin doctor`**, including a plugin's own check.
6. **The smaller four**, each its own commit.

## Verification

- `make verify` after every step, and `cargo test` with no features — steps 1 to 4 are core and must
  work in `oslo-minimal`; only the doctor is behind `plugin`.
- **A Lua `where` is the test for step 1**: a plugin verb that filters rows, in the corpus, piped
  from a built-in producer and into a built-in consumer.
- **Two plugins, one event** for step 2: one emits, the other counts, neither knows the other exists.
- **Startup cost measured against `develop`** after step 3. A timer registry read on every prompt is
  the kind of thing that costs microseconds until it does not.
- The 600-line rule will bite `data/tools/mod.rs` and `lua/api/hooks.rs`; split by subject.

## What this does not do

- **No 139 events.** nvim's vocabulary is buffers and windows, which a shell does not have. Events
  nobody fires are worse than none: they read as capability and behave as absence.
- **No `vim.uv`.** Exposing libuv means an event loop; the timers above ride the read loop instead,
  and say plainly that they do not fire during a command.
- **No async bus, no promises, no coroutine scheduler.** oslo is single-threaded through the path
  every one of these touches, and that is what makes them small.
- **No sandbox for plugins.** Unchanged from the plugin plan: the trust gate decides whether you run
  somebody's code, not what it may do afterwards.
