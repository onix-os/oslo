-- `oslo.spawn` — work off the prompt, and a callback when it is done.
--
-- **A callback arrives between commands, never the instant the process exits.** In a script there is
-- no read loop, so nothing delivers on its own; `oslo.proc.exec` is what runs a command and gives
-- the loop-equivalent a turn. That is the promise being demonstrated, not a limitation being worked
-- around: the same rule holds at a prompt, one command late.

local heard = {}

oslo.spawn {
  "printf",
  "one",
  on_exit = function(out, status)
    heard[#heard + 1] = "printf said " .. out .. " (" .. status .. ")"
  end,
}

-- Nothing has been delivered yet: the process may even have finished, but the shell has not reached
-- a point where it holds nothing and can call Lua.
print("immediately: " .. #heard .. " heard")

-- A command nobody can run answers 127, the status a shell already uses for it.
oslo.spawn {
  "definitely-not-a-command-anywhere",
  on_exit = function(_, status)
    heard[#heard + 1] = "missing command status " .. status
  end,
}

-- A spawn with no callback at all is allowed: something whose *effect* is the point.
oslo.spawn { "true" }

-- `cancel` forgets the callback. It does not kill the process — a different decision.
local job = oslo.spawn { "printf", "never", on_exit = function()
  heard[#heard + 1] = "this should not be heard"
end }
print("cancelled: " .. tostring(job:cancel()))

-- Give the shell somewhere to deliver. Each of these is a command boundary.
--
-- **The bound is generous on purpose.** In practice both callbacks arrive within two or three
-- iterations; a tighter bound passed when run by hand and failed under a loaded test machine, which
-- is a flaky test rather than a demonstration. The loop stops as soon as both have arrived.
for _ = 1, 500 do
  oslo.proc.exec("true")
  if #heard >= 2 then break end
end

table.sort(heard)
for _, line in ipairs(heard) do
  print(line)
end

--[[ expect
immediately: 0 heard
cancelled: true
missing command status 127
printf said one (0)
]]
