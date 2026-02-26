# Standard Library

Nexus stdlib APIs are provided by `nxlib/stdlib/*.nx` modules.

Core modules:

- `nxlib/stdlib/core.nx`
- `nxlib/stdlib/stdio.nx`
- `nxlib/stdlib/fs.nx`
- `nxlib/stdlib/net.nx`
- `nxlib/stdlib/string.nx`
- `nxlib/stdlib/math.nx`
- `nxlib/stdlib/list.nx`
- `nxlib/stdlib/array.nx`
- `nxlib/stdlib/set.nx`
- `nxlib/stdlib/hashmap.nx`
- `nxlib/stdlib/random.nx`
- `nxlib/stdlib/result.nx`

Primitive linear values are auto-dropped at scope end. Composite linear values must be consumed via pattern matching or function calls.

## Console (`stdio`)

```nexus
pub external print = [=[print]=] : (val: string) -> unit effect { Console }
```

## String (`string`)

### Conversions

```nexus
pub external i64_to_string = [=[__nx_i64_to_string]=] : (val: i64) -> string
pub external float_to_string = [=[__nx_float_to_string]=] : (val: float) -> string
pub external bool_to_string = [=[__nx_bool_to_string]=] : (val: bool) -> string
pub external string_to_i64 = [=[__nx_string_to_i64]=] : (s: string) -> i64
```

## File System (`fs`)

All fs operations are defined in `port Fs` and dispatched via handler.
Use `inject default_fs do ... endinject` for real filesystem access, or inject a mock handler for testing.

### Query operations

```nexus
fn exists(path: string) -> bool
fn read_to_string(path: string) -> string
```

### Mutating path-level operations

These raise `RuntimeError` on failure instead of returning `bool`:

```nexus
fn write_string(path: string, content: string) -> unit effect { Exn }
fn append_string(path: string, content: string) -> unit effect { Exn }
fn remove_file(path: string) -> unit effect { Exn }
fn create_dir_all(path: string) -> unit effect { Exn }
```

### Directory listing

`read_dir` returns a list of opened file handles (subdirectories are skipped):

```nexus
fn read_dir(path: string) -> List<Handle> effect { Exn }
```

### Stateful fd operations (consume-and-return pattern)

```nexus
pub type Handle = Handle(id: i64)   // non-opaque — any handler can construct

fn open_read(path: string) -> %Handle effect { Exn }
fn open_write(path: string) -> %Handle effect { Exn }
fn open_append(path: string) -> %Handle effect { Exn }
fn read(handle: %Handle) -> { content: string, handle: %Handle }
fn fd_write(handle: %Handle, content: string) -> { ok: bool, handle: %Handle }
fn fd_path(handle: %Handle) -> { path: string, handle: %Handle }
fn close(handle: %Handle) -> unit
```

Usage pattern:
```nexus
import { default_fs, Fs, Handle } from nxlib/stdlib/fs.nx

inject default_fs do
  try
    Fs.write_string(path: [=[data.txt]=], content: [=[hello]=])
    let %h = Fs.open_read(path: [=[data.txt]=])
    let %r = Fs.read(handle: %h)
    match %r do case { content: content, handle: %h2 } ->
      Fs.close(handle: %h2)
      // use content
    endmatch
  catch e ->
    // handle error
  endtry
endinject
```

`open_*` and mutating operations may raise `RuntimeError`. The `read`, `fd_write`, and `fd_path` operations consume the handle and return a new one in the result record, enabling stateless handlers (no borrow needed).

## Network (`net`)

`net` functions are capability-gated by `require { Net }`.

```nexus
pub type Header = Header(name: string, value: string)
pub type Response = Response(status: i64, body: string)

pub let header = fn (name: string, value: string) -> Header do ... endfn
pub let request_response = fn (method: string, url: string, headers: List<Header>, body: string) -> Response require { Net } do ... endfn
pub let request = fn (method: string, url: string, headers: List<Header>) -> string require { Net } do ... endfn
pub let request_with_body = fn (method: string, url: string, headers: List<Header>, body: string) -> string require { Net } do ... endfn
pub let get = fn (url: string) -> string require { Net } do ... endfn
```

## List and Array

`List<T>` is an ADT with `Nil()` / `Cons(v: T, rest: List<T>)`.

```nexus
import as list from nxlib/stdlib/list.nx
import as array from nxlib/stdlib/array.nx

list.length(xs)
list.fold_left(xs, init, f)
list.map_rev(xs, f)
list.map(xs, f)

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
array.filter(arr, pred)
array.partition(arr, pred)
array.zip_with(left, right, f)
array.zip(left, right)
```

## Set and HashMap

Both collections use dictionary-passed key operations (`eq`/`hash`).

```nexus
import as set from nxlib/stdlib/set.nx
import as hashmap from nxlib/stdlib/hashmap.nx

let key_ops = set.make_key_ops(eq: eq_fn, hash: hash_fn)
let s0 = set.empty(key_ops: key_ops)
let s1 = set.insert(set: s0, val: 10)

let map_ops = hashmap.make_key_ops(eq: eq_fn, hash: hash_fn)
let m0 = hashmap.empty(key_ops: map_ops)
let m1 = hashmap.put(map: m0, key: 1, value: [=[one]=])
```

## Random

`random` helpers are effectful (`effect { Console }`):

```nexus
import as random from nxlib/stdlib/random.nx

random.next_i64()
random.range(min: 0, max: 10)  // [min, max)
random.next_bool()
```

## Result Helpers

`result.nx` provides helpers and Exn bridges.

```nexus
import as result from nxlib/stdlib/result.nx

result.is_ok(res)
result.is_err(res)
result.unwrap_or(res, default)
result.from_exn(exn)
result.to_exn(res) // effect { Exn }
```
