# Standard Library

Nexus provides a set of built-in functions available in the global scope.

## I/O Functions

Functions that perform Input/Output operations have the `IO` effect.

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

## Resource Management

Functions for managing linear resources.

### `drop_i64`

Explicitly consumes and drops a linear integer.

```nexus
fn drop_i64(val: %i64) -> unit
```

### `drop_array`

Explicitly consumes and drops a linear array.

```nexus
fn drop_array<T>(arr: [| T |]) -> unit
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
