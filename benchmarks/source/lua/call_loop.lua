local function add_one(n)
    return n + 1
end

local i = 0
local total = 0

while i < 200000 do
    total = add_one(total)
    i = i + 1
end

print(total)
