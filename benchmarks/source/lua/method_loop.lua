local Counter = {}
Counter.__index = Counter

function Counter.new(value)
    return setmetatable({ value = value }, Counter)
end

function Counter:inc()
    self.value = self.value + 1
end

local c = Counter.new(0)
local i = 0

while i < 200000 do
    c:inc()
    i = i + 1
end

print(c.value)
