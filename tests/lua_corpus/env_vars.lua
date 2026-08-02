-- Variables in both directions, and the environment as something you can iterate.
oslo.env.set("OSLO_CORPUS_V", "one")
print("get=" .. oslo.env.get("OSLO_CORPUS_V"))
print("in env=" .. tostring(oslo.env.all()["OSLO_CORPUS_V"] ~= nil))

-- A variable set from Lua is exported, so a child command sees it.
print("child sees=" .. oslo.proc.capture("printf '%s' \"$OSLO_CORPUS_V\"").out)

oslo.env.unset("OSLO_CORPUS_V")
print("after unset=" .. tostring(oslo.env.get("OSLO_CORPUS_V")))
print("gone from env=" .. tostring(oslo.env.all()["OSLO_CORPUS_V"] == nil))

-- The environment is a real table: countable, and PATH is in it.
local n = 0
for _ in pairs(oslo.env.all()) do n = n + 1 end
print("iterable=" .. tostring(n > 0))
print("has PATH=" .. tostring(oslo.env.all()["PATH"] ~= nil))
--[[ expect
get=one
in env=true
child sees=one
after unset=nil
gone from env=true
iterable=true
has PATH=true
]]
