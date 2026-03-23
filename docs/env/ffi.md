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

This section walks through creating FFI bindings from scratch — the Rust implementation, the Nexus declarations, and the build/bundling steps.

### Architecture Overview

An FFI binding spans three layers:

```
┌─────────────────────────────┐
│  Nexus (.nx)                │  external declarations + wrapper functions
├─────────────────────────────┤
│  WASM boundary              │  compiler auto-converts types at the edge
├─────────────────────────────┤
│  Rust (lib crate → .wasm)   │  #[no_mangle] pub extern "C" fn
└─────────────────────────────┘
```

The Nexus compiler handles the boundary layer automatically. Your job is to write matching declarations on both sides.

### Step 1: Create the Rust Crate

Create a new crate under `src/lib/`:

```
src/lib/mylib/
├── Cargo.toml
└── src/
    └── lib.rs
```

**`Cargo.toml`:**

```toml
[package]
name = "nexus_mylib_wasm"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[features]
no_alloc_export = []

[dependencies]
nexus_wasm_alloc = { path = "../wasm_alloc" }
```

Key points:

- **`cdylib`** — produces a standalone `.wasm` module with exported symbols.
- **`rlib`** — allows the crate to be linked into `stdlib_bundle`.
- **`no_alloc_export`** — when enabled, suppresses `allocate`/`deallocate` exports (the bundle exports its own single copy).
- **`nexus_wasm_alloc`** — shared allocator and string utilities. Required for any binding that handles strings or dynamic memory.

### Step 2: Write the Rust Implementation

Every exported function must be `#[no_mangle] pub extern "C"` with WASM-compatible parameter types.

#### Primitive-only Functions

The simplest case — parameters and return values are numeric primitives:

```rust
// src/lib/mylib/src/lib.rs

#[cfg(not(feature = "no_alloc_export"))]
#[no_mangle]
pub extern "C" fn allocate(size: i32) -> i32 {
    nexus_wasm_alloc::allocate(size)
}

#[cfg(not(feature = "no_alloc_export"))]
#[no_mangle]
pub unsafe extern "C" fn deallocate(ptr: i32, size: i32) {
    nexus_wasm_alloc::deallocate(ptr, size);
}

#[no_mangle]
pub extern "C" fn __nx_clamp_i64(val: i64, lo: i64, hi: i64) -> i64 {
    val.max(lo).min(hi)
}

#[no_mangle]
pub extern "C" fn __nx_is_even(val: i64) -> i32 {
    (val % 2 == 0) as i32  // bool → i32: 0 or 1
}
```

Type mapping at the Rust boundary:

| Nexus Type | Rust Parameter | Notes |
|---|---|---|
| `i64` | `i64` | Direct |
| `i32` | `i32` | Direct |
| `float` / `f64` | `f64` | Direct |
| `f32` | `f32` | Direct |
| `bool` | `i32` (param) / `i32` (return) | 0 = false, 1 = true |
| `string` | `i32, i32` (ptr, len) | See below |
| `unit` | (no param) / (no return) | Omit |

#### Functions That Receive Strings

At the FFI boundary, the compiler unpacks a Nexus `string` (internally packed `i64`) into two `i32` parameters: a pointer and a byte length.

```rust
#[no_mangle]
pub extern "C" fn __nx_my_strlen(ptr: i32, len: i32) -> i64 {
    // Validate pointer and get safe (offset, length)
    let Some((offset, len)) = nexus_wasm_alloc::checked_ptr_len(ptr, len) else {
        return 0;
    };
    // Read UTF-8 bytes from WASM linear memory
    let bytes = unsafe { std::slice::from_raw_parts(offset as *const u8, len) };
    let s = std::str::from_utf8(bytes).unwrap_or("");
    s.chars().count() as i64
}
```

- **Always** validate with `checked_ptr_len` before dereferencing — it checks bounds against WASM memory size.
- Use `read_string(ptr, len)` as a convenience to get an owned `String`.

#### Functions That Return Strings

Return strings as a packed `i64` using `store_string_result`:

```rust
#[no_mangle]
pub extern "C" fn __nx_my_repeat(ptr: i32, len: i32, n: i64) -> i64 {
    let s = nexus_wasm_alloc::read_string(ptr, len);
    let result = s.repeat(n as usize);
    nexus_wasm_alloc::store_string_result(result)
}
```

`store_string_result` allocates memory, copies the string bytes, and returns the packed `(offset << 32) | length` value that Nexus expects.

#### Stateful Bindings (Handle Pattern)

For resources that live across calls (data structures, file handles, connections), use the **handle pattern**: Rust holds the actual state in `thread_local!` storage, keyed by an `i64` handle ID.

```rust
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

thread_local! {
    static COUNTERS: RefCell<HashMap<i64, i64>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<i64> = Cell::new(1);
}

#[no_mangle]
pub extern "C" fn __nx_counter_new(initial: i64) -> i64 {
    NEXT_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        COUNTERS.with(|c| c.borrow_mut().insert(id, initial));
        id
    })
}

#[no_mangle]
pub extern "C" fn __nx_counter_inc(id: i64) -> i64 {
    COUNTERS.with(|c| {
        let mut map = c.borrow_mut();
        if let Some(val) = map.get_mut(&id) {
            *val += 1;
            *val
        } else {
            0
        }
    })
}

#[no_mangle]
pub extern "C" fn __nx_counter_free(id: i64) -> i32 {
    COUNTERS.with(|c| c.borrow_mut().remove(&id).is_some()) as i32
}
```

Why `thread_local!`: WASM is single-threaded per instance. Thread-local storage is safe and avoids the overhead of `Mutex` for the common case. (The `conc` runtime spawns separate WASM instances per thread, each with their own thread-local state.)

### Step 3: Build the WASM Module

```bash
cd src/lib/mylib
cargo build --target wasm32-unknown-unknown --release
```

The output is at `target/wasm32-unknown-unknown/release/nexus_mylib_wasm.wasm`. Copy it to wherever your Nexus source can reference it (e.g., alongside your `.nx` files).

### Step 4: Write the Nexus Declarations

Create a `.nx` module that declares the external bindings and wraps them in a user-friendly API.

#### Primitive Bindings

```nexus
import external "mylib.wasm"

export external clamp = "__nx_clamp_i64" : (val: i64, lo: i64, hi: i64) -> i64
export external is_even = "__nx_is_even" : (val: i64) -> bool
```

Direct and simple — the Nexus types map 1:1 to the WASM types.

#### String Bindings

```nexus
import external "mylib.wasm"

export external my_strlen = "__nx_my_strlen" : (s: string) -> i64
export external repeat = "__nx_my_repeat" : (s: string, n: i64) -> string
```

Even though the Rust side takes `(ptr: i32, len: i32)` for a string, you declare `(s: string)` on the Nexus side. **The compiler handles the split automatically.** This is the key asymmetry:

| Direction | Nexus declaration | Rust signature |
|---|---|---|
| String param | `(s: string)` | `(ptr: i32, len: i32)` |
| String return | `-> string` | `-> i64` (packed) |

#### Opaque Type Bindings (Stateful Resources)

For handle-based resources, wrap the raw `i64` handle in an `opaque type` with linear ownership:

```nexus
import external "mylib.wasm"

export opaque type Counter = Counter(id: i64)

external __nx_counter_new = "__nx_counter_new" : (initial: i64) -> i64
external __nx_counter_inc = "__nx_counter_inc" : (id: i64) -> i64
external __nx_counter_free = "__nx_counter_free" : (id: i64) -> bool

/// Creates a new counter with the given initial value.
export let new = fn (initial: i64) -> %Counter do
  let id = __nx_counter_new(initial: initial)
  let c = Counter(id: id)
  let %lc = c
  return %lc
end

/// Increments the counter. Consumes and returns the handle.
export let inc = fn (counter: %Counter) -> { value: i64, counter: %Counter } do
  let Counter(id: id) = counter
  let val = __nx_counter_inc(id: id)
  let c = Counter(id: id)
  let %lc = c
  return { value: val, counter: %lc }
end

/// Reads the current value without consuming the handle.
export let value = fn (counter: &Counter) -> i64 do
  let Counter(id: id) = counter
  return __nx_counter_inc(id: id)  // or a dedicated __nx_counter_get
end

/// Frees the counter. Consumes the linear handle.
export let free = fn (counter: %Counter) -> unit do
  let Counter(id: id) = counter
  let _ = __nx_counter_free(id: id)
  return ()
end
```

The key patterns:

- **`opaque type`** — hides the internal constructor from importers. Only this module can construct/destructure `Counter`.
- **`%Counter`** (linear) — the type system enforces that every counter is eventually freed. You cannot silently drop it.
- **`&Counter`** (borrow) — read-only access without consuming the handle. Use for query operations.
- **Consume-and-return** — mutating operations destructure the handle, perform the FFI call, then reconstruct and return it. This preserves linear ownership.

### Step 5: Bundle into stdlib (Optional)

To include your crate in the standard library bundle:

**1. Add the dependency to `src/lib/stdlib_bundle/Cargo.toml`:**

```toml
[dependencies]
nexus_mylib_wasm = { path = "../mylib", features = ["no_alloc_export"] }
```

The `no_alloc_export` feature prevents duplicate `allocate`/`deallocate` symbols — the bundle exports its own.

**2. Force-link the crate in `src/lib/stdlib_bundle/src/lib.rs`:**

```rust
extern crate nexus_mylib_wasm;
```

**3. Update the Nexus import path to use the bundle:**

```nexus
import external "stdlib/stdlib.wasm"

external __nx_counter_new = "__nx_counter_new" : (initial: i64) -> i64
```

**4. Rebuild the bundle:**

```bash
cd src/lib/stdlib_bundle
cargo build --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/nexus_stdlib_bundle.wasm \
   nxlib/stdlib/stdlib.wasm
```

### Naming Conventions

- **WASM export names**: prefix with `__nx_` to avoid collisions (e.g., `__nx_hset_new`).
- **Nexus binding names**: use the `__nx_`-prefixed name as an internal binding, then wrap with a clean public API function.
- **Crate names**: `nexus_<module>_wasm` (e.g., `nexus_math_wasm`, `nexus_collection_wasm`).

### Common Pitfalls

**String param count mismatch.** If your Rust function takes `(ptr: i32, len: i32)` for a string but you declare two separate `i32` params in Nexus instead of `string`, the compiler won't unpack correctly. Always use `string` in the Nexus declaration — the compiler generates the two-parameter split.

**Forgetting `allocate`/`deallocate` exports.** Standalone WASM modules (not bundled) must export `allocate` and `deallocate`. Without them, the runtime cannot manage memory for string returns. Include the `#[cfg(not(feature = "no_alloc_export"))]` boilerplate.

**Returning strings without `store_string_result`.** If you return a raw pointer instead of a packed `i64`, the Nexus runtime will interpret it as a corrupted string. Always use `store_string_result` for string returns.

**Linear handle leaks.** If your Nexus API takes `%Handle` but a code path doesn't consume it (no destructure + free), the typechecker will reject the program. This is by design — every linear resource must be explicitly freed or returned.

**Memory allocator conflict.** If your module is bundled with stdlib, all heap allocations go through stdlib's `allocate`. Do not maintain a separate allocator — use `nexus_wasm_alloc` exclusively.
