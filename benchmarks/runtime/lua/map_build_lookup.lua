local m = {}
local i = 0

while i < 10000 do
    m[i] = i * 2
    i = i + 1
end

local total = 0
local j = 0
while j < 10000 do
    total = total + (m[j] or 0)
    j = j + 1
end

print(total)
