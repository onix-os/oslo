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

-- `glob = true` asks for the pattern to be read, for the call that wants it. Off by default so
-- that the guarantee above holds for every call that did not say so — the safety is the default,
-- not the option. A pattern matching nothing comes back unchanged, as in any shell without
-- `nullglob`, which is what lets an argument that merely *contains* a metacharacter through.
local none = oslo.run { "printf", "%s", "*.no-such-suffix", glob = true, capture = true }
print("unmatched stays put=" .. tostring(none.out == "*.no-such-suffix"))

-- The command word is never expanded, only its arguments. A command named by a glob is never what
-- was meant, and one that expanded to nothing would leave the first argument looking like a
-- command. The same rule the shell's own `run_simple` follows.
local named = oslo.run { "tru*", glob = true, capture = true }
print("command word not globbed=" .. tostring(named.status == 127))

-- A failing command answers a status rather than raising: the script decides what a failure means.
local bad = oslo.run { "false", capture = true }
print("false status=" .. bad.status)

-- `sh.<name>` is the same call with the command as the name. It does not capture — its output goes
-- straight to the terminal, the way a command typed at the prompt does — and answers the status.
local echoed = sh.printf("[%s]", "one two")
print("")
print("sh status=" .. echoed.status)
print("sh captures nothing=" .. tostring(echoed.out == nil))

-- **And it expands patterns, which `oslo.run` does not.** The one difference between the two
-- forms, and the reason both exist: `sh.rm("*.txt")` is written to read like the command line it
-- stands in for, and a command line reads its patterns. A caller who needs an argument left alone
-- whatever it holds writes `oslo.run{…}` without `glob`.
--
-- Note what this line proves in passing: `[%s]` is a bracket expression, matches no file here, and
-- so arrives at printf unchanged. That is the no-match rule doing the work of quoting.
sh.rm("no-such-file-*.tmp")

--[[ expect
status=0
argv survived=a b|c'd|e"f|g$h|
star is literal=true
unmatched stays put=true
command word not globbed=true
false status=1
[one two]
sh status=0
sh captures nothing=true
]]
-- stderr: yes
