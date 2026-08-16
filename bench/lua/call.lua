local function f(a) return a + 1 end
local t0 = os.clock()
local n = 0
for i = 1, 200000 do n = f(n) end
print(string.format("call_200k %.3f", os.clock() - t0))
