# Nexus stdlib.wasm (Rust source)

This directory contains the Rust source for `nxlib/stdlib/stdio.wasm`.

## Build

```sh
rustup target add wasm32-wasip1
cargo build --manifest-path src/lib/stdlib/Cargo.toml --target wasm32-wasip1 --release
cp src/lib/stdlib/target/wasm32-wasip1/release/nexus_stdlib_wasm.wasm nxlib/stdlib/stdio.wasm
```

## Exported symbols

- `allocate(size: i32) -> i32`
- `print(ptr: i32, len: i32) -> ()`
- `i64_to_string(val: i64) -> i64`
- `float_to_string(val: f64) -> i64`
- `bool_to_string(val: i32) -> i64`
- `array_length(ptr: i32, len: i32) -> i64`

For `*_to_string`, the return ABI is packed:

- upper 32 bits: pointer
- lower 32 bits: length

`array_length` operates on array metadata passed from the interpreter.

`drop` is a language statement handled by the interpreter, not a wasm export.
