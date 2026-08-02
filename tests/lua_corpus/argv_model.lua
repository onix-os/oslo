-- The argv call model: `oslo.run{…}` and the `sh` sugar over it.
--
-- The rest of the corpus exercises `oslo.proc.exec`/`oslo.proc.capture`, which take a *command line* and so
-- have a quoting step between the script and the program. This one takes the words directly, which
-- is the whole point: there is no shell parse in the middle, so an argument holding a space, a
-- quote or a `$` reaches the program exactly as written and there is no quoting bug to have.

local r = oslo.run { "printf", "%s|", "a b", "c'd", 'e"f', "g$h", capture = true }
print("status=" .. r.status)
print("argv survived=" .. r.out)

-- Nothing is expanded on the way through. A glob is a literal argument, not a pattern, because
-- there is no shell to expand it — this is what makes `oslo.run{"rm", name}` safe for a filename
-- the script did not choose.
local star = oslo.run { "printf", "%s", "*", capture = true }
print("star is literal=" .. tostring(star.out == "*"))

-- A failing command answers a status rather than raising: the script decides what a failure means.
local bad = oslo.run { "false", capture = true }
print("false status=" .. bad.status)

-- `sh.<name>` is the same call with the command as the name. It does not capture — its output goes
-- straight to the terminal, the way a command typed at the prompt does — and answers the status.
local echoed = sh.printf("[%s]", "one two")
print("")
print("sh status=" .. echoed.status)
print("sh captures nothing=" .. tostring(echoed.out == nil))

--[[ expect
status=0
argv survived=a b|c'd|e"f|g$h|
star is literal=true
false status=1
[one two]
sh status=0
sh captures nothing=true
]]
