local c = { value = 0 }

for _ = 1, 500000 do
    c.value = c.value + 1
end

print(c.value)
