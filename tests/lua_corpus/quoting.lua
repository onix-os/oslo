-- Text crossing the Lua/shell boundary keeps its bytes: what Lua sends is what the shell runs.
local tricky = "a b'c\"d$e"
oslo.env.set("OSLO_CORPUS_Q", tricky)
print("round trip=" .. tostring(oslo.env.get("OSLO_CORPUS_Q") == tricky))

-- A command built from data needs quoting, which is what `printf %q` is for; without it the
-- `$e` and the quotes would be read by the shell as syntax.
local quoted = oslo.proc.capture("printf '%q' \"$OSLO_CORPUS_Q\"").out
print("captured back=" .. tostring(oslo.proc.capture("printf '%s' " .. quoted).out == tricky))

-- Interior newlines survive a capture; only trailing ones are stripped.
print("newlines=[" .. oslo.proc.capture("printf 'x\\n\\ny\\n\\n\\n'").out .. "]")
--[[ expect
round trip=true
captured back=true
newlines=[x

y]
]]
