-- Lua has no enum type. Model Op as small integer constants and
-- dispatch via if/elseif. Measures the same high-level work shape
-- (construct tag, branch on tag, apply).

local OP_ADD  = 0
local OP_SUB  = 1
local OP_MUL  = 2
local OP_NOOP = 3

local function apply(op, acc)
    if op == OP_ADD then
        return acc + 1
    elseif op == OP_SUB then
        return acc - 1
    elseif op == OP_MUL then
        return acc * 2
    else
        return acc
    end
end

local total = 0
local i = 0
while i < 20000 do
    local r = i - math.floor(i / 4) * 4
    local op
    if r == 0 then
        op = OP_ADD
    elseif r == 1 then
        op = OP_SUB
    elseif r == 2 then
        op = OP_MUL
    else
        op = OP_NOOP
    end
    total = apply(op, total)
    i = i + 1
end

print(total)
