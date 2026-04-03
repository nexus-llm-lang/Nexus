---
layout: default
title: Standard Library
---

# Standard Library

Import stdlib modules from `stdlib/`:

```nexus
import { Console }, * as stdio from "stdlib/stdio.nx"
```

## I/O Ports

I/O is capability-gated via ports. Each port has a `system_handler` that declares `require { PermX }`, propagating the permission to the caller when injected. Mock handlers without `require` need no runtime permissions.

### Console (`stdio.nx`)

Requires `PermConsole`. CLI flag: `--allow-console`.

```nexus
port Console do
  fn print(val: string) -> unit
  fn println(val: string) -> unit
  fn eprint(val: string) -> unit
  fn eprintln(val: string) -> unit
  fn read_line() -> string
  fn getchar() -> string
end
```

```nexus
let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "Hello")
  end
  return ()
end
```

### File System (`fs.nx`)

Requires `PermFs`. CLI flag: `--allow-fs`.

**Types:**

```nexus
type Handle = Handle(id: i64)   // linear file handle
```

**Direct-call API** (no `inject` required):

```nexus
fn exists(path: string) -> bool require { PermFs }
fn is_file(path: string) -> bool require { PermFs }
fn read_to_string(path: string) -> string require { PermFs } throws { Exn }
fn write_string(path: string, content: string) -> unit require { PermFs } throws { Exn }
fn append_string(path: string, content: string) -> unit require { PermFs } throws { Exn }
fn remove_file(path: string) -> unit require { PermFs } throws { Exn }
fn create_dir_all(path: string) -> unit require { PermFs } throws { Exn }
fn list_dir(path: string) -> [ string ] require { PermFs } throws { Exn }
fn open_read(path: string) -> %Handle require { PermFs } throws { Exn }
fn open_write(path: string) -> %Handle require { PermFs } throws { Exn }
fn open_append(path: string) -> %Handle require { PermFs } throws { Exn }
fn read(handle: %Handle) -> { content: string, handle: %Handle } require { PermFs }
fn write(handle: %Handle, content: string) -> { ok: bool, handle: %Handle } require { PermFs }
fn handle_path(handle: %Handle) -> { path: string, handle: %Handle } require { PermFs }
fn close(handle: %Handle) -> unit require { PermFs }
```

**Port methods** (for DI / testing):

```nexus
port Fs do
  // Query
  fn exists(path: string) -> bool
  fn read_to_string(path: string) -> string throws { Exn }

  // Mutating (raise on failure)
  fn write_string(path: string, content: string) -> unit throws { Exn }
  fn append_string(path: string, content: string) -> unit throws { Exn }
  fn remove_file(path: string) -> unit throws { Exn }
  fn create_dir_all(path: string) -> unit throws { Exn }
  fn read_dir(path: string) -> %[ Handle ] throws { Exn }

  // File descriptor operations (consume-and-return pattern)
  fn open_read(path: string) -> %Handle throws { Exn }
  fn open_write(path: string) -> %Handle throws { Exn }
  fn open_append(path: string) -> %Handle throws { Exn }
  fn read(handle: %Handle) -> { content: string, handle: %Handle }
  fn write(handle: %Handle, content: string) -> { ok: bool, handle: %Handle }
  fn handle_path(handle: %Handle) -> { path: string, handle: %Handle }
  fn close(handle: %Handle) -> unit
end
```

The fd operations use a **consume-and-return** pattern: the linear handle is consumed and a fresh handle is returned in the result record, enabling stateless handlers.

```nexus
let %h = Fs.open_read(path: "data.txt")
let %r = Fs.read(handle: %h)
match %r do
  case { content: c, handle: %h2 } ->
    Fs.close(handle: %h2)
end
```

### Network (`net.nx`)

Requires `PermNet`. CLI flag: `--allow-net`.

**Types:**

```nexus
type Header = Header(name: string, value: string)
type Response = Response(status: i64, headers: string, body: string)
opaque type Server = Server(id: i64)   // linear server handle
type Request = Request(method: string, path: string, headers: string, body: string, req_id: i64)
```

**Direct-call API** (no `inject` required):

```nexus
// HTTP client (all raise on failure)
fn get(url: string) -> string require { PermNet } throws { Exn }
fn post(url: string, body: string) -> string require { PermNet } throws { Exn }
fn request_raw(method: string, url: string, headers: string, body: string) -> Response require { PermNet } throws { Exn }
fn request(method: string, url: string, headers: [ Header ], body: string) -> Response require { PermNet } throws { Exn }

// HTTP server
fn listen(addr: string) -> %Server require { PermNet } throws { Exn }
fn accept(server: &Server) -> Request require { PermNet }
fn respond(req: Request, status: i64, body: string) -> unit require { PermNet } throws { Exn }
fn respond_with_headers(req: Request, status: i64, headers: [ Header ], body: string) -> unit require { PermNet } throws { Exn }
fn stop(server: %Server) -> unit require { PermNet }
```

**Port methods** (for DI / testing):

```nexus
port Net do
  // HTTP client (all raise on failure)
  fn get(url: string) -> string throws { Exn }
  fn request(method: string, url: string, headers: [ Header ], body: string) -> Response throws { Exn }

  // HTTP server
  fn listen(addr: string) -> %Server throws { Exn }
  fn accept(server: &Server) -> Request
  fn respond(req: Request, status: i64, body: string) -> unit throws { Exn }
  fn respond_with_headers(req: Request, status: i64, headers: [ Header ], body: string) -> unit throws { Exn }
  fn stop(server: %Server) -> unit
end
```

**Helper functions:**

```nexus
fn header(name: string, value: string) -> Header
fn response_status(res: Response) -> i64
fn response_headers(res: Response) -> string
fn response_body(res: Response) -> string
fn request_method(req: &Request) -> string
fn request_path(req: &Request) -> string
fn request_body(req: &Request) -> string
```

### Random (`random.nx`)

Requires `PermRandom`. CLI flag: `--allow-random`.

```nexus
port Random do
  fn next_i64() -> i64
  fn range(min: i64, max: i64) -> i64
  fn next_bool() -> bool
end
```

### Clock (`clock.nx`)

Requires `PermClock`. CLI flag: `--allow-clock`.

```nexus
port Clock do
  fn sleep(ms: i64) -> unit
  fn now() -> i64
end
```

### Process (`proc.nx`)

Requires `PermProc`. CLI flag: `--allow-proc`.

**Types:**

```nexus
type ExecResult = ExecResult(exit_code: i64, stdout: string, stderr: string)
```

**Direct-call API:**

```nexus
fn argv() -> [ string ] require { PermProc }
```

**Port methods:**

```nexus
port Proc do
  fn exit(status: i64) -> unit
  fn argv() -> [ string ]
  fn exec(cmd: string, args: [ string ]) -> ExecResult
end
```

### Environment (`env.nx`)

Requires `PermEnv`. CLI flag: `--allow-env`.

```nexus
port Env do
  fn get(key: string) -> Option<string>
  fn set(key: string, value: string) -> unit
end
```

`Env.get` returns `None` when the variable is not set, avoiding exceptions for simple absence.

## Data Structures

### Option (`option.nx`)

```nexus
type Option<T> = Some(val: T) | None

fn is_some<T>(opt: Option<T>) -> bool
fn is_none<T>(opt: Option<T>) -> bool
fn unwrap_or<T>(opt: Option<T>, default: T) -> T
fn map<T, U>(opt: Option<T>, f: (val: T) -> U) -> Option<U>
fn and_then<T, U>(opt: Option<T>, f: (val: T) -> Option<U>) -> Option<U>
fn or_else<T>(opt: Option<T>, other: Option<T>) -> Option<T>
fn unwrap<T>(opt: Option<T>) -> T throws { Exn }
fn expect<T>(opt: Option<T>, msg: string) -> T throws { Exn }
```

### List (`list.nx`)

Immutable singly-linked list: `type List<T> = Nil | Cons(v: T, rest: List<T>)`.
`[ T ]` is an alias for `List<T>` with literal syntax sugar.

```nexus
type Partition<T> = Partition(matched: [ T ], rest: [ T ])

fn empty<T>() -> [ T ]
fn cons<T>(x: T, xs: [ T ]) -> [ T ]
fn is_empty<T>(xs: [ T ]) -> bool
fn length<T>(xs: [ T ]) -> i64
fn head<T>(xs: [ T ]) -> T
fn tail<T>(xs: [ T ]) -> [ T ]
fn last<T>(xs: [ T ]) -> T
fn reverse<T>(xs: [ T ]) -> [ T ]
fn concat<T>(xs: [ T ], ys: [ T ]) -> [ T ]
fn take<T>(xs: [ T ], n: i64) -> [ T ]
fn drop_n<T>(xs: [ T ], n: i64) -> [ T ]
fn nth<T>(xs: [ T ], n: i64) -> T
fn contains(xs: [ i64 ], val: i64) -> bool
fn fold_left<T, U>(xs: [ T ], init: U, f: (acc: U, val: T) -> U) -> U
fn map<T, U>(xs: [ T ], f: (val: T) -> U) -> [ U ]
fn map_rev<T, U>(xs: [ T ], f: (val: T) -> U) -> [ U ]
```

### Tuple (`tuple.nx`)

```nexus
type Pair<A, B> = Pair(left: A, right: B)

fn fst<A, B>(p: Pair<A, B>) -> A
fn snd<A, B>(p: Pair<A, B>) -> B
```

### Array (`array.nx`)

Linear mutable array: `[| T |]`

```nexus
fn length<T>(arr: &[| T |]) -> i64
fn is_empty<T>(arr: &[| T |]) -> bool
fn get<T>(arr: &[| T |], idx: i64) -> T
fn set<T>(arr: &[| T |], idx: i64, val: T) -> unit
fn head<T>(arr: &[| T |]) -> T
fn last<T>(arr: &[| T |]) -> T
fn fold_left<T, U>(arr: &[| T |], init: U, f: (acc: U, val: T) -> U) -> U
fn any<T>(arr: &[| T |], pred: (val: T) -> bool) -> bool
fn all<T>(arr: &[| T |], pred: (val: T) -> bool) -> bool
fn find_index<T>(arr: &[| T |], pred: (val: T) -> bool) -> i64
fn for_each<T>(arr: &[| T |], f: (val: T) -> unit) -> unit
fn map_in_place<T>(arr: &[| T |], f: (val: T) -> T) -> unit
fn filter<T>(arr: &[| T |], pred: (val: T) -> bool) -> [ T ]
fn partition<T>(arr: &[| T |], pred: (val: T) -> bool) -> Partition<T>
fn zip_with<A, B, C>(left: &[| A |], right: &[| B |], f: (left: A, right: B) -> C) -> [ C ]
fn zip<A, B>(left: &[| A |], right: &[| B |]) -> [ Pair<A, B> ]
fn consume<T>(%arr: [| T |], f: (val: %T) -> unit) -> unit
```

### Set (`set.nx`)

FFI-backed hash set of i64 values. Uses opaque linear handles backed by Rust `HashSet<i64>`.

```nexus
opaque type Set = Set(id: i64)  // linear -- must be freed

fn empty() -> %Set
fn insert(set: %Set, val: i64) -> %Set
fn contains(set: &Set, val: i64) -> bool
fn remove(set: %Set, val: i64) -> %Set
fn size(set: &Set) -> i64
fn from_list(xs: [ i64 ]) -> %Set
fn to_list(set: &Set) -> [ i64 ]
fn union(left: &Set, right: &Set) -> %Set
fn intersection(left: &Set, right: &Set) -> %Set
fn difference(left: &Set, right: &Set) -> %Set
fn free(set: %Set) -> unit
```

### HashMap (`hashmap.nx`)

FFI-backed hash map from i64 keys to i64 values. Uses opaque linear handles backed by Rust `HashMap<i64, i64>`.

```nexus
opaque type HashMap = HashMap(id: i64)  // linear -- must be freed
type Lookup = Found(value: i64) | Missing

fn empty() -> %HashMap
fn put(map: %HashMap, key: i64, value: i64) -> %HashMap
fn get(map: &HashMap, key: i64) -> Lookup
fn get_or(map: &HashMap, key: i64, default: i64) -> i64
fn contains_key(map: &HashMap, key: i64) -> bool
fn remove(map: %HashMap, key: i64) -> %HashMap
fn size(map: &HashMap) -> i64
fn keys(map: &HashMap) -> [ i64 ]
fn values(map: &HashMap) -> [ i64 ]
fn free(map: %HashMap) -> unit
```

### StringMap (`stringmap.nx`)

FFI-backed hash map from string keys to i64 values. Uses opaque linear handles backed by Rust `HashMap<String, i64>`.

```nexus
opaque type StringMap = StringMap(id: i64)  // linear -- must be freed
type Lookup = Found(value: i64) | Missing

fn empty() -> %StringMap
fn put(map: %StringMap, key: string, value: i64) -> %StringMap
fn get(map: &StringMap, key: string) -> Lookup
fn get_or(map: &StringMap, key: string, default: i64) -> i64
fn contains_key(map: &StringMap, key: string) -> bool
fn remove(map: %StringMap, key: string) -> %StringMap
fn size(map: &StringMap) -> i64
fn keys(map: &StringMap) -> [ string ]
fn values(map: &StringMap) -> [ i64 ]
fn free(map: %StringMap) -> unit
```

### ByteBuffer (`bytebuffer.nx`)

FFI-backed mutable byte buffer for binary data construction. Uses opaque linear handles backed by Rust `Vec<u8>`. Provides LEB128 encoding, little-endian integer writes, and raw byte/string/buffer append operations.

```nexus
opaque type ByteBuffer = ByteBuffer(id: i64)  // linear -- must be freed

fn empty() -> %ByteBuffer
fn push_byte(buf: %ByteBuffer, byte: i64) -> %ByteBuffer
fn push_i32_le(buf: %ByteBuffer, val: i64) -> %ByteBuffer
fn push_i64_le(buf: %ByteBuffer, val: i64) -> %ByteBuffer
fn push_uleb128(buf: %ByteBuffer, val: i64) -> %ByteBuffer
fn push_sleb128(buf: %ByteBuffer, val: i64) -> %ByteBuffer
fn push_string(buf: %ByteBuffer, s: string) -> %ByteBuffer
fn push_buf(dst: %ByteBuffer, src: &ByteBuffer) -> %ByteBuffer
fn length(buf: &ByteBuffer) -> i64
fn get_byte(buf: &ByteBuffer, idx: i64) -> i64
fn to_string(buf: &ByteBuffer) -> string
fn write_file(buf: &ByteBuffer, path: string) -> bool require { PermFs }
fn free(buf: %ByteBuffer) -> unit
```

All mutating operations consume the buffer and return a new handle (consume-and-return pattern).

## Utilities

### String (`string.nx`)

```nexus
// Inspection
fn length(s: string) -> i64
fn contains(s: string, sub: string) -> bool
fn index_of(s: string, sub: string) -> i64
fn starts_with(s: string, prefix: string) -> bool
fn ends_with(s: string, suffix: string) -> bool
fn char_at(s: string, idx: i64) -> char
fn char_code(s: string, idx: i64) -> i64              // Unicode codepoint, -1 if OOB

// Transformation
fn substring(s: string, start: i64, len: i64) -> string
fn trim(s: string) -> string
fn to_upper(s: string) -> string
fn to_lower(s: string) -> string
fn replace(s: string, from_str: string, to_str: string) -> string
fn concat(a: string, b: string) -> string
fn repeat(s: string, n: i64) -> string
fn pad_left(s: string, width: i64, fill: string) -> string
fn pad_right(s: string, width: i64, fill: string) -> string
fn join(xs: [ string ], sep: string) -> string
fn split(s: string, sep: string) -> [ string ]

// Conversion
fn from_i64(val: i64) -> string
fn from_float(val: float) -> string
fn from_bool(val: bool) -> string
fn from_char(c: char) -> string
fn from_char_code(code: i64) -> string                // Unicode codepoint → string
fn parse_i64(s: string) -> Option<i64>
fn parse_f64(s: string) -> Option<f64>
fn to_f64(s: string) -> f64 throws { Exn }
```

### Math (`math.nx`)

```nexus
fn abs(val: i64) -> i64
fn max(a: i64, b: i64) -> i64
fn min(a: i64, b: i64) -> i64
fn mod_i64(a: i64, b: i64) -> i64
fn abs_float(val: float) -> float
fn sqrt(val: float) -> float
fn floor(val: float) -> float
fn ceil(val: float) -> float
fn pow(base: float, exp: float) -> float
fn i64_to_float(val: i64) -> float
fn float_to_i64(val: float) -> i64
fn negate(val: bool) -> bool
```

### Result (`result.nx`)

```nexus
type Result<T, E> = Ok(val: T) | Err(err: E)

fn is_ok<T, E>(res: Result<T, E>) -> bool
fn is_err<T, E>(res: Result<T, E>) -> bool
fn unwrap_or<T, E>(res: Result<T, E>, default: T) -> T
fn map<T, U, E>(res: Result<T, E>, f: (val: T) -> U) -> Result<U, E>
fn map_err<T, E, F>(res: Result<T, E>, f: (val: E) -> F) -> Result<T, F>
fn and_then<T, U, E>(res: Result<T, E>, f: (val: T) -> Result<U, E>) -> Result<U, E>
fn from_exn<T>(exn: Exn) -> Result<T, Exn>
fn to_exn<T>(res: Result<T, Exn>) -> T throws { Exn }
```

### Exception Utilities (`exn.nx`)

```nexus
fn to_string(exn: Exn) -> string
fn backtrace(exn: Exn) -> [string]
```

`backtrace` returns call-stack frames (with source file and line info) captured at the raise point.

### Char (`char.nx`)

Character classification functions for ASCII analysis:

```nexus
fn ord(c: char) -> i64
fn is_upper(c: char) -> bool
fn is_lower(c: char) -> bool
fn is_alpha(c: char) -> bool
fn is_digit(c: char) -> bool
fn is_alnum(c: char) -> bool
fn is_hex_digit(c: char) -> bool
fn is_whitespace(c: char) -> bool
fn is_ident_start(c: char) -> bool
fn is_ident_char(c: char) -> bool
fn is_newline(c: char) -> bool
fn digit_value(c: char) -> i64
fn hex_digit_value(c: char) -> i64
```

### Lazy (`lazy.nx`)

Combinators for `@` thunk evaluation. Backed by `nexus:runtime/lazy` host functions.

```nexus
fn race(a: i64, b: i64) -> i64
fn cancel(thunk: i64) -> unit
fn detach(thunk: i64) -> unit
fn force_all(tasks: [i64]) -> [i64]
```

| Function | Description |
|---|---|
| `race(a, b)` | Force two thunks in parallel, return the first to complete; loser discarded |
| `cancel(thunk)` | Consume a thunk without evaluating (satisfies linearity) |
| `detach(thunk)` | Fire-and-forget: start evaluation, don't wait for result |
| `force_all(tasks)` | Spawn all thunks in parallel, join results in order |

Note: functions use `i64` internally (all values are i64 at WASM level). The `@T` linearity is enforced at the call site by the typechecker.

### Core (`core.nx`)

Legacy re-exports for backwards compatibility. Prefer `tuple.nx`, `list.nx`, `math.nx` for new code.

```nexus
type Pair<A, B> = Pair(left: A, right: B)
type Partition<T> = Partition(matched: [ T ], rest: [ T ])

fn fst<A, B>(p: Pair<A, B>) -> A
fn snd<A, B>(p: Pair<A, B>) -> B
fn negate(val: bool) -> bool
fn id<T>(val: T) -> T
```
