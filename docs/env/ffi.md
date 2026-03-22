---
layout: default
title: Foreign Function Interface (FFI)
---

# Foreign Function Interface (FFI)

Nexus interoperates with WebAssembly modules, allowing extension with functions written in Rust, C, or other languages that compile to WASM.

## Importing WASM Modules

Load a WASM module with `import external`:

```nexus
import external "math.wasm"
```

The module's exports become available for binding.

## External Bindings

Bind a WASM export to a Nexus name:

```nexus
export external add_ints = "add" : (a: i64, b: i64) -> i64
external internal_helper = "helper" : (x: i64) -> unit
```

- `export` makes the binding visible to other modules
- The string literal after `=` is the WASM export name
- The type after `:` must be an arrow type

## Generic External Bindings

Polymorphic externals require explicit type parameters:

```nexus
export external length = "array_length" : <T>(arr: &[| T |]) -> i64
```

Using an undeclared type variable (e.g., `T` without `<T>`) is a type error. This prevents typos from silently becoming type variables.

## Type Mapping

| Nexus Type | WASM Type | Notes |
|---|---|---|
| `i64` | `i64` | Direct |
| `float` / `f64` | `f64` | Direct |
| `i32` | `i32` | Via `i64` casting |
| `f32` | `f32` | Via `float` casting |
| `bool` | `i32` | 0 = false, 1 = true |
| `string` | `i64` | Packed as (offset, length) pair |
| `unit` | (none) | No WASM parameter generated |
| Records | `i64` | Heap pointer |

## Example

```nexus
import external "utils.wasm"

external process_data = "process" : (val: float) -> float

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    let result = process_data(val: 42.0)
    Console.println(val: string.from_float(val: result))
  end
  return ()
end
```
