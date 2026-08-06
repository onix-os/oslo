-- A hook that only observes may change the shell. It could not, and said nothing about it.
--
-- `post-change-dir` fires from `attempt_directory`, which runs with the `Environment` locked —
-- there is no other way to reach it, since it takes a `&mut Environment`. So a handler calling
-- `oslo.env.set` met `borrow_env`'s deliberate `try_lock` failure, every time, and the message it
-- produced named `oslo.register_builtin` — a thing the config had never used. It read as noise.
--
-- Found by writing a classic-`direnv` hook: the same code worked run directly and did nothing at
-- all inside the hook. The config had to move to `post_cmd` to work.
--
-- The fix defers notifying hooks to the next moment the shell is idle. This case is what proves
-- it, and it is written against a *script* rather than a prompt on purpose: a script has no REPL
-- loop to drain into, so it exercises the other drain point — the one in `borrow_env` — as well.

local fired = {}

oslo.on.post_change_dir(function(d)
  fired[#fired + 1] = "post-change-dir"
  -- The whole point. Reaching the shell from here used to raise.
  oslo.env.set("MOVED_TO", d.to)
end)

-- `cd` through the argv model, which holds the lock for the whole call exactly as the REPL does
-- for a command line.
local target = oslo.fs.cwd()
oslo.run { "cd", target }

-- Run by the time the call that deferred it returns: the state is released at the end of
-- `oslo.run`, and releasing it is what drains. A script never has to know the queue is there.
print("hook ran=" .. tostring(#fired == 1))
print("hook could act=" .. tostring(oslo.env.get("MOVED_TO") == target))

-- Ordering survives the deferral: hooks run in the order they fired, not the order they drained.
local order = {}
oslo.on.post_change_dir(function() order[#order + 1] = "second" end)
oslo.run { "cd", target }
print("both handlers ran=" .. tostring(#fired == 2 and #order == 1))

-- A hook that *answers* is a different case and must stay inline: `pre-change-dir` returning
-- `false` has to be read while the move can still be refused, so deferring it would mean nothing.
-- It gets the destination as an argument rather than by asking the shell for it.
oslo.on.pre_change_dir(function(d)
  return not d.to:match("refused$")
end)
local blocked = oslo.run { "cd", "/tmp/oslo-corpus-refused" }
print("veto still works=" .. tostring(blocked.status ~= 0))

--[[ expect
hook ran=true
hook could act=true
both handlers ran=true
veto still works=true
]]
-- stderr: yes
