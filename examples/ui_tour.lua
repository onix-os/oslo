-- A tour of `oslo.ui`, the Lua half of the drawing and asking API.
--
--   oslo examples/ui_tour.lua            the measuring and drawing, checked as it goes
--   oslo examples/ui_tour.lua --ask      and the widgets that need a person
--
-- `examples/ui_tour.sh` is the sibling of this: the same widgets reached through the `ui` builtin,
-- for a shell script. This one is the Lua surface, which is larger — the shell builtin cannot
-- measure a string in cells or lay out a table.
--
-- **It checks itself.** Every non-interactive step asserts what it produced, so running this is a
-- smoke test of the whole surface and not only a demonstration. That matters here: `oslo.ui.style`
-- was installed twice for months, each overwriting the other, and nothing noticed because nothing
-- exercised the Lua side end to end.

local failures = 0
local step = 0

local function check(what, got, want)
  if got ~= want then
    failures = failures + 1
    io.write("  FAIL  " .. what .. "\n    got  " .. tostring(got) .. "\n    want " .. tostring(want) .. "\n")
  end
end

local function heading(title)
  step = step + 1
  print("")
  print(oslo.ui.style(step .. "/12  " .. title, { fg = "magenta", bold = true }))
end

-- ---------------------------------------------------------------- the terminal

heading("what the terminal is")
local w, h = oslo.ui.width(), oslo.ui.height()
print(("width %d  height %d  tty %s  colours %s"):format(w, h, tostring(oslo.ui.is_tty()), oslo.ui.colors()))
-- Both dimensions are load-bearing: every layout below divides by the width, and a zero would be a
-- division by zero in somebody's shell rather than a wonky column.
check("width is positive", w > 0, true)
check("height is positive", h > 0, true)

-- ---------------------------------------------------------------- measuring

heading("measuring in cells, not bytes")
-- The whole reason `width_of` exists. A terminal has cells; a string has bytes; an escape sequence
-- has bytes and no cells at all.
check("plain", oslo.ui.width_of("cargo"), 5)
check("escapes are not width", oslo.ui.width_of("\27[1;36mcargo\27[0m"), 5)
check("strip removes them", oslo.ui.strip("\27[1;36mcargo\27[0m"), "cargo")
print("  cargo            = " .. oslo.ui.width_of("cargo") .. " cells")
print("  \27[36mcargo\27[0m (coloured) = " .. oslo.ui.width_of("\27[36mcargo\27[0m") .. " cells")
print("  📁 (emoji)       = " .. oslo.ui.width_of("📁") .. " cells")

heading("cutting and padding to an exact size")
check("truncate marks the cut", oslo.ui.truncate("abcdefghij", 5), "abcd…")
check("truncate leaves it alone", oslo.ui.truncate("abc", 5), "abc")
check("pad left", oslo.ui.pad("ab", 5), "ab   ")
check("pad right", oslo.ui.pad("ab", 5, "right"), "   ab")
check("fit is both", oslo.ui.width_of(oslo.ui.fit("abcdefghij", 6)), 6)
check("fit pads too", oslo.ui.width_of(oslo.ui.fit("ab", 6)), 6)
print("  truncate('abcdefghij', 5) = " .. oslo.ui.truncate("abcdefghij", 5))
print("  pad('ab', 6, 'center')    = [" .. oslo.ui.pad("ab", 6, "center") .. "]")

-- ---------------------------------------------------------------- painting

heading("painting text")
-- **Both call shapes.** The two-argument form used to take the string and drop the spec silently,
-- which is the bug this file would have caught.
local two_arg = oslo.ui.style("green", { fg = "green" })
local table_form = oslo.ui.style{ text = "green", fg = "green" }
check("both shapes agree", two_arg, table_form)
check("the text survives", oslo.ui.strip(two_arg), "green")
print("  " .. oslo.ui.style("green", { fg = "green" })
   .. "  " .. oslo.ui.style("bold red", { fg = "red", bold = true })
   .. "  " .. oslo.ui.style("on blue", { fg = "white", bg = "blue" }))
print(oslo.ui.style{ text = "bordered", border = "rounded", padding_x = 1, border_fg = "cyan" })

-- ---------------------------------------------------------------- layout

heading("columns")
local months = { "January", "February", "March", "April", "May", "June",
                 "July", "August", "September", "October", "November", "December" }
local packed = oslo.ui.columns(months)
check("columns answers lines", type(packed), "table")
check("nothing was lost", #packed > 0, true)
oslo.ui.print(packed)

heading("a grid")
local rows = {
  { "cargo build", "12.4s", "ok" },
  { "cargo test",  "3.1s",  "failed" },
  { "cargo clippy", "0.9s", "ok" },
}
-- **`grid`, not `table`.** `oslo.ui.table` is the interactive row *picker*, which is what the `ui`
-- builtin's `table` is too. This one formats. They used to share a name, and the picker — installed
-- second — won, so a script asking for a formatted table quietly got `nil`.
local drawn = oslo.ui.grid(rows, { headers = { "step", "took", "result" }, align = { "left", "right", "left" } })
check("grid answers lines", type(drawn), "table")
check("a header row plus three", #drawn, 4)
oslo.ui.print(drawn)

heading("a rule")
print(oslo.ui.rule())
print(oslo.ui.rule("=", 20))

-- ---------------------------------------------------------------- blocks

heading("a block: a headline and a rail")
-- The shape every report oslo prints has, and the same code it uses to print them.
local b = oslo.ui.block(oslo.ui.style("direnv loaded", { fg = "green", bold = true }))
b:row("PATH", "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaa/bin:/nix/store/bbbbbbbbbbbbbbbbbbbb/bin:/usr/bin",
      { overflow = "ellipsis", label_style = "cyan" })
b:row("aliases", "_b _c _r _t _v", { label_style = "magenta", style = "magenta" })
b:note("and a note, for a detail under the row above")
b:done()

heading("the three overflow policies")
local long = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi"
for _, how in ipairs({ "count", "ellipsis", "wrap" }) do
  local o = oslo.ui.block(oslo.ui.style(how, { fg = "yellow" }))
  o:row("names", long, { overflow = how })
  o:done()
end
-- A misspelt policy is refused rather than defaulted: a typo must not silently cut a value you
-- meant to wrap.
local ok = pcall(function()
  oslo.ui.block("x"):row("a", "b", { overflow = "elipsis" })
end)
check("a bad policy is an error", ok, false)

heading("a block without printing it")
-- `lines()` hands the rows back, for a caller putting them somewhere other than the screen.
local q = oslo.ui.block("head")
q:row("one", "first")
q:row("two", "second")
local lines = q:lines()
check("headline plus two rows", #lines, 3)
check("the headline is first", lines[1], "head")
for i, line in ipairs(lines) do print(("  [%d] %s"):format(i, line)) end

-- ---------------------------------------------------------------- reporting

heading("log lines")
oslo.ui.log{ message = "a plain note", level = "info" }
oslo.ui.log{ message = "something to look at", level = "warn", fields = { where = "ui_tour" } }
oslo.ui.log{ message = "and something wrong", level = "error", fields = { step = step } }

-- ---------------------------------------------------------------- asking

heading("asking")
-- `confirm` works without a terminal: the raw-mode widget falls back to an ordinary line, so a
-- script down a pipe gets an answer rather than `nil`. Not exercised here — it would block on
-- stdin — but it is the reason this file does not skip `confirm` for lack of a tty.
print("  oslo.ui.confirm falls back to a line when there is no tty")

if not (arg and arg[1] == "--ask") then
  print("  skipped — rerun with --ask to try the widgets that need a person")
else
  local name = oslo.ui.input{ prompt = "your name? ", placeholder = "nobody" }
  print("  input    -> " .. tostring(name))

  local sure = oslo.ui.confirm{ question = "carry on?", yes = "yes", no = "stop" }
  print("  confirm  -> " .. tostring(sure))

  local picked = oslo.ui.choose{ items = months, header = "pick a month", height = 8 }
  print("  choose   -> " .. tostring(picked))

  local several = oslo.ui.choose{ items = months, header = "pick a few", multi = true, height = 8 }
  print("  multi    -> " .. (type(several) == "table" and table.concat(several, ", ") or tostring(several)))

  local row = oslo.ui.table{ rows = rows, headers = { "step", "took", "result" } }
  print("  table    -> " .. (type(row) == "table" and table.concat(row, " | ") or tostring(row)))

  local status = oslo.ui.spin{ title = "working", command = { "sleep", "1" } }
  print("  spin     -> exit " .. tostring(status))
end

-- ----------------------------------------------------------------

print("")
if failures == 0 then
  print(oslo.ui.style("all checks passed", { fg = "green", bold = true }))
else
  print(oslo.ui.style(failures .. " check(s) failed", { fg = "red", bold = true }))
  oslo.proc.exit(1)
end
