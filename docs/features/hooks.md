# Hooks

Twenty-two moments in a shell's life that a config can attach to, from `pre-cmd` down to every
keystroke. They exist so that a prompt integration, a `direnv` clone or a package-manager handler is
a function in your config rather than a shell function that has to be re-sourced into every session.

<!-- demo:begin -->
[![hooks demo](https://asciinema.org/a/1262739.svg)](https://asciinema.org/a/1262739)
<!-- demo:end -->

## How it works

`oslo.on.<name>(f)` adds `f` to a list and answers with a handle. **Handlers accumulate; they never
replace.** Two plugins both attaching to `pre-cmd` both fire, in the order they attached, and
`handle:remove()` takes one off without disturbing the others — a removed handler is overwritten
with `false` in the list rather than deleted, so every other handle's position stays valid.

A hook is either **told** something or **asked** something, and which one is fixed by the `answers`
field of its row in `HOOKS`. A hook that is told runs every handler and discards what they return. A
hook that is asked runs handlers until one returns a non-nil value, and that value is the answer;
**the remaining handlers are not called at all.** That short-circuit is the right rule for an
interception chain — the handler that decided what a key means has ended the question.

Five of the six answering hooks work that way, through `answer_hook_with`. `on-command-not-found`
is the exception: it goes through `ask_hook_on`, which reads the first return value only when it is
a **number**. A handler there that returns a string, `true` or a table is passed over and the next
handler is still asked.

### The registry, and why it is in `oslo-base`

The line editor, the executor and the `cd` builtin all fire hooks. They used to do it by calling
`lua::engine::fire_at_here` directly, which pinned the whole Lua layer *underneath* them — but the
same Lua layer holds `lua::api`, which has to sit *above* them, because it is how a config reaches
the shell. One module cannot be on both sides of everything else, and that is what forced the
inversion when the crate was split up.

```
   oslo-runtime   LuaEngine::new()
                    └─ oslo_base::hooks::install(Dispatch { watched, fire, answer, ask })
                                    │  four fn pointers, once, into a OnceLock
                                    ▼
        ┌─────────────────────────────────────────────────────┐
        │ oslo-base::hooks                                    │
        │   at::PRE_CMD … at::ON_KEY   the moments, by index   │
        │   watched / fire_at_here / answer_hook_with / ask    │
        │   knows nothing about Lua beyond oslo_lua::Value    │
        └─────────────────────────────────────────────────────┘
             ▲                    ▲                    ▲
        oslo-ui              oslo-shell           oslo-runtime
        editor, vi mode,     cd, exec,            repl loop,
        idle, on-report      jobs, time           history, completion
```

Before `install` runs — a script, `sh -c`, a test that never starts an interpreter — `watched`
answers `false` and every fire is one relaxed load and a return.

**Moments are named by index, not by name, and that too was forced by a bug.** The one fire site
that asked for its hook by string asked for `"command-not-found"`, which is an *alias*; handlers are
stored under the canonical name, so the lookup found an empty list and `on-command-not-found` never
fired under either spelling.

### Fire, ask, or defer

```
fire site names a moment by index
  │
  ├─ watched(index)? ──── no ──► return              the entire cost of an unused hook
  │        yes
  ├─ HOOKS[index].answers = true
  │        └─ inline, always. Handlers in attachment order; first non-nil value wins,
  │           the rest are skipped. The shell's state may be locked here, so an
  │           oslo.* call that reaches the shell raises rather than hangs.
  │
  └─ HOOKS[index].answers = false
           └─ state_is_held()?          (one uncontended try_lock on the Environment)
                 yes ──► queue::defer(index, args)   held, oldest first
                 no  ──► fire_now: every handler, return values discarded
```

The deferred queue exists because half the fire sites are inside something holding the shell's
`Environment`. `post-change-dir` fires from `attempt_directory`, which can only be reached through a
`&mut Environment` — so a handler calling `oslo.env.set` met `borrow_env`'s deliberate `try_lock`
failure and did nothing, every time, while reporting a `oslo.register_builtin` the config had never
used. Deferring keeps both halves: the fire site stays where it is accurate (it catches a `cd`
inside a function, which comparing directories across a command line cannot), and the handler runs a
moment later against a shell it can actually change.

The queue is drained at five places:

| drain point | why |
|---|---|
| entry to `borrow_env` | any `oslo.*` call that touches the shell picks up what an earlier one left |
| drop of `EnvBorrow` | the guard is released **before** the drain, or a handler taking the same lock deadlocks |
| the REPL loop, after a command | the first moment in a command's life when nothing is locked |
| the end of a Lua chunk, in `LuaEngine::run` | a script has no loop to drain into, so `oslo.run{"cd", d}` at the end of one would leave its `post-change-dir` queued for a REPL that never comes |
| `fire_exit` | a `cd` in the command that ended the shell would otherwise queue a hook nothing drains |

Draining is a no-op when the state is still held or when a drain is already running further up the
stack — a handler that itself runs `cd` re-enters here, and without that guard it would recurse. The
queue is taken whole rather than popped, so work queued by a handler waits for the next drain
instead of extending the walk it is inside.

### The moments

Fields marked *(strings)* are strings even when they read as numbers: a notifying hook fired through
`fire_at_here` carries `(name, value)` pairs of `&str`, and nothing converts them on the way in.

| hook | fired from | handed | answer |
|---|---|---|---|
| `pre-cmd` | the REPL, before the line runs | `{ text, cwd, mode }`, plus `commands` when the line is shell and parses | string replaces, `false` cancels |
| `post-cmd` | the REPL, after | `{ text, cwd, mode, status, ok, duration_ms }` | — |
| `pre-change-dir` | `cd`, move resolved and not yet made | `{ from, to }` | `false` refuses the move |
| `post-change-dir` | every `cd`, `pushd`, `popd` and jump | `{ from, to }` | — |
| `pre-prompt` | the REPL, before drawing | nothing | — |
| `post-prompt` | the editor, prompt on screen | an empty table | — |
| `pre-mode-change` | vi mode, and the language switch | `{ kind = "vi"\|"language", from, to }` | — |
| `post-mode-change` | the same two | the same | — |
| `on-history-open` | the finder, once it will really open | `{ seed }` | — |
| `on-history-select` | a line chosen | `{ line }` | — |
| `on-history-close` | either ending | `{ chosen }` — `"true"` / `"false"` | — |
| `on-completion-start` | Tab, only with candidates | `{ word, line, count }` *(strings)* | — |
| `on-completion-cancel` | the menu declined | `{ word }` | — |
| `on-completion-select` | a candidate taken | `{ value, word }` | — |
| `on-job-finish` | the job reaper, ended jobs only | `{ id, pid, text, status }` *(strings)* | — |
| `on-process-exit` | the job reaper, one per process | `{ pid, job, status }` *(strings)* | — |
| `on-job-state` | the job reaper, on a transition | `{ id, pid, text, from, to, background }` *(strings)* | — |
| `on-time-report` | a `time`-prefixed pipeline | `{ real_ms, user_ms, sys_ms }` *(strings)* | — |
| `on-command-not-found` | the end of the command search | the command name, a bare string | a number is the status, and means handled |
| `on-idle-timeout` | the editor's timed read | `{ seconds }` *(string)* | — |
| `on-report` | five reporters | `{ kind, … }`, `kind` ∈ direnv, slow, chain, job, time | exactly `true` means "I drew it" |
| `pre-record` | the tracker, line finished | `{ text, cwd, mode, status, duration_ms, profile, segments }` | see below |
| `on-exit` | both ways a REPL ends, before the EXIT trap | `{ status }` | — |
| `on-key` | every keystroke, before any binding | `{ name, char, text, cursor, word, word_start }` | `false` swallows; string or `{ text = … }` replaces |
| `on-secret-encrypt` | a store whose config says `crypto hook` | `(store, name, base64)` *(three arguments)* | base64 of the sealed bytes; **nil is "not mine"** |
| `on-secret-decrypt` | the same, reading | `(store, name, base64)` | base64 of the value; nil declines |
| `pre-secret-access` | any secret read or written | `{ store, name, how }`, `how` ∈ read, write | — |
| `post-secret-access` | the same, afterwards | `{ store, name, how }` | — |

The four secret hooks are the only ones that exist to *replace* a mechanism rather than to watch or
veto one, and they carry two rules of their own. **`nil` means "not mine"** — the next handler is
asked, so several plugins can each claim their own store, and a store nobody claims is a refusal
rather than a fall back to age. And they are only reachable in a process that ran your config, which
is why the store's configuration file has to say `crypto hook` out loud: `oslo secret get` under
`cron` then fails naming the reason instead of quietly doing something else. See
[secrets](secrets.md#pluggable-hooks-do-the-crypto-lua-does-the-storage).

**The two watching ones are never given the value.** A hook that logs is the likeliest thing anybody
writes on them, and a log of secrets is worse than no log.

`pre-cmd`'s contract in full, because it is the one with three outcomes: **a string replaces the
line that runs, `false` cancels it, and nil leaves it alone.** A cancelled line reports status 130,
which is what a line abandoned at the prompt already reports, so nothing downstream needed a new
case. Anything else a handler returns is treated as nil by the caller — but it still counts as an
answer, so it stops the later handlers being asked.

`pre-record` decides what is written down: nil records the line as typed, `false` records nothing at
all, a string records that one line, and a list of strings records each of them. An *empty* list is
not a refusal — a handler that built a list and matched nothing meant "no change", and reading it as
"forget this line" would lose commands to a rule that did not apply.

An error raised by a handler is printed to stderr and the next handler is asked. A broken plugin
must not silence a job notice, turn a missing command into a success, or stop the other handlers.

### The three job hooks, and which one to want

`on-job-finish` is one per **job**; `on-process-exit` is one per **process**. A pipeline of three
stages is three of the latter and one of the former, which is the difference a plugin watching one
particular child needs. `on-job-state` is the transition — `from` and `to`, both of `running`,
`stopped` and `ended` — because "it stopped" and "it was already stopped" are different things to a
status line.

**They fire after the job table's lock is released, not while it is held.** A handler is entitled to
call `oslo.job.list()`, which takes that same lock; firing from inside the reaper would be a handler
waiting for a lock its own caller holds. So the reaper records what happened, drops the lock, and
then announces — which is also why the payload is a snapshot of strings rather than a live handle.

**They fire at an idle prompt too, not only at a command boundary.** `SIGCHLD` is installed without
`SA_RESTART`, so a child ending makes the editor's blocked `read` fail with `EINTR`; the reader
services the background and repaints before going back to waiting. That is the same route `SIGWINCH`
has always taken for a resize, rather than a second mechanism — the editor has a blocking read, not
an event loop, and the interrupt is already load-bearing.

Only an interactive shell arms it. A script reaps at its command boundaries and has no editor to
wake, so the signal would buy it nothing and cost it an interrupted `read` in every library call
that makes one.

## What makes it different

Every hook here is a list from the start. `oslo.on` is the only way to attach, two plugins
attaching to the same moment both fire, and there is no single-owner spelling — no variable holding
one function, no trap to overwrite — for a second plugin to clobber.

The deferral is the price of handlers written in another language. oslo's are Lua and reach the
shell through a lock, which is a real boundary — and the answer is not to weaken the lock but to
move the handler to a moment when it is free.

## Configuration

```lua
oslo.on.pre_cmd(function(c)
  if c.text:match("^rm %-rf /%s*$") then return false end   -- cancel
  return nil                                                -- leave it alone
end)

local h = oslo.on.post_change_dir(function(d) oslo.env.set("LAST_DIR", d.from) end)
h:remove()

oslo.misc.idle_timeout = 300        -- seconds; on-idle-timeout does nothing without it
```

Every canonical name is installed three ways where it can be: kebab-case (`oslo.on["pre-cmd"]`),
underscored (`oslo.on.pre_cmd`), and once per alias. The aliases are the spellings oslo shipped
first — `preexec`, `precmd`, `postexec`, `postcmd`, `prompt`, `cd`, `command-not-found`, `key` — and
each fires on the same list as its canonical name, so no config breaks. `cd` is an alias of
**`post-change-dir`**, since it always fired after the move.

Aliases are installed exactly as written, so an alias containing a dash needs bracket syntax:
`oslo.on["command-not-found"]` works and `oslo.on.command_not_found` is nil. The underscored field
spelling of that hook is `oslo.on.on_command_not_found`.

## What it cannot do

- **Add a moment.** The list is fixed in `HOOKS`. A name nothing fires is indistinguishable from a
  typo, and `oslo.on.precmb(f)` doing nothing for ever is what a fixed list avoids — an unknown
  spelling is simply not a field on `oslo.on`.
- **Grow past 32 moments** without widening the watched bitset from a `u32`; a test asserts it.
- **Forget that a hook was ever used.** The watched bit is never cleared, so removing the last
  handler leaves the bit set and the fire site then walks an empty list. Clearing it correctly would
  mean counting handlers across config reloads, and a wrong count there silently kills the hook.
- **Let a hook that fires from a held state change the shell.** `pre-change-dir`,
  `on-command-not-found` and the `chain`, `job` and `time` kinds of `on-report` run inline while the
  `Environment` is locked, and an `oslo.*` call that reaches the shell raises there rather than
  hanging. Everything they could want is passed in as an argument for that reason. Drawing —
  `oslo.ui.block` — always works. The other answering hooks (`pre-cmd`, `pre-record`, `on-key`) fire
  from moments where the state is free and may use the whole API.
- **Order handlers.** Attachment order is the only order. There is no priority and no way for one
  handler to run before another attached earlier.
- **Answer partially.** For the five hooks dispatched through `answer_hook_with`, the first non-nil
  return value ends the question, even when it is a value the caller does not understand. A handler
  that returns `true` from `pre-cmd` by accident silences every handler after it.
- **Reach a script.** A config is only read by an interactive shell, so nothing here changes what
  `sh -c` or a `#!/bin/sh` file does.
- **Run anywhere but the shell's own thread.** Handlers run against the interpreter parked on that
  thread; there is no other thread that could drain the queue.

## Where it lives

| path | what |
|---|---|
| `crates/oslo-base/src/hooks.rs` | `at::*`, `Dispatch`, `install`, `watched`, `fire_at_here`, `answer_hook_with`, `ask_hook_here`, `fields` |
| `crates/oslo-runtime/src/lua/api/hooks.rs` | `HOOKS`, `Hook.answers`, the aliases, `resolve`, `spellings`, the watched bitset |
| `crates/oslo-runtime/src/lua/api/shell.rs` | `oslo.on` itself: `hooks`, `append`, `handle`, `handlers` |
| `crates/oslo-runtime/src/lua/engine/hooks.rs` | `fire_or_defer`, `fire_now`, `answer_hook_with`, `ask_hook_on`, `key_hook_here` |
| `crates/oslo-runtime/src/lua/engine/queue.rs` | `PENDING`, `DRAINING`, `defer`, `drain` |
| `crates/oslo-runtime/src/lua/engine/borrow.rs` | `borrow_env`, `EnvBorrow` — the two drain points that are not the REPL |
| `crates/oslo-runtime/src/lua/engine.rs` | `state_is_held`, `install`, `command_started`, `command_finished`, `hook_fields` |
| `crates/oslo-runtime/src/lua/parsed.rs` | `commands_of` — `pre-cmd`'s `commands` field |
| `crates/oslo-runtime/src/startup/tracking.rs` | `pre-record`, `Recording`, `lines_to_record` |
| `crates/oslo-ui/src/report.rs` | `on-report`: `handled`, and which kinds fire from a held state |
| `crates/oslo-ui/src/editor.rs` | `key_table`, `key_outcome_from`, `answer_from` |
| `tests/hook_dispatch_tests.rs` | every hook has a fire site, and it uses the dispatch `answers` says it should |
| `tests/lua_corpus/hooks_may_act.lua` | a deferred `post-change-dir` that changes the shell, and a `pre-change-dir` veto |
