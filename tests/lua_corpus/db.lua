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

--[[ expect
opened=true
get="hello\nworld"
missing=nil
has_missing=false has_empty=true
prefix=a/1,a/2
delete=true,false
escaped=nil refused=true
]]
