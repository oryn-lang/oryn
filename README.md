# ✦ Oryn

> A tiny language for game scripting.

## Status

Early development.

## Examples

Start with [`examples/`](./examples). A short, commented tour of the
language:

- [`01_hello.on`](./examples/01_hello.on) — variables, types, string interpolation, `print`
- [`02_functions.on`](./examples/02_functions.on) — `fn`, return types, recursion
- [`03_control_flow.on`](./examples/03_control_flow.on) — `if`/`elif`/`else`, `while`, `for`, ranges
- [`04_objects.on`](./examples/04_objects.on) — `obj` types, fields, methods, `self`
- [`05_composition.on`](./examples/05_composition.on) — field/method composition with `use`
- [`06_private_fields.on`](./examples/06_private_fields.on) — `pub` visibility and constructor patterns
- [`modules/`](./examples/modules) — the module system across multiple files

Run any of them with:

```bash
cargo run --bin oryn -- examples/01_hello.on
```

See [`examples/README.md`](./examples/README.md) for the full tour.
