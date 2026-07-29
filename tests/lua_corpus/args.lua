-- `arg` and `...`, the thing whose absence made Lua a configuration language.
-- The harness always passes `first-arg` and `second arg` — the second has a space in it, because
-- an argv that survives quoting is the whole point.
print("script=" .. arg[0]:match("[^/]+$"))
print("count=" .. #arg)
print("1=" .. arg[1])
print("2=" .. arg[2])
print("varargs=" .. select("#", ...))
print("interpreter is set: " .. tostring(arg[-1] ~= nil))
print("past the end is nil: " .. tostring(arg[3] == nil))
--[[ expect
script=args.lua
count=2
1=first-arg
2=second arg
varargs=2
interpreter is set: true
past the end is nil: true
]]
