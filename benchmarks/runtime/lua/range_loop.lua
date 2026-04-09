local total = 0

for _ = 1, 5 do
    local warm = 0

    for i = 0, 49999 do
        warm = warm + i
    end
end

for i = 0, 49999 do
    total = total + i
end

print(total)
