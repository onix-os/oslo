-- Events a plugin names itself: `oslo.on.emit` and `oslo.on.user`.
--
-- Every other hook is a name oslo chose. This is the one two plugins can agree on without either
-- knowing the other exists — the same thing neovim gets from its single `User` event.

-- "Plugin A": it does something and says so.
local function save(key)
  local heard = oslo.on.emit("notes:saved", { key = key })
  print("emitted, heard by " .. heard)
end

-- "Plugin B": it cares, and knows nothing about A.
oslo.on.user("notes:saved", function(data, name)
  print("B saw " .. name .. " key=" .. tostring(data.key))
end)

-- "Plugin C": also cares. Both run, in the order they attached.
local handle = oslo.on.user("notes:saved", function()
  print("C too")
end)

save("first")

-- A handler can be taken off, like any other hook's.
handle:remove()
save("second")

-- Nobody is listening to this one, and that is not an error.
print("unheard: " .. oslo.on.emit("nobody:listening"))

-- A handler that raises is reported on stderr; the others still run.
oslo.on.user("notes:saved", function() error("handler broke") end)
oslo.on.user("notes:saved", function() print("still ran") end)
save("third")

-- A name that is not an event name is refused, because the failure it prevents is silent: a typo
-- subscribes to something nothing will ever emit.
print("bad name: " .. tostring(pcall(oslo.on.user, "two words", function() end)))
print("bad emit: " .. tostring(pcall(oslo.on.emit, "")))

--[[ expect
B saw notes:saved key=first
C too
emitted, heard by 2
B saw notes:saved key=second
emitted, heard by 1
unheard: 0
B saw notes:saved key=third
still ran
emitted, heard by 2
bad name: false
bad emit: false
]]
-- stderr: yes
