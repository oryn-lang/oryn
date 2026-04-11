# Oryn examples

Short, runnable programs that walk through Oryn's features in roughly
ascending order of complexity. If you're exploring the language for
the first time, read them in order — each one introduces a little
more than the previous.

Every example runs with:

```bash
cargo run --bin oryn -- examples/<file>.on
```

from the `oryn/` directory (where `Cargo.toml` lives).

## Single-file examples

| File                      | What it shows |
|---------------------------|---------------|
| [`01_hello.on`](01_hello.on)                   | `let` / `val` bindings, primitive types, string interpolation, `print`, arithmetic, booleans. The smallest useful program. |
| [`02_functions.on`](02_functions.on)           | `fn` declarations, required parameter type annotations, return types, the `return` keyword, recursion (fibonacci). |
| [`03_control_flow.on`](03_control_flow.on)     | `if` / `if not` / `elif` / `else`, `while`, `for x in <range>`, `..` vs `..=`, `break`, `continue`. |
| [`04_objects.on`](04_objects.on)               | `struct` declarations, fields, instance methods with `self`, static methods, object literals. The struct-with-methods that replaces Lua tables. |
| [`05_composition.on`](05_composition.on)       | `use` for field/method composition — Oryn's answer to shared health, inventory, etc. without inheritance trees. |
| [`06_private_fields.on`](06_private_fields.on) | `pub` visibility on fields and the static-constructor pattern for enforcing invariants. |
| [`07_nil_and_errors.on`](07_nil_and_errors.on) | `nil`, nillable values, error unions, `try`, `orelse`, and explicit error construction. |
| [`08_tests.on`](08_tests.on)                   | `test` blocks and `assert(...)`. |
| [`09_lists.on`](09_lists.on)                   | `[T]` list types, list literals, indexing, assignment, methods, and iteration. |
| [`10_maps.on`](10_maps.on)                     | `{K: V}` map types, map literals, empty maps, indexing, and index assignment. |
| [`11_enums.on`](11_enums.on)                   | `enum` declarations with nullary and payload variants, constructors, structural equality, and `match` as an expression with wildcard arms. |

## Multi-file example

[`modules/`](modules/) is a small project that demonstrates the module
system end-to-end: nested imports, qualified types and literals,
cross-module method calls, private-field enforcement across module
boundaries, and why imports are non-transitive.

Run it with:

```bash
cargo run --bin oryn -- examples/modules/main.on
```

The entry point is `modules/main.on`. From there, follow the
`import` statements at the top to see how the pieces fit together.

## Using these with the LSP

If you've got the Oryn language server hooked up in your editor,
hovering over any identifier in these files will show its type and
any `//` comments attached above its declaration. The `modules/`
example is especially good for trying out cross-file goto-definition
and signature help.
