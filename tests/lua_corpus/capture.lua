-- `oslo.capture` is what makes a command's *answer* usable from Lua.
-- The command is *shell*, so the arguments are field-split before `echo` sees them:
-- two words in, one space out. Capturing does not change that.
local r = oslo.capture("echo  spaced  out ")
print("out=[" .. r.out .. "]")
print("status=" .. r.status)

-- Trailing newlines go, as with `$(cmd)`; interior ones stay.
print("multiline=[" .. oslo.capture("printf 'a\\nb\\n'").out .. "]")
print("empty=[" .. oslo.capture("true").out .. "]")

-- A failing command reports its own status, not the shell's.
print("failing=" .. oslo.capture("exit 7").status)
print("notfound=" .. oslo.capture("no_such_command_here 2>/dev/null").status)

-- There is no `err` field on purpose: stderr stays on the shell's own, so fold it the shell way.
print("has err field: " .. tostring(r.err ~= nil))
print("folded=[" .. oslo.capture("{ echo diag >&2; } 2>&1").out .. "]")
--[[ expect
out=[spaced out]
status=0
multiline=[a
b]
empty=[]
failing=7
notfound=127
has err field: false
folded=[diag]
]]
