# ✦ Oryn

> A tiny language for game scripting.

## Status

Early development.

## [Benchmarks](./benchmarks/report.md)

- Overall: across 12 benchmarks, Oryn is about 1.23x slower than Lua and 6.69x slower than LuaJIT.
- Source: across 6 benchmarks, Oryn is about 2.05x slower than Lua and 5.27x slower than LuaJIT.
- Runtime: across 6 benchmarks, Oryn is about 0.74x slower than Lua and 8.51x slower than LuaJIT.

## Example

```oryn
obj Health {
    hp: i32
    
    fn heal(self, amount: i32) {
        self.hp = self.hp + amount
    }
    
    fn is_alive(self) -> bool {
        rn self.hp > 0
    }
}

obj Player {
    use Health
    name: String
}

let player = Player { hp: 100, name: "Alice" }

player.heal(20)
print(player.hp)         // 120
print(player.is_alive()) // true
```
