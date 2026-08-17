# A richer Lua API, now that there is a VM

The `oslo.*` surface was designed against a tree walker that had no coroutines, no working
metatables, no finalizers and no byte-exact strings. Every one of those now works. This is what the
API would look like if it had been designed after the switch rather than before it.

## What the VM actually provides

Measured against the shipped binary, not assumed — thirteen pass:

| | | | |
|---|---|---|---|
| coroutines | `coroutine.wrap` | `goto`/labels | metatable `__index` |
| `__close` (to-be-closed) | weak tables | `<const>` enforced | `string.pack` |
| `utf8` | `//` integer division | `io.open` on a real file | `os.date`/`os.time` |
| `debug.traceback` | | | |

And two do not, both to do with *when* things are released:

* **`__gc` never fires.** `setmetatable({}, {__gc = …})` runs no finalizer, and `collectgarbage()`
  frees nothing a native was holding onto. So there is no backstop for a handle nobody closes — see
  §1, which is written around that.
* **A generic `for` does not close its fourth value.** Lua 5.4 closes the loop's closing value on
  `break` and on error; luna does not, which is what would otherwise have made
  `for line in oslo.lines{…} do … break … end` reap its child.

Plus, from luna 0.5.0 and not yet used by anything here: **frozen tables**, a **memory ceiling**,
**typed userdata** (`UserRef`), and a **serde bridge** that can serialise a Lua value into any serde
format.

## What the API's shape still assumes

Three assumptions are baked into the current surface, and each one was true of the tree walker:

1. **A handle is a table of closures**, because there were no metatables to hang behaviour on.
2. **Everything is eager**, because there were no coroutines to suspend.
3. **A failure is `nil, "message"`**, because there was no way to give an error object behaviour.

---

## 1. Handles become objects — **done**

Every handle oslo hands out was a plain table of closures with **no metatable**, and three things
followed from that:

```lua
local db = oslo.db.open("notes")
-- `__begin` and `__commit` were *visible*: internals of `db:write` sitting in the public surface,
--   walked by `pairs` and callable by anyone.
-- `db.nmae = 1` added a key. Nothing refused it.
-- Nothing closed the store. It lived until the session ended.
```

`crates/oslo-runtime/src/lua/api/handle.rs` is the builder they now share. `__index` carries the
verbs, `__name` is what an error message calls the thing, `__newindex` refuses the typo, and
`__close` releases — after which every verb says the handle is closed:

```lua
local db <close> = oslo.db.open("notes")   -- the file is shut at the end of the block
db:set("last", os.date())
-- pairs(db)   --> nothing; the handle has no keys of its own
-- db.nmae = 1 --> error: oslo.db: cannot set "nmae" on a handle
```

`db.get("k")` with a dot was described here, and in the module's own docs, as reading the right
database anyway. It does not: the verbs read their first real argument at position 2, so a dot call
is `db:get: argument #2 must be a string, got no value`. The comment was stale.

| handle | what `<close>` does |
|---|---|
| `oslo.db.open` | shuts the file — the store is held in one place all the verbs share, and the session's map of open databases is weak so emptying it is enough |
| `oslo.spawn` | forgets the callback |
| `oslo.after` / `oslo.every` | stops the timer |
| `oslo.fs.mktempdir` | removes the directory |

`oslo.fs.mktempdir` answers a handle rather than a path as a result: `tmp.path` is the directory and
`tostring(tmp)` is the same string, so it still reads as a path wherever one is wanted.

**There is no `__gc` backstop, and this is a luna gap rather than a choice.** The table above says
what the metatables would carry if finalizers ran; measured, they do not — `setmetatable({}, {__gc =
…})` never fires, and `collectgarbage()` releases nothing a native was holding. So a handle nobody
closes holds what it holds until the session ends, exactly as before. `<close>` is the whole of the
improvement, and it is enough: 60 databases opened with `<close>` leave the process on 4 file
descriptors, and 60 opened without leave it on 244.

A file handle oslo does not yet have is the fifth — see §2.

## 2. Streaming — **done for the filesystem and for commands**

The section as written said everything the API produces is materialised whole. `oslo.lines` was
already the exception and had been since before the VM, so the real gap was narrower and sharper:
**the things that streamed could not be let go of, and the things that could not stream were on the
filesystem.**

```lua
for line in oslo.lines{"cargo", "build"} do oslo.ui.log(line) end
for path in oslo.fs.walk("/etc") do print(path) end          -- was: one table, the whole tree
for line in oslo.fs.lines("/var/log/syslog") do … end        -- was: the whole file, as a table

local out <close> = oslo.lines{"journalctl", "-f"}
for line in out do if line:find("error") then break end end  -- reaped when the block ends
```

**Not coroutines, and not because coroutines are missing.** They work. But the producer here is
Rust — a directory being read, a pipe being drained — and a coroutine would only be a wrapper
around the same native call. What was actually needed was somewhere to put the *cleanup*, and
`__call` on a handle is that: one value a generic `for` accepts and `<close>` releases, rather than
an iterator function and a separate closer the caller has to keep together.

luna does not close a `for`'s closing value, so a loop that `break`s still needs `<close>` or
`:close()` said out loud. A loop that runs to the end cleans up by itself.

### What is left: the structured pipeline

`oslo.register_tool`'s `rows` function still returns a whole table, so `mytool | head -1` computes
everything. **This is not a Lua-side change.** The pipeline's own contract is
`Fn(&[String], Option<&[Record]>) -> Result<Vec<Record>, String>` — `Vec<Record>`, materialised, in
`oslo_shell::data::custom`. Accepting a coroutine at the Lua boundary and draining it into that
`Vec` would give the tool author the generator notation and make `head` no cheaper, which is the
wrong half of the change. Making it lazy means an iterator contract through `oslo-shell`'s tool
plumbing, and belongs with that work rather than with this.

## 3. Errors that carry more than a sentence — **done**

```lua
local text, err = oslo.fs.read("/nope")
print(err)                       -- /nope: No such file or directory (os error 2)
print(err.kind, err.code)        -- not-found  2
```

`api/problem.rs` builds them. `kind` is one of `not-found`, `permission`, `exists`, `invalid`,
`truncated`, `timeout`, `interrupted`, `other`; `code` is the errno; `path` is what the call was
about, plus `to` for `rename` and `copy`. `oslo.db` carries `name` and a kind of its own.

**Additive in practice, not just in principle.** `__tostring` was the plan's argument and it is not
enough on its own — oslo's own tests write `err:find("nope.txt")`, which a table cannot answer. So
the metatable also has `__concat`, and an `__index` that falls through to the string library, which
means `err:find`, `err:match` and `err:upper` all still work on the message. The fallthrough is
compiled Lua rather than a native, because it has to reach `string`, and a Rust callback indexing a
VM global on every miss is the expensive way to write `string[key]`.

`type(err)` is `"table"` now. That is the whole of the break.

`oslo.run` is not included: its failure is already `r.ok`, `r.status` and `r.err`, which is
structure, and wrapping the status in an object would only add a layer.

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

1. ~~**Handles as objects** (§1)~~ — **done**: `api/handle.rs`, and the four call sites use it.
2. ~~**Streaming producers** (§2)~~ — **done** for `oslo.fs.walk`, `oslo.fs.lines` and
   `oslo.lines`; the structured pipeline needs an `oslo-shell` change first.
3. ~~**Structured errors** (§3)~~ — **done**: `api/problem.rs`, `oslo.fs` and `oslo.db`.
4. **Plugin sandbox** (§4) — turns an existing disclosure into an actual boundary.
5. **Binary-safe reads** (§5) — narrow, but currently silently corrupts.

## What I would not do

* **Expose `luna` itself.** The boundary is the thing that let the engine be swapped in one crate;
  a config reaching VM internals would weld oslo to this VM.
* **A general `oslo.async`.** Coroutines are the right primitive for *iteration*; a scheduler is a
  different project and a shell has no event loop to hang it on.
* **`oslo.time`.** `os.date`, `os.time` and `os.clock` are all there and are what people know.
