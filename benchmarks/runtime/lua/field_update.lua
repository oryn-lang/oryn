for _ = 1, 5 do
    local warm = { value = 0 }

    for _ = 1, 500000 do
        warm.value = warm.value + 1
    end
end

local c = { value = 0 }

for _ = 1, 500000 do
    c.value = c.value + 1
end

print(c.value)
