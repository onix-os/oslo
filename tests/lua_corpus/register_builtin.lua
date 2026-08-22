-- A Lua function becomes a shell builtin, ahead of PATH.
oslo.register_builtin{ name = "greet", run = function(argv)
  print("hello " .. (argv[2] or "world") .. " (called as " .. argv[1] .. ")")
  return 0
end }
oslo.proc.exec("greet there")
oslo.proc.exec("greet")

-- The return value is the exit status, in each of its spellings.
oslo.register_builtin{ name = "num", run = function() return 3 end }
oslo.register_builtin{ name = "yes_", run = function() return true end }
oslo.register_builtin{ name = "no_", run = function() return false end }
oslo.register_builtin{ name = "void", run = function() end }
print("number=" .. oslo.proc.capture("num; echo $?").out)
print("true=" .. oslo.proc.capture("yes_; echo $?").out)
print("false=" .. oslo.proc.capture("no_; echo $?").out)
print("nothing=" .. oslo.proc.capture("void; echo $?").out)

-- It really is a builtin, not something on PATH.
print(oslo.proc.capture("type greet").out)
--[[ expect
hello there (called as greet)
hello world (called as greet)
number=3
true=0
false=1
nothing=0
greet is a shell builtin
]]
