local i = 0

while i < 200000 do
    local s = string.format("x=%d y=%d", i, i)
    i = i + 1
end

print(i)
