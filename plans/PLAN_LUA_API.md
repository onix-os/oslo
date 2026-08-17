# A richer Lua API, now that there is a VM

The `oslo.*` surface was designed against a tree walker that had no coroutines, no working
metatables, no finalizers and no byte-exact strings. Every one of those now works. This is what the
API would look like if it had been designed after the switch rather than before it.

## What the VM actually provides

Measured against the shipped binary, not assumed — all fourteen pass:

| | | | |
|---|---|---|---|
| coroutines | `coroutine.wrap` | `goto`/labels | metatable `__index` |
| `__close` (to-be-closed) | `__gc` finalizers | weak tables | `<const>` enforced |
| `string.pack` | `utf8` | `//` integer division | `io.open` on a real file |
| `os.date`/`os.time` | `debug.traceback` | | |

Plus, from luna 0.5.0 and not yet used by anything here: **frozen tables**, a **memory ceiling**,
**typed userdata** (`UserRef`), and a **serde bridge** that can serialise a Lua value into any serde
format.

## What the API's shape still assumes

Three assumptions are baked into the current surface, and each one was true of the tree walker:

1. **A handle is a table of closures**, because there were no metatables to hang behaviour on.
2. **Everything is eager**, because there were no coroutines to suspend.
3. **A failure is `nil, "message"`**, because there was no way to give an error object behaviour.

---

## 1. Handles become objects — the highest value, and the smallest change

`oslo.db.open` today answers a plain table of closures with **no metatable**. Three things follow,
all of them bad, and all of them fixed by the same change:

```lua
local db = oslo.db.open("notes")
-- `db.get("k")` — a dot instead of a colon — reads the right database anyway, because the
--   closures captured the store and ignore `self`. The module's own docs call this out as a
--   mistake it cannot detect.
-- `__begin` and `__commit` are *visible*: internals of `db:write` sitting in the public surface.
-- Nothing closes the store. It lives until the session ends.
```

With a metatable, all three go:

```lua
local db <close> = oslo.db.open("notes")   -- closed at the end of the block, deterministically
db:set("last", os.date())
-- db.get("k")  --> error: use db:get, not db.get
-- pairs(db)    --> the verbs, without the internals
```

`__index` carries the verbs, `__close` releases the store, `__gc` is the backstop for a handle
nobody closed, and `__newindex` refuses writes so a typo cannot silently add a field to a handle.

**The same shape applies to every handle oslo hands out**, and there are four:

| handle | `__close` should | today |
|---|---|---|
| `oslo.db.open` | commit and release the store | never released |
| `oslo.spawn` | wait for and reap the child | a table; the child is reaped elsewhere |
| `oslo.fs.mktempdir` | remove the directory | removed by nobody |
| a file handle oslo does not yet have | close the descriptor | there is none — see §2 |

This is the change I would make first. It is contained, it deletes a documented footgun, and
`<close>` is the single most useful thing the VM gained that a shell config can feel.

## 2. Streaming, via coroutines

**Everything the API produces is materialised whole**, because the tree walker could not suspend.
That is fine for `ls` and wrong for anything long:

```lua
-- today: this cannot be written at all
oslo.run{"journalctl", "-f", capture = true}   -- captures forever, returns never
```

Coroutines make the iterator form possible, and it is the form a shell wants:

```lua
for line in oslo.run.lines{"journalctl", "-f"} do
  if line:find("error") then oslo.ui.log(line) end
end

for entry in oslo.fs.walk("/large/tree") do ... end   -- today: one table, all of it
for row in oslo.rows("ps") do ... end                 -- a structured producer, lazily
```

Three call sites want it: `oslo.run` (process output), `oslo.fs.walk`/`ls` (large trees), and the
structured pipeline (a Lua tool that produces rows one at a time instead of building a table).

The pipeline case is the interesting one: a Lua `rows` function currently has to return the whole
table, so `mytool | head -1` still computes everything. A coroutine producer makes `head` cheap.

## 3. Errors that carry more than a sentence

The convention is `nil, message`, and it is the right convention — but the second value can be an
object now:

```lua
local ok, err = oslo.fs.read("/nope")
-- today: err is a string, and a caller wanting the path or the errno parses English
-- possible: err.path, err.code, err.kind == "not-found", with __tostring giving today's string
```

`__tostring` keeps every existing `print(err)` working, so this is additive. Worth doing for
`oslo.fs`, `oslo.run` and `oslo.db`, where the failure has structure worth reading.

## 4. A sandbox for plugins

luna 0.5.0 added **frozen tables** and a **memory ceiling**, and oslo has a plugin system that runs
somebody else's Lua. Today a plugin gets the same unrestricted `oslo` the config does.

```lua
-- a plugin could be loaded with:
--   * `oslo` frozen, so it cannot redefine oslo.run for everything else in the session
--   * a memory ceiling, so a runaway table is an error rather than an OOM
--   * `oslo.secret` filtered to the names its manifest declared (the manifest already declares them)
```

The manifest already *records* which secrets a plugin says it will read — the disclosure exists and
nothing enforces it. Frozen tables are what would turn that from a statement into a boundary.

## 5. Binary, and text that is text

`string.pack`/`unpack` and byte-exact strings both work now. Two consequences:

* `oslo.fs.read` can return bytes that are not UTF-8 without mangling them — today the boundary
  goes through `String::from_utf8_lossy`, so a binary file comes back with replacement characters.
* `utf8.len` and `utf8.offset` exist, so `oslo.ui.width` and friends could stop guessing about
  multi-byte input.

## 6. Smaller things the VM unlocked

* **`os.date` exists**, so a prompt can show a clock without shelling out. Worth a note in the docs
  rather than an API.
* **`debug.traceback`** means a failing hook could report *where* it failed. Today a broken
  `pre-cmd` says what went wrong and not where.
* **Weak tables** make a completion cache that does not pin memory.
* **The serde bridge** could give `oslo.toml.decode` and `oslo.yaml.decode` for the price of a
  dependency each, rather than a hand-written parser — the same way `oslo.json` works today.

---

## Order I would do them in

1. **Handles as objects** (§1) — deletes a footgun, hides internals that are currently leaking, and
   gives `<close>`. Contained to four call sites.
2. **Streaming producers** (§2) — the largest new *capability*; makes long-running commands and big
   trees usable at all.
3. **Structured errors** (§3) — additive, `__tostring` keeps everything working.
4. **Plugin sandbox** (§4) — turns an existing disclosure into an actual boundary.
5. **Binary-safe reads** (§5) — narrow, but currently silently corrupts.

## What I would not do

* **Expose `luna` itself.** The boundary is the thing that let the engine be swapped in one crate;
  a config reaching VM internals would weld oslo to this VM.
* **A general `oslo.async`.** Coroutines are the right primitive for *iteration*; a scheduler is a
  different project and a shell has no event loop to hang it on.
* **`oslo.time`.** `os.date`, `os.time` and `os.clock` are all there and are what people know.
