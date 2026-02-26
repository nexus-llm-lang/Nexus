# Foreign Function Interface (FFI)

Nexus supports interfacing with WebAssembly (Wasm) modules, allowing developers to extend the language with functions written in languages like Rust, C, or C++.

## Loading Wasm Modules

To load a Wasm module, use the `import external` directive.

```nexus
import external math.wasm
```

## Binding External Functions

Once a module is loaded (or if the function is available in the global Wasm store), you can bind it to a Nexus name using the `external` statement.

```nexus
pub external add_ints = [=[add]=] : (a: i64, b: i64) -> i64
external internal_helper = [=[helper]=] : (x: i64) -> unit
```

- **`pub`**: Makes the binding visible to other modules. If omitted, the binding is private.
- **Name**: The name of the function in Nexus (`add_ints`).
- **Wasm Symbol**: The string literal `[=[add]=]` after `=` specifies the name of the exported function in the Wasm module.
- **Type**: The type signature after `:`. It must be an arrow type.

## Generic External Bindings

If the external function is polymorphic, type parameters must be declared explicitly with `<T, U, ...>`:

```nexus
pub external length = [=[array_length]=] : <T>(arr: &[| T |]) -> i64
```

Using an undeclared type variable (e.g., writing `T` without `<T>`) is a type error.
This prevents typos like `Strng` from silently becoming type variables.

## Supported Types

Currently, the FFI supports basic types that map directly to Wasm types:

- `i64` -> `i64`
- `float` -> `f64`
- `i32` (via `i64` casting)
- `f32` (via `float` casting)

## Example

```nexus
import external utils.wasm

external process_data = [=[process]=] : (val: float) -> float

let main = fn () -> unit do
  let result = process_data(val: 42.0)
  // assuming print_float is an effectful operation
  print_float(val: result)
  return ()
endfn
```
