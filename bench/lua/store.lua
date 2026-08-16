local t0 = os.clock()
local x = {}
for i = 1, 1000000 do x[i] = i * 2 end
print(string.format("store_1M %.3f", os.clock() - t0))
