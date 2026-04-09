local c = { value = 0 }
local i = 0

while i < 200000 do
    c.value = c.value + 1
    i = i + 1
end

print(c.value)
