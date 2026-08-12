-- `oslo.messages` and the `messages` builtin — what this session said, after it has scrolled.
--
-- **A session's own memory, not a log.** Another shell has its own, and this one forgets when it
-- exits. That is the whole point: a plugin that failed at startup said so once, and the line is
-- otherwise gone twenty commands later.

oslo.messages.clear()

oslo.messages.note("notes", "loaded 3 notes")
oslo.messages.note("notes", "loaded 3 notes")
oslo.messages.note("notes", "loaded 3 notes")

local said = oslo.messages.all()
print("kept: " .. #said)
print("counted: " .. said[1].times)
print("source: " .. said[1].source .. " / " .. said[1].level)

-- Something else in between, then the same line again: that is a second occurrence, not a repeat of
-- the first, because it stopped and started.
oslo.messages.note("other", "hello")
oslo.messages.note("notes", "loaded 3 notes")
print("after an interruption: " .. #oslo.messages.all())

-- The builtin reads the same buffer, and filters it by source.
print("== messages notes ==")
oslo.proc.exec("messages notes | cut -d: -f2-")

print("== nothing matches ==")
oslo.proc.exec("messages nosuchthing")
print("status: " .. oslo.proc.exec("messages --errors"))

oslo.messages.clear()
print("cleared: " .. #oslo.messages.all())

--[[ expect
kept: 1
counted: 3
source: notes / note
after an interruption: 3
== messages notes ==
 loaded 3 notes  (x3)
 loaded 3 notes
== nothing matches ==
status: 0
cleared: 0
]]
