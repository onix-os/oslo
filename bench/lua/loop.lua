local t0 = os.clock()
local n = 0
for i = 1, 1000000 do n = n + i end
print(string.format("loop_1M %.3f", os.clock() - t0))
