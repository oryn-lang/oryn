local function make_adder(n)
    return function(x) return x + n end
end

local add10 = make_adder(10)
local total = 0
local i = 0

while i < 800000 do
    total = add10(i)
    i = i + 1
end

print(total)
