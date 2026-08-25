# The control socket

Another program asking a running shell a question, in Lua.

```lua
-- in hexe, pixy, or any Lua VM at all
local src  = io.popen("oslo lua-api"):read("a")
local oslo = load(src)(transport)

local sh = oslo.connect()
print(sh.env.get("PATH"))       -- the shell's live $PATH, not a snapshot from exec
```

**Nothing serves until it is asked to.** A shell binds no socket, holds no copy of its environment
and opens no descriptor unless its config or a keypress says so. The client half costs nothing
either way, so a shell can *ask* another tool things while serving nothing itself — which is the
common direction and the cheap one.

```lua
oslo.live.serve()                                     -- in init.lua, for a shell that should answer
oslo.keys["ctrl-g"] = function() oslo.live.serve() end -- or on a key, when you decide
```

<!-- demo:begin -->
[![control-socket demo](https://asciinema.org/a/1263749.svg)](https://asciinema.org/a/1263749)
<!-- demo:end -->

## Why not just read `/proc/<pid>/environ`

Because it is wrong, and quietly. That file is the environment the process was **exec'd** with: it
does not have what a directory environment loaded, what the config exported, or what you typed two
commands ago. A session manager that copies it into a new pane hands over a shell's environment as
it was at birth.

This is the same fact, asked of the shell that actually has it.

## Three layers, and only the bottom one is oslo's

| | |
|---|---|
| `oslo.stream` | the socket, as a Lua handle. A host native, ~150 lines |
| `oslo lua-api` | the client library, **plain Lua**, printed on stdout |
| `oslo.live` | the server: binds on demand, answers the verbs below |

The middle layer is the point. It is a plain Lua file with no dependencies, so it runs unchanged in
oslo's own VM, in ziglua, in PUC Lua and in whatever the next sibling embeds — **copied between
tools rather than ported.** It carries its own JSON codec for exactly that reason: a client that
needed the host to have one is a client most hosts could not load.

It is handed out by a *command* rather than installed at a path, because everything already knows
how to run `oslo` — it was reached through `$PATH` by definition — while nobody knows where a distro
put its share directory. That also versions the two together: there is no way to load a stub from
one oslo and speak to another.

```lua
load(src)(transport)     -- transport.connect(path, timeout_ms) -> handle
                         -- handle:send(bytes) / handle:recv(n) / handle:close()
```

Inside oslo the transport is `oslo.stream` and is found automatically. Elsewhere, pass the host's
own — that one function is the whole porting job.

## The verbs

`oslo lua-api --verbs`, and this is all of them:

| verb | answers |
|---|---|
| `cwd` | the directory the shell is in |
| `session` | this shell's session id |
| `verbs` | this list |
| `env.get` | one variable, exactly as the shell has it |
| `env.all` | every exported variable, as a record |
| `env.set` | set one variable in the running shell |
| `macros.get` | the body of one stored macro |
| `notify` | put a line in the shell's message log |
| `cd` | ask the shell to move; applied at its next safe point |

**`cd` is asked for, not done.** Every other verb answers on the server thread and is finished when
it replies. This one cannot: `set_current_dir` is process-wide, so a server thread making the move
would shift the ground under a command part-way through resolving a path for `exec` — a command
that runs in the wrong directory, occasionally. So the request is recorded, the shell is woken down
a pipe its input wait already polls, and the shell makes the move on its own thread between
keystrokes, holding nothing.

The reply therefore means **accepted**. A shell at a prompt gets there in a millisecond; one running
a build gets there when the build ends. Read `cwd` back if you need to know it happened.

It goes through `cd` itself, not through a second route: `$PWD` and `$OLDPWD` move with it, the
directory ring records it, and `post-change-dir` fires — so a hook cannot tell a peer's move from a
typed one. The prompt is rebuilt and redrawn where it stands, with **no keystroke**, through the
same door an asynchronous prompt already comes through.

What it does *not* do is run the new directory's `.env.lua`. That is a prompt-boundary concern on
purpose — arriving mid-line would change `$PATH` under a completion already in flight — so a peer's
move is picked up by the same check that catches every other route, at the next prompt.

**There is no `run`.** A socket that executes what a caller sends is remote code execution on
somebody's session, and every later decision would be made in that shadow. Adding one is a separate
argument, not a later commit — a test asserts the absence so it cannot arrive unnoticed.

The list is short for a second reason: each entry answers something the asker **cannot answer for
itself**. That rule is what keeps the surface from becoming a mirror of `oslo.*`, which would be a
second definition of everything and would drift from the first.

## While the shell is busy

A foreground command holds the environment lock for as long as it runs — which is exactly when
somebody asks what a shell's `$PATH` is. So a serving shell keeps a copy of its exported environment,
refreshed before each prompt, and the two read verbs fall back to it:

```
idle at a prompt  →  the live environment, exact
running a build   →  the last prompt's copy, one command stale
```

Still better than `/proc/<pid>/environ`, which is stale since `exec`. `env.set` has no fallback — a
write needs the real thing — and says the shell is busy instead.

The copy exists **only while serving**. A shell that never binds a socket never holds one.

## Where it lives, and who may reach it

```
$XDG_RUNTIME_DIR/onix/<tool>/<session>.sock        the directory is 0700
```

`$XDG_RUNTIME_DIR` because it is per-user, cleared at logout, and not `/tmp`. The directory mode is
the real access control; on top of it the server reads the connecting process's uid from the kernel
(`SO_PEERCRED`) and refuses anything that is not yours. A pid or uid the *peer* sends is a claim,
and nothing here reads one.

Finding a shell, in order: `$OSLO_SOCK` (a process oslo started inherits it), then a session id, then
the newest socket in the directory. A socket **file** is not a running shell — one left by a shell
that was killed looks identical — so the client tries candidates newest-first and a failed connect
is the staleness check. It is the only one that cannot be raced.

## Bounds

| | |
|---|---|
| connections at once | 8 |
| frame | 4 MiB, refused rather than clamped |
| idle connection | 30 s |
| client connect / read / write | 5 s, overridable |
| encode depth | 24 |

The server runs on a thread of its own, and that is a **consequence of the verbs rather than a
design goal**: every one of them answers from the environment, which is `Send`, so none needs the
Lua VM. A verb that needed the VM would need the read loop — the VM is not `Send` — which means a
queue, a drain at prompt time and an answer to what happens when a request arrives mid-call. Adding
such a verb is not a small change.

## Where it lives in the tree

| path | what |
|---|---|
| `crates/oslo-base/src/wire.rs` | the socket path and the frame, shared so the two ends cannot disagree |
| `crates/oslo-runtime/src/lua/api/stream.rs` | `oslo.stream` |
| `crates/oslo-runtime/src/lua/api/client.lua` | the client library, plain Lua |
| `crates/oslo-runtime/src/lua/api/live.rs` | the verbs, the dispatch, `oslo.live` |
| `crates/oslo-runtime/src/lua/api/live/server.rs` | accepting, framing, the uid check |
| `src/cli/live.rs` | `oslo lua-api` |
