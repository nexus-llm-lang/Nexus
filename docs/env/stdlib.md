# Standard Library

Nexus provides a stdlib API available in the global scope.

Public stdlib APIs are split across:
- `nxlib/stdlib/stdio.nx` (I/O convenience APIs, backed by `nxlib/stdlib/stdio.wasm`)
- `nxlib/stdlib/core.nx` (core helpers and conversions, backed by `nxlib/stdlib/core.wasm`)
- `nxlib/stdlib/string.nx` / `nxlib/stdlib/math.nx` / `nxlib/stdlib/fs.nx` / `nxlib/stdlib/net.nx` (module-specific backends)
- collection helpers in `nxlib/stdlib/list.nx`, `nxlib/stdlib/array.nx`, `nxlib/stdlib/set.nx`, and `nxlib/stdlib/hashmap.nx`
- random utilities in `nxlib/stdlib/random.nx` (backed by `nxlib/stdlib/random.wasm`)
- core ADTs like `Result<T, E>` in `nxlib/stdlib/result.nx`

`drop` is a language statement, not a stdlib function.

## I/O Functions

Functions that perform Input/Output operations have the `IO` effect.
`stdio` should stay focused on I/O.

### `print`

Prints a string to standard output as-is (no trailing newline is added).

```nexus
pub let print = external [=[print]=] : (val: string) -> unit effect { IO }
```

## Conversions

Pure functions for converting between types.

### `i64_to_string`

Converts a 64-bit integer to its string representation.

```nexus
pub let i64_to_string = external [=[i64_to_string]=] : (val: i64) -> string
```

### `float_to_string`

Converts a 64-bit float to its string representation.

```nexus
pub let float_to_string = external [=[float_to_string]=] : (val: float) -> string
```

### `bool_to_string`

Converts a boolean to "true" or "false".

```nexus
pub let bool_to_string = external [=[bool_to_string]=] : (val: bool) -> string
```

## File System Access (`fs`)

Functions for interacting with the file system. These have the `IO` effect.

### `read_to_string`

Reads the entire contents of a file into a string.

```nexus
pub let read_to_string = external [=[read_to_string]=] : (path: string) -> string effect { IO }
```

### `write_string`

Writes a string to a file. Returns `true` on success.

```nexus
pub let write_string = external [=[write_string]=] : (path: string, content: string) -> bool effect { IO }
```

### `append_string`

Appends a string to a file. Creates the file if missing.

```nexus
pub let append_string = external [=[append_string]=] : (path: string, content: string) -> bool effect { IO }
```

### `exists`

Checks whether a file or directory exists.

```nexus
pub let exists = external [=[exists]=] : (path: string) -> bool effect { IO }
```

### `remove_file`

Removes a file.

```nexus
pub let remove_file = external [=[remove_file]=] : (path: string) -> bool effect { IO }
```

### `create_dir_all`

Creates a directory and any missing parent directories.

```nexus
pub let create_dir_all = external [=[create_dir_all]=] : (path: string) -> bool effect { IO }
```

### `read_dir`

Reads direct children of a directory and returns their names as `List<string>`.

```nexus
pub let read_dir = fn (path: string) -> List<string> effect { IO } do ... endfn
```

### Linear `Closer`

`open_*` APIs return a linear closer token (`%Closer`).
This token must be consumed exactly once (normally via `close`), so forgetting to close is a type error.

```nexus
pub type Closer = Closer(path: string, mode: i64)

pub let open_read = fn (path: string) -> %Closer effect { IO, Exn }
pub let open_write = fn (path: string) -> %Closer effect { IO, Exn }
pub let open_append = fn (path: string) -> %Closer effect { IO, Exn }
pub let close = fn (closer: %Closer) -> unit
```

`open_read` raises `RuntimeError` when the path does not exist.
`open_write`/`open_append` raise `RuntimeError` when the path cannot be opened (for example missing parent directory).

## Network Access (`net`)

Functions for network operations. These have the `Net` effect.

### `get`

Performs an HTTP GET request and returns the response body as a string.

```nexus
pub let get = fn (url: string) -> string effect { Net } do
  return perform request(method: [=[GET]=], url: url, headers: Nil())
endfn
```

## Array Helpers

### `array_length`

Returns the element count of a borrowed linear array.

```nexus
pub let array_length = fn <T>(arr: &[| T |]) -> i64 do ... endfn
```

### `array` module helpers

`nxlib/stdlib/array.nx` provides:

```nexus
array.length(arr)
array.is_empty(arr)
array.get(arr, idx)
array.set(arr, idx, val)
array.head(arr)
array.last(arr)
array.fold_left(arr, init, f)
array.any(arr, pred)
array.all(arr, pred)
array.find_index(arr, pred)
array.for_each(arr, f)
array.map_in_place(arr, f)
array.filter(arr, pred)                // -> List<T>
array.partition(arr, pred)             // -> Partition<T> { matched, rest }
array.zip_with(left, right, f)         // -> List<U> (length = min)
array.zip(left, right)                 // -> List<Pair<A, B>>
```

## Collection Modules

### `list.length`

Returns the element count of an immutable list.

```nexus
import as list from [=[nxlib/stdlib/list.nx]=]

pub let list.length = fn <T>(xs: [T]) -> i64 do ... endfn
```

### `array.length`

Returns the element count of a borrowed linear array.

```nexus
import as array from [=[nxlib/stdlib/array.nx]=]

pub let array.length = fn <T>(arr: &[| T |]) -> i64 do ... endfn
```

### `set` module

`nxlib/stdlib/set.nx` provides an immutable set with dictionary-passed key operations.

```nexus
import as set from [=[nxlib/stdlib/set.nx]=]

let ops = set.i64_key_ops()

set.empty(key_ops: ops)
set.from_list(key_ops: ops, xs)
set.to_list(set)
set.contains(set, val)
set.insert(set, val)
set.remove(set, val)
set.size(set)
set.union(left, right)
set.intersection(left, right)
set.difference(left, right)
set.make_key_ops(eq, hash)
```

### `hashmap` module

`nxlib/stdlib/hashmap.nx` provides an immutable map with dictionary-passed key operations.

```nexus
import as hashmap from [=[nxlib/stdlib/hashmap.nx]=]

let ops = hashmap.i64_key_ops()

hashmap.empty(key_ops: ops)
hashmap.put(map, key, value)
hashmap.get(map, key)           // -> Found(value: V) | Missing
hashmap.get_or(map, key, default)
hashmap.contains_key(map, key)
hashmap.remove(map, key)
hashmap.size(map)
hashmap.keys(map)               // -> List<i64>
hashmap.values(map)             // -> List<V>
hashmap.make_key_ops(eq, hash)  // custom dictionary
```

## Random

`nxlib/stdlib/random.nx` provides random number helpers.
These are effectful and require `perform`.

```nexus
import as random from [=[nxlib/stdlib/random.nx]=]

perform random.next_i64()
perform random.range(min: 0, max: 10)  // [min, max)
perform random.next_bool()
```

## Result Helpers

`Result<T, E>` is defined in `nxlib/stdlib/result.nx` with constructors:

```nexus
Ok(val: T)
Err(err: E)
```

Helper functions:

```nexus
import as result from [=[nxlib/stdlib/result.nx]=]

pub let result.is_ok = fn <T, E>(res: Result<T, E>) -> bool
pub let result.is_err = fn <T, E>(res: Result<T, E>) -> bool
pub let result.unwrap_or = fn <T, E>(res: Result<T, E>, default: T) -> T
pub let result.from_exn = fn <T>(exn: Exn) -> Result<T, Exn>
pub let result.to_exn = fn <T>(res: Result<T, Exn>) -> T effect { Exn }
```
