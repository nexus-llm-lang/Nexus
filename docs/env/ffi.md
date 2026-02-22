# Foreign Function Interface (FFI)

Nexus supports interfacing with WebAssembly (Wasm) modules, allowing developers to extend the language with functions written in languages like Rust, C, or C++.

## Loading Wasm Modules

To load a Wasm module, use the `import external` directive.

```nexus
import external [=[math.wasm]=]
```

## Binding External Functions

Once a module is loaded (or if the function is available in the global Wasm store), you can bind it to a Nexus name using the `external` expression within a `let` binding.

```nexus
pub let add_ints = external [=[add]=] : (a: i64, b: i64) -> i64
let internal_helper = external [=[helper]=] : (x: i64) -> unit
```

- **`pub let`**: Makes the binding visible to other modules. If omitted, the binding is private.
- **Name**: The name of the function in Nexus (`add_ints`).
- **Wasm Symbol**: The string literal `[=[add]=]` specifies the name of the exported function in the Wasm module.
- **Type**: The type signature. It must be an arrow type.

## Supported Types

Currently, the FFI supports basic types that map directly to Wasm types:

- `i64` -> `i64`
- `float` -> `f64`
- `i32` (via `i64` casting)
- `f32` (via `float` casting)

## Example

```nexus
import external [=[utils.wasm]=]

let process_data = external [=[process]=] : (val: float) -> float

let main = fn () -> unit do
  let result = process_data(val: 42.0)
  // assuming print_float is an effectful operation
  perform print_float(val: result)
  return ()
endfn
```
