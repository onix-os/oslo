-- A Lua tool that *consumes* rows: a verb, not a source.
--
-- Until this worked, every Lua tool was a producer — the pipeline had the previous stage's rows in
-- hand and dropped them, so `notes` was expressible and `redact` was not.

-- A source, to have something to transform.
oslo.register_tool {
  name = "three",
  produces = "rows",
  rows = function()
    return {
      { name = "alpha", size = 10 },
      { name = "beta", size = 200 },
      { name = "gamma", size = 3000 },
    }
  end,
}

-- A verb: rows in, rows out.
oslo.register_tool {
  name = "big",
  accepts = "rows",
  produces = "rows",
  rows = function(argv, input)
    local least = tonumber(argv[2]) or 0
    local kept = {}
    for _, row in ipairs(input or {}) do
      if row.size >= least then
        kept[#kept + 1] = row
      end
    end
    return kept
  end,
}

-- A verb that rewrites rather than filters, to show the rows are ordinary tables.
oslo.register_tool {
  name = "shout",
  accepts = "rows",
  produces = "rows",
  rows = function(_, input)
    for _, row in ipairs(input or {}) do
      row.name = row.name:upper()
    end
    return input
  end,
}

print("== filtering ==")
oslo.proc.exec("three | big 200 | cols name")
print("== rewriting, then a built-in verb after it ==")
oslo.proc.exec("three | shout | where 'name == \"BETA\"' | cols name size")
print("== a built-in verb before a Lua one ==")
oslo.proc.exec("three | where 'size > 5' | big 3000 | cols name")
print("== nothing reached it ==")
oslo.register_tool {
  name = "given",
  accepts = "rows",
  produces = "rows",
  rows = function(_, input)
    return { { got = input == nil and "nothing" or ("rows:" .. #input) } }
  end,
}
oslo.proc.exec("given")
oslo.proc.exec("three | given")

--[[ expect
== filtering ==
beta
gamma
== rewriting, then a built-in verb after it ==
BETA	200
== a built-in verb before a Lua one ==
gamma
== nothing reached it ==
nothing
rows:3
]]
