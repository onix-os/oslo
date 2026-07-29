-- A Lua function becomes a shell builtin, ahead of PATH.
oslo.register_builtin("greet", function(argv)
  print("hello " .. (argv[2] or "world") .. " (called as " .. argv[1] .. ")")
  return 0
end)
oslo.exec("greet there")
oslo.exec("greet")

-- The return value is the exit status, in each of its spellings.
oslo.register_builtin("num", function() return 3 end)
oslo.register_builtin("yes_", function() return true end)
oslo.register_builtin("no_", function() return false end)
oslo.register_builtin("void", function() end)
print("number=" .. oslo.capture("num; echo $?").out)
print("true=" .. oslo.capture("yes_; echo $?").out)
print("false=" .. oslo.capture("no_; echo $?").out)
print("nothing=" .. oslo.capture("void; echo $?").out)

-- It really is a builtin, not something on PATH.
print(oslo.capture("type greet").out)
--[[ expect
hello there (called as greet)
hello world (called as greet)
number=3
true=0
false=1
nothing=0
greet is a shell builtin
]]
