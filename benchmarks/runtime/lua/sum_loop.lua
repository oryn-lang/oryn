local total = 0

for _ = 1, 5 do
    local warm = 0
    local i = 0

    while i < 400000 do
        warm = warm + math.floor(i / 100)
        i = i + 1
    end
end

local i = 0

while i < 400000 do
    total = total + math.floor(i / 100)
    i = i + 1
end

print(total)
