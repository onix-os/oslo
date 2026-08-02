-- The two ways to run a command differ in where the output goes, and both report a status.
-- `exec` writes through to the shell's stdout, so its text appears in order with `print`.
print("before")
local status = oslo.proc.exec("echo from-exec")
print("exec status=" .. status)
-- A failing command's status comes back. `exit` is deliberately not used here: `exec` runs in
-- *this* shell — which is the whole reason `cd` through it persists — so `oslo.proc.exec("exit 5")`
-- ends the script rather than returning 5, exactly as `exit` in a sourced file would.
print("exec failing=" .. oslo.proc.exec("false"))

-- `capture` takes the text instead, so nothing is printed until the script prints it.
local r = oslo.proc.capture("echo from-capture")
print("capture kept=[" .. r.out .. "]")

-- Shell state set by `exec` persists: it is this shell, not a subshell.
oslo.proc.exec("cd /tmp")
print("cd through exec moved us=" .. tostring(oslo.sys.pwd() == "/tmp"))
--[[ expect
before
from-exec
exec status=0
exec failing=1
capture kept=[from-capture]
cd through exec moved us=true
]]
