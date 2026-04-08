# Oryn

A programming language for games, mods, and tools. Oryn combines Lua's simplicity with Rust's safety guarantees and static typing. It compiles to bytecode and runs on a garbage-collected virtual machine.

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

## Features

- **Objects with methods** - define types with fields and behavior
- **Composition via `use`** - inherit fields and methods without class hierarchies
- **Value bindings** - `let` for mutable, `val` for immutable
- **Type annotations** - `let x: i32 = 5`, `fn add(a: i32, b: i32) -> i32`
- **Primitives** - `i32`, `f32`, `bool`, `String`
- **Control flow** - `if`/`elif`/`else`, `while`, `break`, `continue`
- **Functions** - first-class, recursive, with closures coming soon
- **Compile-time checks** - undefined variables, val reassignment, field validation, arity mismatches
- **GC-managed heap** - objects are reference types collected by gc-arena
- **Editor support** - tree-sitter grammar, Neovim plugin, LSP with diagnostics and go-to-definition

## Quick start

```bash
cargo build
cargo run --bin oryn -- run examples/hello.on
```

## Status

Early development. See [docs/PLAN.md](../docs/PLAN.md) for the roadmap.
