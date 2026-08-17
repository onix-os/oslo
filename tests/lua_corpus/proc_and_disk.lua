-- What a process is and what a filesystem has, as fields rather than as somebody's output.

-- This shell, which is the one process a case can be certain of the answers for.
local me = oslo.proc.info(oslo.proc.pid())
print("pid=" .. tostring(me.pid == oslo.proc.pid()))
print("ppid=" .. tostring(me.ppid == oslo.proc.ppid()))
print("threads=" .. tostring(me.threads >= 1))
print("rss=" .. tostring(me.rss > 0))
print("state=" .. tostring(type(me.state) == "string" and not me.state:match("^%u$")))

-- `argv` is the arguments as they were passed, so one holding a space stays one argument. That is
-- the whole reason this is read from `/proc` rather than out of `ps`.
print("argv=" .. tostring(#me.argv >= 1))
print("exe=" .. tostring(me.exe ~= nil))

-- A pid that cannot be live is a message, never a raise: a process stops existing between when you
-- read its id and when you ask about it.
print("gone=" .. tostring(oslo.proc.info(9999999)))

-- And something that is not a pid at all is the caller's mistake.
print("refused=" .. tostring(not pcall(oslo.proc.info, "init")))

-- Children are found by scanning, because the kernel records only the edge upwards.
print("children=" .. tostring(type(oslo.proc.children(1)) == "table"))

-- A filesystem, in bytes. `available` is what you may write; `free` is what exists, and the gap is
-- the reserve a filesystem keeps for root.
local d = oslo.fs.disk(".")
print("total=" .. tostring(d.total > 1024 * 1024))
print("order=" .. tostring(d.available <= d.free and d.free <= d.total))
print("inodes=" .. tostring(d.files_free <= d.files))
print("nowhere=" .. tostring(oslo.fs.disk("/nowhere/at/all")))

--[[ expect
pid=true
ppid=true
threads=true
rss=true
state=true
argv=true
exe=true
gone=nil
refused=true
children=true
total=true
order=true
inodes=true
nowhere=nil
]]
