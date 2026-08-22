-- The plugin proper, run the first time a line mentions `note` or `notes`.
--
-- **Only public API.** A database, a builtin and a row-producing tool — none of which needed a
-- change to oslo to be reachable from here.

local db = oslo.db.open("notes")

-- `note "text"` writes one down; `note` with nothing prints them.
oslo.register_builtin{ name = "note", run = function(argv)
  if argv[2] then
    db:set(os.date("!%Y-%m-%dT%H:%M:%SZ"), argv[2])
    return 0
  end
  for _, key in ipairs(db:keys()) do
    print(key .. "  " .. (db:get(key) or ""))
  end
  return 0
end }

-- The same notes as rows, so they compose: `notes | where 'note:match("shell")' | cols at`.
--
-- Declared before the tests below so they have something to call.
oslo.register_tool {
  name = "notes",
  produces = "rows",
  rows = function(_)
    local rows = {}
    for _, key in ipairs(db:keys()) do
      rows[#rows + 1] = { at = key, note = db:get(key) or "" }
    end
    return rows
  end,
}

-- What `oslo plugin test` runs. A temporary home means the database really is empty, which is the
-- state a user sees on their first day and the one an author never has.
oslo.plugin.test("a fresh install has no notes", function(t)
  t.equal(#db:keys(), 0, "the database starts empty")
end)

oslo.plugin.test("a note written is a note kept", function(t)
  db:set("2026-01-01T00:00:00Z", "hello")
  t.equal(db:get("2026-01-01T00:00:00Z"), "hello", "it reads back")
  t.ok(db:has("2026-01-01T00:00:00Z"), "and it is there")
  db:delete("2026-01-01T00:00:00Z")
end)
