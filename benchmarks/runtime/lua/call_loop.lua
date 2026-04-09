local function add_one(n)
    return n + 1
end

for _ = 1, 5 do
    local warm = 0

    for _ = 1, 500000 do
        warm = add_one(warm)
    end
end

local total = 0

for _ = 1, 500000 do
    total = add_one(total)
end

print(total)
