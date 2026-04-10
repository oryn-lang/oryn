local Counter = {}
Counter.__index = Counter

function Counter.new(value)
    return setmetatable({ value = value }, Counter)
end

function Counter:inc()
    self.value = self.value + 1
end

local c = Counter.new(0)

for _ = 1, 500000 do
    c:inc()
end

print(c.value)
