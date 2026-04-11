-- Lua doesn't have error unions. The closest idiomatic equivalent is
-- returning (value, err) pairs and checking `err` at each step. This
-- is what Lua users actually write for expected-failure paths; it
-- mirrors the work Oryn's `try` does (check, propagate, unwrap).

local function safe_div(a, b)
    if b == 0 then
        return nil, "divbyzero"
    end
    return math.floor(a / b), nil
end

local function step(x)
    local a, err = safe_div(x, 2)
    if err ~= nil then return nil, err end
    local b
    b, err = safe_div(a, 1)
    if err ~= nil then return nil, err end
    local c
    c, err = safe_div(b, 1)
    if err ~= nil then return nil, err end
    return c + 1, nil
end

local total = 0
local i = 0
while i < 10000 do
    local v, err = step(i + 2)
    if err ~= nil then error(err) end
    total = v
    i = i + 1
end

print(total)
