local xs = {}
local i = 0
while i < 100 do
    xs[#xs + 1] = i
    i = i + 1
end

local total = 0
local round = 0
while round < 200 do
    for _, x in ipairs(xs) do
        total = total + x
    end
    round = round + 1
end

print(total)
