-- `oslo.proc.capture` is what makes a command's *answer* usable from Lua.
-- The command is *shell*, so the arguments are field-split before `echo` sees them:
-- two words in, one space out. Capturing does not change that.
local r = oslo.proc.capture("echo  spaced  out ")
print("out=[" .. r.out .. "]")
print("status=" .. r.status)

-- Trailing newlines go, as with `$(cmd)`; interior ones stay.
print("multiline=[" .. oslo.proc.capture("printf 'a\\nb\\n'").out .. "]")
print("empty=[" .. oslo.proc.capture("true").out .. "]")

-- A failing command reports its own status, not the shell's.
print("failing=" .. oslo.proc.capture("exit 7").status)
print("notfound=" .. oslo.proc.capture("no_such_command_here 2>/dev/null").status)

-- There is no `err` field on purpose: stderr stays on the shell's own, so fold it the shell way.
print("has err field: " .. tostring(r.err ~= nil))
print("folded=[" .. oslo.proc.capture("{ echo diag >&2; } 2>&1").out .. "]")
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
