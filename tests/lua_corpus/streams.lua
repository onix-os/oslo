-- Everything in oslo that iterates is lazy, and the iterator is a handle: callable through
-- `__call` so a generic `for` takes it, with `__close` so an abandoned loop can let go.

-- A command's output, a line at a time.
local seen = {}
for line in oslo.lines{"printf", "a\nb\nc\n"} do seen[#seen + 1] = line end
print("lines=" .. table.concat(seen, ","))

-- The handle is the iterator, so it can also be closed. `close` answers whether this call was
-- the one that did it.
local out = oslo.lines{"seq", "1", "100000"}
local first = out()
print("first=" .. first .. " close=" .. tostring(out:close()) .. "," .. tostring(out:close()))
print("after=" .. tostring(select(2, pcall(out))))

-- A file's lines, without reading the file.
oslo.fs.write("corpus-lines", "one\ntwo\nthree\n")
local n = 0
do
  local f <close> = oslo.fs.lines("corpus-lines")
  for _ in f do n = n + 1 end
end
print("file=" .. n)

-- A tree, depth first, directories before their contents and symlinks never followed.
oslo.fs.mkdir("corpus-tree/inner")
oslo.fs.write("corpus-tree/inner/leaf", "")
oslo.fs.symlink("..", "corpus-tree/inner/up")
local found = {}
for path in oslo.fs.walk("corpus-tree") do found[#found + 1] = path end
table.sort(found)
print("walk=" .. table.concat(found, " "))

-- Stopping stops the walk rather than the reading of it.
local took = 0
do
  local tree <close> = oslo.fs.walk("corpus-tree")
  for _ in tree do took = took + 1; break end
end
print("took=" .. took)

-- What is not there is a message, before anything is opened.
print("missing=" .. tostring(oslo.fs.walk("corpus-nope")) .. "," .. tostring(oslo.fs.lines("corpus-nope")))

oslo.fs.remove("corpus-lines")
oslo.fs.remove("corpus-tree", true)

--[[ expect
lines=a,b,c
first=1 close=true,false
after=oslo.lines: the handle is closed
file=3
walk=corpus-tree/inner corpus-tree/inner/leaf corpus-tree/inner/up
took=1
missing=nil,nil
]]
