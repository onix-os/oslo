# Timers

`oslo.after` and `oslo.every` are the only things in oslo that mean "later".

```lua
oslo.after(500, function() print("half a second on") end)

local tick = oslo.every(30000, function() check_something() end)
tick:stop()
```

Both take a delay in **milliseconds** and a function, and both answer with a handle whose only
method is `:stop()`. Stopping a timer that has already gone returns `false`; stopping a live one
returns `true`.

A handle is an object: it has no keys of its own, writing a field on it is refused, and
`local t <close> = oslo.every(…)` stops the timer at the end of the block. Dropping the handle does
not stop the timer — which is the useful default, since `oslo.every(…)` is normally written for its
effect and its handle thrown away.

<!-- demo:begin -->
[![timers demo](https://asciinema.org/a/1263432.svg)](https://asciinema.org/a/1263432)
<!-- demo:end -->

## When they actually fire

**Between commands, never during one, and never while you are typing.** The read loop checks what
is due at the top of each iteration and again after each command finishes. Those are the two moments
the shell is holding nothing and can safely call into Lua, and they are already where deferred hooks
drain.

**And at an idle prompt**, which is the third such moment. A prompt sitting untouched holds nothing
either, and it is where a timer set for "in five minutes" would otherwise have waited for the next
keystroke to be noticed — the one moment its author did not mean. The editor's wait is given the
nearest deadline instead of "for ever", so the wake happens when the timer says rather than when the
terminal does.

So this is the part worth reading twice:

```lua
oslo.every(1500, function() print("tick") end)
```

at an idle prompt **does** tick — four times over seven untouched seconds, measured. That is what
the idle wake buys: the editor's read is given the nearest deadline rather than "for ever", so a
timer set for five minutes fires in five minutes instead of waiting for the next keystroke to be
noticed.

What it still is not is a clock. A tick lands at one of the three moments the shell is holding
nothing — top of the read loop, after a command, or an idle wake — so one that comes due *while a
command is running* waits for it to finish, and a long command absorbs every tick that fell inside
it rather than firing them in a burst afterwards. The promise is "close to when you asked, never in
the middle of something", not "on the second".

The alternative is a real event loop — neovim has one, and `vim.uv` timers fire whenever it turns.
That means libuv or `tokio` inside a shell that deliberately removed `tokio`, and every Lua callback
becoming a thing that can run in the middle of an expansion. The trade taken here is the honest one:
a smaller promise, kept exactly.

## The edges

| | |
|---|---|
| delay of `0` | `oslo.after(0, f)` runs `f` at the next safe point |
| repeat of `0` | raised to 1 ms — a repeating timer of zero is due for ever and the loop would never move on |
| a delay over a day | capped at 24 hours |
| a negative, `nan` or `inf` delay | an error, naming the value |
| a second argument that is not a function | an error |
| a repeating handler that raises | reported, and the timer is stopped rather than left to raise on every command |

## What they pair with

`oslo.spawn` runs something in the background and calls you back at these same safe points, so a
timer and a spawn callback arrive by the same rule and neither can land mid-command.

```lua
oslo.every(60000, function()
  oslo.spawn{ "git", "fetch", "--quiet",
    on_exit = function(out, status) oslo.state.set("git.fetched", status == 0) end }
end)
```

Nothing has to ask for a redraw afterwards. A prompt whose inputs have changed is drawn again on its
own, which is why there is no `oslo.ui.redraw` to call.

## Waiting, where there is no prompt to wait at

A safe point needs something to arrive at, and a script or a `.make.lua` never reaches one — so a
spawn there used to queue its result and have nobody ever look. Two calls block until it does:

```lua
local job = oslo.spawn{ "cargo", "build" }
local out, status = job:wait(60000)     --> out, status — or nil, why

for _, crate in ipairs(crates) do oslo.spawn{ "cargo", "build", "-p", crate } end
oslo.settle{ timeout_ms = 600000 }      --> { fired = 4, outstanding = 0, settled = true }
```

Both wait on the same descriptors the line editor polls, so they return the instant a worker
finishes. `on_exit` still fires for a job that was also waited on — "always" would be a poor promise
otherwise. Waiting on one that was cancelled, or whose callback already ran, answers `nil` and says
which rather than blocking for a result that is not coming.

**Interactively you want neither.** At a prompt the callback arrives on its own, and blocking the
shell to wait for it is the thing `oslo.spawn` exists to avoid.

See [hooks](hooks.md) for the other way to have code run without typing it, and
[the Lua interpreter](lua-interpreter.md) for what the language itself is.
