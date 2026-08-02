-- `oslo.sys.cd` and `oslo.glob`: the filesystem work a shell exists for, which Lua's own library
-- cannot do (it has no pathname expansion at all).
oslo.proc.exec("mkdir -p conf sub")
oslo.proc.exec("touch conf/a.conf conf/b.conf conf/notes.txt")

local hits = oslo.glob("conf/*.conf")
print("matches=" .. #hits)
print("sorted=" .. table.concat(hits, " "))

-- No match yields an empty table, never the pattern back — the shell convention would make
-- `#hits == 0` unreachable, and that is what a Lua caller naturally writes.
print("no match=" .. #oslo.glob("conf/*.nothing"))

-- cd reports Lua-style: true, or nil plus a message.
local ok = oslo.sys.cd("sub")
print("cd ok=" .. tostring(ok))
print("pwd ends=" .. oslo.sys.pwd():match("[^/]+$"))
-- The shell has to agree, or `pwd` and Lua would disagree about where the script is.
print("shell agrees=" .. tostring(oslo.sys.pwd() == oslo.proc.capture("pwd").out))

local bad, err = oslo.sys.cd("/no/such/directory")
print("bad cd=" .. tostring(bad) .. " has message=" .. tostring(err ~= nil))
-- stderr: yes
--[[ expect
matches=2
sorted=conf/a.conf conf/b.conf
no match=0
cd ok=true
pwd ends=sub
shell agrees=true
bad cd=nil has message=true
]]
