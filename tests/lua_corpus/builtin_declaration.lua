-- `oslo.register_builtin` in its table form: a command that can say what it is.

-- The form that has always existed still works, untouched.
oslo.register_builtin{ name = "older", run = function()
  print("older ran")
  return 0
end }

-- The table form: the same, plus a description and its own completion.
oslo.register_builtin {
  name = "note",
  desc = "write a note down",
  run = function(argv)
    print("note ran with " .. (argv[2] or "nothing"))
    return 0
  end,
  complete = function(prior, word)
    return { "yesterday", "today" }
  end,
}

oslo.proc.exec("older")
oslo.proc.exec("note something")

-- What has been declared, and what each said it is for.
for _, b in ipairs(oslo.builtins()) do
  print(b.name .. " = " .. tostring(b.desc))
end

-- **A declared `complete` lands where completion is actually read**, rather than in a table of its
-- own that would then have to agree with this one.
print("wired: " .. type(oslo.completion.for_command.note))

--[[ expect
older ran
note ran with something
note = write a note down
older = nil
wired: function
]]
