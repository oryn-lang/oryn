local xs = {0}
local i = 0

while i < 20000 do
    xs[#xs + 1] = i
    i = i + 1
end

print(#xs)
