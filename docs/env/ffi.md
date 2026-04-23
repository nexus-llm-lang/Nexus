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

---

## How to Write Bindings

A WASM module that exports functions with the correct signatures can be used from Nexus via `external` declarations. This section documents the ABI contract that the WASM module must satisfy, the Nexus-side declaration patterns, and how the compiler transforms types at the boundary.

### FFI Parameter Encoding

The compiler transforms certain Nexus types when crossing the FFI boundary. Internal calls use packed representations, but external calls unpack them:

| Nexus Type | WASM Signature (external) | Notes |
|---|---|---|
| `i64` | 1x `i64` | Direct |
| `i32` | 1x `i32` | Direct |
| `float` / `f64` | 1x `f64` | Direct |
| `f32` | 1x `f32` | Direct |
| `bool` | 1x `i32` | 0 = false, 1 = true |
| `string` | 2x `i32` (ptr, len) | Unpacked from internal `i64` |
| `unit` | (none) | No parameter generated |

**String parameters** are the critical case. Internally, Nexus represents strings as a packed `i64` (`(offset << 32) | length`). At the FFI boundary, the compiler automatically unpacks this into two `i32` arguments — a pointer into linear memory and a byte length. The WASM export must accept these two `i32`s, not a single `i64`.

**String return values** go the other direction: the WASM export returns a packed `i64` using the same `(offset << 32) | length` encoding. The caller is responsible for allocating memory and writing UTF-8 bytes into linear memory before packing the result.

**Bool values** are encoded as `i32` in both directions — `0` for false, `1` for true.

### Labeled Argument Reordering

Nexus uses labeled (named) arguments, but WASM functions are positional. The compiler converts labeled arguments to positional parameters **sorted by label name** (lexicographic order). This matters when the WASM export's parameter order must match.

For example:

```nexus
external write = "write_buf" : (content: string, offset: i64) -> i64
```

The WASM signature for `write_buf` will be `(i32, i32, i64) -> i64` — the `content` string (unpacked to ptr + len) comes before `offset`, because `"content" < "offset"` lexicographically.

If you declare:

```nexus
external send = "send_msg" : (to: i64, msg: string) -> bool
```

The WASM signature is `(i32, i32, i64) -> i32` — `msg` (→ ptr, len) before `to`, because `"msg" < "to"`.

When writing a WASM module, order your export's parameters alphabetically by the label names used in the Nexus declaration.

### WASM Module Requirements

A WASM module used via FFI must:

1. **Export named functions** matching the WASM names in `external` declarations.
2. **Use the correct parameter encoding** as described above — especially the string split.
3. **Share linear memory** with the Nexus caller. String pointers reference offsets in this shared memory.
4. **Export `allocate(i32) -> i32`** if the module returns strings or allocates memory that the caller reads. The Nexus runtime calls this to allocate space for data that crosses the boundary.

### Declaring Bindings

#### Primitive Functions

When the WASM export uses only numeric types, the declaration is straightforward:

```nexus
import external "mylib.wasm"

export external clamp = "clamp_i64" : (val: i64, lo: i64, hi: i64) -> i64
export external is_even = "is_even" : (val: i64) -> bool
```

The Nexus name (left of `=`) and the WASM export name (string literal) are independent.

#### String Functions

Declare `string` on the Nexus side — the compiler generates the two-parameter split:

```nexus
import external "mylib.wasm"

external char_count = "char_count" : (s: string) -> i64
external repeat = "str_repeat" : (s: string, n: i64) -> string
```

A WASM export for `char_count` must have the signature `(i32, i32) -> i64`, and `str_repeat` must have `(i32, i32, i64) -> i64`. You never declare the split manually.

#### Wrapping with Opaque Types

For stateful resources backed by handles, wrap the raw `i64` in an `opaque type` with linear ownership:

```nexus
import external "mylib.wasm"

export opaque type Counter = Counter(id: i64)

external __counter_new = "counter_new" : (initial: i64) -> i64
external __counter_inc = "counter_inc" : (id: i64) -> i64
external __counter_free = "counter_free" : (id: i64) -> bool

/// Creates a new counter with the given initial value.
export let new = fn (initial: i64) -> %Counter do
  let id = __counter_new(initial: initial)
  let c = Counter(id: id)
  let %lc = c
  return %lc
end

/// Increments the counter. Consumes and returns the handle.
export let inc = fn (counter: %Counter) -> { value: i64, counter: %Counter } do
  let Counter(id: id) = counter
  let val = __counter_inc(id: id)
  let c = Counter(id: id)
  let %lc = c
  return { value: val, counter: %lc }
end

/// Reads the current value without consuming the handle.
export let value = fn (counter: &Counter) -> i64 do
  let Counter(id: id) = counter
  return __counter_inc(id: id)
end

/// Frees the counter. Consumes the linear handle.
export let free = fn (counter: %Counter) -> unit do
  let Counter(id: id) = counter
  let _ = __counter_free(id: id)
  return ()
end
```

The patterns at work:

- **`opaque type`** — hides the constructor from importers. Only this module can construct/destructure `Counter`.
- **`%Counter`** (linear) — the type system enforces that every counter is eventually freed. You cannot silently drop it.
- **`&Counter`** (borrow) — read-only access without consuming the handle.
- **Consume-and-return** — mutating operations destructure the handle, call the FFI function, then reconstruct and return. This preserves linear ownership across the boundary.

The WASM module is responsible for managing the actual state behind the handle (e.g., an ID-keyed table). Nexus only sees the `i64` handle value.

### Organizing Bindings — One Wrapper File per WASM Module

**Keep every `import external "foo.wasm"` and its `external` declarations confined to a single `.nx` file** — a thin wrapper module — and have all other code depend on that wrapper instead of re-declaring the bindings.

```
stdlib/
  array.nx          // import external "array.wasm" + external __nx_* + typed wrappers
  string.nx         // import external "string.wasm" + external __nx_* + typed wrappers
  ...
app/
  main.nx           // import * as arr from "stdlib/array.nx"  — no `external` here
```

Why this matters:

- **Single source of truth for the ABI.** Parameter order, encoding, and WASM export names are fragile (see [Labeled Argument Reordering](#labeled-argument-reordering)). Declaring the same binding in two files invites them to drift.
- **Linear/borrow discipline lives in the wrapper.** Raw `external` functions traffic in plain `i64` handles; the wrapper is where `%T` / `&T` / opaque types re-establish safety. Callers should never see the raw `__nx_*` form.
- **`wasm-merge` inlining is per-module.** One wrapper file means one place where the `.wasm` blob is linked, avoiding duplicate symbol work at build time.

Rule of thumb: if a non-wrapper `.nx` file contains the keyword `external`, that's a smell — extract the bindings into a dedicated wrapper module.

### Linking

The compiler resolves `import external "mylib.wasm"` at build time via `wasm-merge`. The referenced `.wasm` file path is relative to the importing `.nx` file. After merging, the final binary has no unresolved imports — all external functions are inlined.

See [WASM and WASI](wasm.md) for details on memory layout, the allocator protocol, and the full ABI specification.
