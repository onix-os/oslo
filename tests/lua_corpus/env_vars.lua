-- Variables in both directions, and the environment as something you can iterate.
oslo.set_var("OSLO_CORPUS_V", "one")
print("get=" .. oslo.get_var("OSLO_CORPUS_V"))
print("in env=" .. tostring(oslo.env()["OSLO_CORPUS_V"] ~= nil))

-- A variable set from Lua is exported, so a child command sees it.
print("child sees=" .. oslo.capture("printf '%s' \"$OSLO_CORPUS_V\"").out)

oslo.unset("OSLO_CORPUS_V")
print("after unset=" .. tostring(oslo.get_var("OSLO_CORPUS_V")))
print("gone from env=" .. tostring(oslo.env()["OSLO_CORPUS_V"] == nil))

-- The environment is a real table: countable, and PATH is in it.
local n = 0
for _ in pairs(oslo.env()) do n = n + 1 end
print("iterable=" .. tostring(n > 0))
print("has PATH=" .. tostring(oslo.env()["PATH"] ~= nil))
--[[ expect
get=one
in env=true
child sees=one
after unset=nil
gone from env=true
iterable=true
has PATH=true
]]
