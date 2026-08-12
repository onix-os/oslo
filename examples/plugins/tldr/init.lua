-- Examples from tldr-pages, offered as completions and as a ghost.
--
-- **The worked example for both provider surfaces.** It answers from a local database, so it is
-- fast enough to be synchronous — which is the easy half. See `slowpoke` for the other one.

local db = oslo.db.open("tldr")

-- A page is `command \t example \t description`, one per line. Seeded here with a handful rather
-- than fetched, so the example is a plugin and not a downloader; `tldr --sync` is the real thing.
local seed = {
  { "git", "git commit --amend", "change the last commit" },
  { "git", "git rebase -i HEAD~3", "rewrite the last three commits" },
  { "git", "git switch -c NAME", "start a branch" },
  { "tar", "tar xf ARCHIVE", "extract an archive" },
  { "tar", "tar czf OUT.tar.gz DIR", "make a gzipped archive" },
}

local function key(command, n) return command .. "/" .. n end

local function fill()
  if db:has("seeded") then return end
  db:write(function(w)
    local counts = {}
    for _, page in ipairs(seed) do
      local command = page[1]
      counts[command] = (counts[command] or 0) + 1
      w:set(key(command, counts[command]), page[2] .. "\t" .. page[3])
    end
    w:set("seeded", "1")
  end)
end
fill()

-- Everything known about one command.
local function pages(command)
  local found = {}
  for _, k in ipairs(db:keys(command .. "/")) do
    local line = db:get(k) or ""
    local example, description = line:match("^(.-)\t(.*)$")
    if example then found[#found + 1] = { example = example, desc = description } end
  end
  return found
end

-- The dropdown: examples *beside* what oslo already knows about the command, not instead of it.
oslo.completion.provider {
  name = "tldr",
  kind = "example",
  -- Below a command you actually run, so `git c<Tab>` still offers `commit` first. An example is
  -- worth reading when nothing better matched, which is what a negative offset says.
  score_offset = -5,
  max_items = 5,
  answer = function(ctx)
    local out = {}
    for _, page in ipairs(pages(ctx.command)) do
      -- The dropdown replaces the word being typed, so the offer is the rest of the line rather
      -- than the whole of it.
      local rest = page.example:match("^" .. ctx.command .. "%s+(.*)$")
      if rest then out[#out + 1] = { display = rest, desc = page.desc } end
    end
    return out
  end,
}

-- The ghost: the first example that continues what has been typed.
oslo.suggest.provider {
  name = "tldr",
  min_chars = 3,
  answer = function(ctx)
    local command = ctx.line:match("^(%S+)")
    if not command then return nil end
    for _, page in ipairs(pages(command)) do
      if page.example:sub(1, #ctx.line) == ctx.line then return page.example end
    end
  end,
}

-- `tldr git` prints what it knows, which is also how you check the database survived a restart.
oslo.register_builtin {
  name = "tldr",
  desc = "examples for a command, from tldr-pages",
  run = function(argv)
    local found = pages(argv[2] or "")
    if #found == 0 then
      print("tldr: nothing for " .. (argv[2] or "<command>"))
      return 1
    end
    for _, page in ipairs(found) do print(page.example .. "  -- " .. page.desc) end
    return 0
  end,
}

oslo.plugin.test("the database is seeded on load", function(t)
  t.ok(db:has("seeded"), "the seed ran")
  t.equal(#pages("git"), 3, "three git pages")
  t.equal(#pages("nothing"), 0, "and none for a command it has never heard of")
end)

oslo.plugin.test("a page is offered as a completion and as a ghost", function(t)
  local first = pages("git")[1]
  t.ok(first.example:match("^git "), "an example starts with its command")
  t.ok(#first.desc > 0, "and says what it does")
end)
