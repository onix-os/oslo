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

`__gc` is the one that is easy to measure wrongly, and I did: the collector is incremental, so two
`collectgarbage()` calls prove nothing. With enough collection finalizers run, and so do the `Drop`s
of the Rust values a native was holding — 60 databases opened and abandoned take the process to 244
file descriptors and back to 4 once the collector has been through.

One thing does not work:

* **A generic `for` does not close its fourth value.** Lua 5.4 closes the loop's closing value when
  the loop ends, `break` and error included; luna does not, which is what would otherwise have made
  `for line in oslo.lines{…} do … break … end` reap its child.

Plus, from luna 0.5.0 and not yet used by anything here: **frozen tables**, a **memory ceiling**,
**typed userdata** (`UserRef`), and a **serde bridge** that can serialise a Lua value into any serde
format.

## What the API's shape still assumes

Four assumptions were baked into the surface, each one true of the tree walker. None is left:

1. ~~**A handle is a table of closures**~~, because there were no metatables to hang behaviour on.
2. ~~**Everything is eager**~~, because there was nothing to hang a suspended read's cleanup on.
3. ~~**A failure is `nil, "message"`**~~, because there was no way to give an error object behaviour.
4. ~~**A string is text**~~, because the shell's own value could not hold anything else.

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

**No handle sets `__gc`**, and the reason differs by kind. For a database, a file or a pipe there is
nothing a finalizer would do that collection does not already: the verbs hold Rust values, and
collecting the handle drops them, which shuts the descriptor with no Lua involved. `<close>` buys the
*moment*, which is what matters when a config opens sixty. For a spawn, a timer or a temporary
directory a finalizer would be actively wrong — those handles are normally written for the effect
and thrown away, so `__gc` would cancel the callback, stop the timer, and `remove_dir_all` a path
somebody had copied out.

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

## 4. A sandbox for plugins — **one of three; the other two do not hold up**

### The memory ceiling — done

A plugin's entry file now loads under a ceiling of whatever the VM is already using plus 64 MB, and
the ceiling comes off when it returns. A load that allocates without end is stopped, the shell
answers the next command, and the person who installed it is told which plugin and why:

```
oslo: plugin greedy: it was stopped part-way through loading: it asked for more than 64 MB of memory
```

Its own `<close>`-shaped honesty: the plugin's *hooks* run later, unbounded. What this stops is the
load that would take the session down with it — the runaway table, not the hostile author.

### Secrets filtered to the manifest — already done

The section said "the disclosure exists and nothing enforces it". Not so:
`api/secret.rs` refuses an undeclared name with `not declared in this plugin's `secrets = { … }``,
and has for as long as the manifest has had the field. There was nothing to add.

### Frozen tables — no

`table.freeze` exists and works. It cannot do the job here, for three reasons that compound:

1. **There is no unfreeze.** luna's Lua binding only ever passes `true`, so freezing `oslo` is for
   the rest of the session. Plugins load *lazily*, on first use of a name they declared — so the
   freeze would land mid-session and the config's own later writes would start failing.
2. **One VM, one `oslo`.** A plugin does not get a copy; freezing for a plugin is freezing for the
   config. Doing this properly means a `Lua` per plugin, or an environment built for one.
3. **It would not be a boundary anyway.** `docs/features/plugins.md` already argues this at length,
   and the argument is right: a plugin that wants your token can run `oslo secret get`. Locking the
   table while leaving `sh` and `oslo.run` open is a lock on the door of a room with no walls.

The one thing freezing would genuinely buy — a plugin cannot *accidentally* clobber `oslo.run` for
everything else — is worth having, and is worth having at the price of a per-plugin environment
rather than at the price of freezing the config's own API. That is a bigger change than this
section, and it belongs with the plugin system rather than with the Lua surface.

## 5. Binary, and text that is text — **done**

```lua
local png = oslo.fs.read("logo.png")   -- the file, exactly
oslo.fs.write("copy.png", png)          -- byte for byte
string.unpack("<i4", db:get("row"))     -- and through the shell and back
```

The loss was never in the VM: luna's strings have always been bytes, and the boundary already went
`LunaStr::from_slice(&ctx, s.as_bytes())`. It was oslo's own `Value::Str(Rc<str>)`, which had
nothing to hand over — so `from_utf8_lossy` sat on both sides of the crossing. Three things were
silently wrong because of it: `oslo.fs.read` on a binary file, anything `string.pack` produced on
its way out, and `oslo.lines` on a command whose output had one non-UTF-8 byte, which `read_line`
rejects outright and which killed the whole loop.

### It cost three match arms, not a refactor

I first judged this at ~150 sites, on the assumption that the fix was widening `Str` to `Rc<[u8]>`.
That was the wrong shape. A second variant works, and the objection I raised against it — that Lua
has one string type, so `t["a"]` and `t[<bytes "a">]` must be the same key — is answered by an
invariant rather than by discipline:

```rust
Value::Bytes(Rc<[u8]>)   // built only by Value::bytes, which routes valid UTF-8 to Value::Str
```

**Valid and invalid UTF-8 are disjoint**, so no `Bytes` can name the same string as any `Str`. There
is nothing for a comparison or a `Key` to get wrong, and nothing to remember. `value/bytes_tests.rs`
pins it.

The whole workspace had **three** non-exhaustive matches after the variant was added: the boundary,
`oslo.json`, and `oslo config which`. Everything else has a catch-all that already means "not text",
which is the correct answer for a path, a variable name or a command word — so `text()` refuses
bytes and a new `raw()` accepts them, for the calls that write content rather than read a name.

`oslo.json.encode` refuses bytes: no JSON string could hold them and still be the same bytes.

### The second half was a misreading

`oslo.ui.width` is the *terminal's* width, not a string's, and display width is already computed in
Rust with `unicode-width` — which handles east-asian wide and combining characters that `utf8.len`
would count wrong. There was nothing for `utf8.len` to fix there.

## 6. Smaller things the VM unlocked

* **`os.date` exists**, so a prompt can show a clock without shelling out. Worth a note in the docs
  rather than an API.
* ~~**`debug.traceback`** means a failing hook could report *where* it failed.~~ It already does. A
  handler that raises reports `…/init.lua:2: could not index into a nil value` — file and line. A
  traceback would add the call chain above that line, which is worth less than the line itself. What
  those messages *do* need is tidying: the chunk name is printed twice, and a `pre-cmd` handler is
  announced as `key hook`.
* **Weak tables** make a completion cache that does not pin memory. No cache in the shell has been
  measured as a memory problem, so this is a solution looking for one.
* **The serde bridge** could give `oslo.toml.decode` and `oslo.yaml.decode` for the price of a
  dependency each, rather than a hand-written parser — the same way `oslo.json` works today.

---

## Order I would do them in

1. ~~**Handles as objects** (§1)~~ — **done**: `api/handle.rs`, and the four call sites use it.
2. ~~**Streaming producers** (§2)~~ — **done** for `oslo.fs.walk`, `oslo.fs.lines` and
   `oslo.lines`; the structured pipeline needs an `oslo-shell` change first.
3. ~~**Structured errors** (§3)~~ — **done**: `api/problem.rs`, `oslo.fs` and `oslo.db`.
4. ~~**Plugin sandbox** (§4)~~ — the memory ceiling is **done**; the secret filtering already
   existed; frozen tables cannot deliver a boundary without a per-plugin environment.
5. ~~**Binary-safe reads** (§5)~~ — **done**: a disjoint `Value::Bytes`, three match arms.

## What I would not do

* **Expose `luna` itself.** The boundary is the thing that let the engine be swapped in one crate;
  a config reaching VM internals would weld oslo to this VM.
* **A general `oslo.async`.** Coroutines are the right primitive for *iteration*; a scheduler is a
  different project and a shell has no event loop to hang it on.
* **`oslo.time`.** `os.date`, `os.time` and `os.clock` are all there and are what people know.
