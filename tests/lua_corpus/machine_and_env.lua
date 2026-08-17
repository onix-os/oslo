-- `$PATH` as the list it is, and what the machine can say about itself — both without a process.

-- A colon-separated variable, handled as a list rather than by string surgery.
oslo.env.set("PATH", "/usr/bin:/bin")
print("entries=" .. table.concat(oslo.env.path(), ","))

oslo.env.path_add("/opt/tool")
print("front=" .. oslo.env.path()[1])

-- Twice must not grow it: a configuration is loaded and reloaded.
oslo.env.path_add("/opt/tool")
print("count=" .. #oslo.env.path())

-- An entry further down is *moved* rather than repeated, so what was asked for wins.
oslo.env.path_add("/bin")
print("moved=" .. oslo.env.path()[1] .. " count=" .. #oslo.env.path())

-- Appending is the fallback end, and leaves what is already preferred where it is.
oslo.env.path_add("/last", { last = true })
print("last=" .. oslo.env.path()[#oslo.env.path()])

print("has=" .. tostring(oslo.env.has_path("/opt/tool")) .. "," .. tostring(oslo.env.has_path("/nope")))

-- A pattern, because the reason to remove one is usually everything under a prefix.
print("removed=" .. oslo.env.path_remove("/opt/*"))
print("after=" .. table.concat(oslo.env.path(), ","))

-- Any colon-separated variable, not only PATH.
oslo.env.set("MANPATH", "/usr/share/man")
oslo.env.path_add("/opt/man", { var = "MANPATH" })
print("man=" .. table.concat(oslo.env.path("MANPATH"), ","))

-- Empty entries are never produced or kept: a doubled colon means "here" to the linker.
oslo.env.set("P", "/a::/b:")
print("empties=" .. table.concat(oslo.env.path("P"), ","))

-- Facts about the machine, as numbers rather than as renderings.
print("cpus=" .. tostring(oslo.sys.cpus() >= 1))
print("arch=" .. tostring(#oslo.sys.arch() > 0))
print("kernel=" .. tostring(oslo.sys.kernel() ~= nil))
print("uptime=" .. tostring(oslo.sys.uptime() > 0))
print("load=" .. #oslo.sys.loadavg())
local m = oslo.sys.memory()
print("memory=" .. tostring(m.total > m.available) .. "," .. tostring(m.total > 1024 * 1024))

--[[ expect
entries=/usr/bin,/bin
front=/opt/tool
count=3
moved=/bin count=3
last=/last
has=true,false
removed=1
after=/bin,/usr/bin,/last
man=/opt/man,/usr/share/man
empties=/a,/b
cpus=true
arch=true
kernel=true
uptime=true
load=3
memory=true,true
]]
