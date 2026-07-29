-- A Lua error is caught and reported, and the shell exits 1 rather than pretending success.
-- The traceback naming the *file* is why chunks are loaded under their path.
-- status: 1
-- stderr: yes
print("before the error")
local ok, message = pcall(function() error("handled") end)
print("pcall caught=" .. tostring(not ok) .. " message ends=" .. message:match("[^:]+$"))

-- An unhandled error ends the script here; nothing after it runs.
error("unhandled")
--[[ expect
before the error
pcall caught=true message ends= handled
]]
