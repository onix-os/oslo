-- A Lua string is a byte string, and now so is the shell's. There is still exactly *one* string
-- type: text and non-text are two representations of it that can never name the same string.

-- What is not text survives being read, written and read again.
local blob = "\137PNG\r\n\26\n\255\254\0\1"
oslo.fs.write("corpus-blob", blob)
local read = oslo.fs.read("corpus-blob")
print("len=" .. #read .. " same=" .. tostring(read == blob))
print("type=" .. type(read) .. " head=" .. read:sub(2, 4) .. " byte=" .. string.byte(read, 9))

-- Round trip through the shell and back, byte for byte.
oslo.fs.write("corpus-blob2", read)
print("roundtrip=" .. tostring(oslo.fs.read("corpus-blob2") == blob))

-- `string.pack` crosses the boundary intact — it goes out to a database and comes back.
local packed = string.pack("<i4i4", 7, -9999)
local db <close> = oslo.db.open("corpus-bytes")
db:set("row", packed)
local back = db:get("row")
local a, b = string.unpack("<i4i4", back)
print("packed=" .. tostring(back == packed) .. " a=" .. a .. " b=" .. b)

-- One string type: text read from a file is the literal, and keys a table the same way.
oslo.fs.write("corpus-text", "hello")
local text = oslo.fs.read("corpus-text")
print("text=" .. tostring(text == "hello"))
local keyed = { hello = 1, [blob] = 2 }
print("keys=" .. keyed[text] .. "," .. keyed[read])

-- And JSON refuses what it cannot represent rather than writing something else.
print("json=" .. tostring(not pcall(oslo.json.encode, { blob = blob })))

oslo.fs.remove("corpus-blob")
oslo.fs.remove("corpus-blob2")
oslo.fs.remove("corpus-text")

--[[ expect
len=12 same=true
type=string head=PNG byte=255
roundtrip=true
packed=true a=7 b=-9999
text=true
keys=1,2
json=true
]]
