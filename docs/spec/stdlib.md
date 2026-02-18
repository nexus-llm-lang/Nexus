# Standard Library

Nexus provides a stdlib API available in the global scope.

Public stdlib APIs are split across:
- `nxlib/stdlib/stdio.nx` (I/O APIs, backed by `nxlib/stdlib/stdio.wasm`)
- `nxlib/stdlib/stdlib.nx` (pure helpers, backed by `nxlib/stdlib/stdlib.wasm`)
- collection helpers in `nxlib/stdlib/list.nx` and `nxlib/stdlib/array.nx`

`drop` is a language statement, not a stdlib function.

## I/O Functions

Functions that perform Input/Output operations have the `IO` effect.
`stdio` should stay focused on I/O.

### `print`

Prints a string to standard output, followed by a newline.

```nexus
fn print(val: string) -> unit effect { IO }
```

## Conversions

Pure functions for converting between types.

### `i64_to_string`

Converts a 64-bit integer to its string representation.

```nexus
fn i64_to_string(val: i64) -> string
```

### `float_to_string`

Converts a 64-bit float to its string representation.

```nexus
fn float_to_string(val: float) -> string
```

### `bool_to_string`

Converts a boolean to "true" or "false".

```nexus
fn bool_to_string(val: bool) -> string
```

## Array Helpers

### `array_length`

Returns the element count of a borrowed linear array.

```nexus
fn array_length<T>(arr: &[| T |]) -> i64
```

## Collection Modules

### `list.length`

Returns the element count of an immutable list.

```nexus
import as list from "nxlib/stdlib/list.nx"

fn list.length<T>(xs: [T]) -> i64
```

### `array.length`

Returns the element count of a borrowed linear array.

```nexus
import as array from "nxlib/stdlib/array.nx"

fn array.length<T>(arr: &[| T |]) -> i64
```
