local function f(a) return a + 1 end
local n = 0
for i = 1, 200000 do n = f(n) end
