local total = 0
local i = 0

while i < 400000 do
    total = total + math.floor(i / 100)
    i = i + 1
end

print(total)
