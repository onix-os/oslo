-- The second half of `nil, message` is an object that still *is* the message.

local text, err = oslo.fs.read("corpus-nothing-here")
print("value=" .. tostring(text))
print("kind=" .. err.kind .. " code=" .. err.code)
print("path=" .. err.path)

-- Everything that read the message still reads it: `tostring`, `..`, and the string methods.
print("tostring=" .. tostring(err):match("^(%S+):"))
print("concat=" .. ("read: " .. err):sub(1, 5))
print("find=" .. tostring(err:find("corpus%-nothing") ~= nil))
print("upper=" .. err:upper():sub(1, 6))

-- The kind is the fact; the sentence is a rendering of it, and the errno is the same fact again.
oslo.fs.write("corpus-in-the-way", "")
local _, taken = oslo.fs.mkdir("corpus-in-the-way")
print("taken=" .. taken.kind .. " code=" .. taken.code)
oslo.fs.remove("corpus-in-the-way")

-- Two ends, so which one failed is not a question about the sentence.
local _, moved = oslo.fs.rename("corpus-nothing-here", "corpus-elsewhere")
print("from=" .. moved.path .. " to=" .. moved.to)

-- A database says which name it refused.
local db, why = oslo.db.open("../history")
print("db=" .. tostring(db) .. " name=" .. why.name .. " kind=" .. why.kind)

--[[ expect
value=nil
kind=not-found code=2
path=corpus-nothing-here
tostring=corpus-nothing-here
concat=read:
find=true
upper=CORPUS
taken=exists code=17
from=corpus-nothing-here to=corpus-elsewhere
db=nil name=../history kind=open
]]
