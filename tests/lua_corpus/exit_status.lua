-- `oslo.exit` decides what the shell exits with, from any depth.
-- status: 42
local function inner() oslo.exit(42) end
local function outer() inner() end
print("before")
outer()
print("NOTREACHED")
--[[ expect
before
]]
