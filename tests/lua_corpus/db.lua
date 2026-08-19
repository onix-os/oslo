-- A database a config owns: `oslo.db`.
local db = oslo.db.open("corpus")
print("opened=" .. tostring(db ~= nil))

-- A value is bytes, kept exactly: no trimming, no added newline, embedded ones survive.
db:set("greeting", "hello\nworld")
print("get=" .. oslo.json.encode(db:get("greeting")))

-- An absent key is nil; an empty value is present, which `get` alone cannot tell you.
db:set("empty", "")
print("missing=" .. tostring(db:get("nope")))
print("has_missing=" .. tostring(db:has("nope")) .. " has_empty=" .. tostring(db:has("empty")))

-- One transaction, and keys read back in order under their prefix.
db:write(function(w)
  w:set("a/2", "two")
  w:set("a/1", "one")
  w:set("b/1", "other")
end)
print("prefix=" .. table.concat(db:keys("a/"), ","))

-- Deleting says whether there was anything there.
print("delete=" .. tostring(db:delete("b/1")) .. "," .. tostring(db:delete("b/1")))

-- A name that could reach another database never opens one.
local escaped, why = oslo.db.open("../history")
print("escaped=" .. tostring(escaped) .. " refused=" .. tostring(why ~= nil))

-- A handle is an object. Its verbs live behind `__index`, so it has no keys of its own and
-- `db:write`'s internals are not part of what `pairs` walks.
local keys = 0
for _ in pairs(db) do keys = keys + 1 end
print("own_keys=" .. keys)

-- A typo is refused rather than quietly added, and a dot instead of a colon is a message rather
-- than a read of the wrong key.
print("typo=" .. tostring(not pcall(function() db.nmae = 1 end)))
print("dot=" .. tostring(not pcall(function() return db.get("greeting") end)))

-- `<close>` releases at the end of the block, and every verb says so afterwards.
local kept
do
  local scoped <close> = oslo.db.open("corpus-scoped")
  scoped:set("x", "y")
  kept = scoped
end
print("after_close=" .. tostring(not pcall(function() return kept:get("x") end)))
print("reopened=" .. tostring(oslo.db.open("corpus-scoped"):get("x")))

--[[ expect
opened=true
get="hello\nworld"
missing=nil
has_missing=false has_empty=true
prefix=a/1,a/2
delete=true,false
escaped=nil refused=true
own_keys=0
typo=true
dot=true
after_close=true
reopened=y
]]
