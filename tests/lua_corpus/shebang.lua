#!/usr/bin/env oslo
-- A `#!` line must reach neither the Lua parser (which cannot read `#`) nor the shell's.
--
-- `#!/usr/bin/env oslo` names the shell, not the language, so it decides nothing and the `.lua`
-- extension answers. This shipped broken: the shebang was read as "shell", which sent every oslo
-- Lua script to the shell parser. It was found by running the README's own example, not by a
-- test — which is what this corpus is for.
print("shebang did not reach the parser")
-- The shebang is blanked rather than removed, so line numbers still point at the source. An
-- error reports its own line, which is how that is visible without walking a traceback. The
-- expected line below is 12, so this comment has to stay two lines long.
local _, message = pcall(function() error("mark") end)
print("error reports line " .. message:match(":(%d+):"))
--[[ expect
shebang did not reach the parser
error reports line 12
]]
