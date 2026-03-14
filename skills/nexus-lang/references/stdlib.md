# Nexus Standard Library Reference

All stdlib modules are in `stdlib/` and imported with `import ... from stdlib/<module>.nx`.

## I/O Modules (Capability-Gated)

### Console — `stdlib/stdio.nx`
**Requires**: `PermConsole` (`--allow-console`)
**Port**: `Console` | **Handler**: `system_handler`

```nexus
import { Console }, * as stdio from stdlib/stdio.nx

Console.print(val: string) -> unit        // print without newline
Console.println(val: string) -> unit      // print with newline
Console.read_line() -> string             // read line from stdin
Console.getchar() -> string               // read single char
```

### Filesystem — `stdlib/fs.nx`
**Requires**: `PermFs` (`--allow-fs`)
**Port**: `Fs` | **Handler**: `system_handler`
**Linear type**: `Handle = Handle(id: i64)`

```nexus
import { Fs, Handle }, * as fs_mod from stdlib/fs.nx

// Direct file operations
Fs.exists(path: string) -> bool
Fs.is_file(path: string) -> bool
Fs.read_to_string(path: string) -> string throws { Exn }
Fs.write_string(path: string, content: string) -> unit throws { Exn }
Fs.append_string(path: string, content: string) -> unit throws { Exn }
Fs.remove_file(path: string) -> unit throws { Exn }
Fs.create_dir_all(path: string) -> unit throws { Exn }
Fs.list_dir(path: string) -> [ string ] throws { Exn }

// Handle-based operations (linear resource)
Fs.open_read(path: string) -> Handle throws { Exn }
Fs.open_write(path: string) -> Handle throws { Exn }
Fs.open_append(path: string) -> Handle throws { Exn }
Fs.read(handle: Handle) -> string throws { Exn }    // consumes handle
Fs.write(handle: Handle, content: string) -> unit throws { Exn }
Fs.handle_path(handle: &Handle) -> string
Fs.close(handle: Handle) -> unit                     // consumes handle
```

### Network — `stdlib/net.nx`
**Requires**: `PermNet` (`--allow-net`)
**Port**: `Net` | **Handler**: `system_handler`

```nexus
import { Net, Header, Response, Server, Request, request_method, request_path, request_body }, * as net_mod from stdlib/net.nx

// HTTP Client
Net.get(url: string) -> Response throws { Exn }
Net.post(url: string, body: string) -> Response throws { Exn }
Net.request_raw(method: string, url: string, body: string, headers: [ Header ]) -> Response throws { Exn }
Net.request(method: string, url: string, body: string) -> Response throws { Exn }

// HTTP Server (Server is linear)
Net.listen(addr: string) -> Server throws { Exn }
Net.accept(server: &Server) -> Request throws { Exn }
Net.respond(req: Request, status: i64, body: string) -> unit throws { Exn }
Net.respond_with_headers(req: Request, status: i64, body: string, headers: [ Header ]) -> unit throws { Exn }
Net.stop(server: Server) -> unit                     // consumes server

// Helpers (pure functions, no port)
header(name: string, value: string) -> Header
response_status(resp: &Response) -> i64
response_headers(resp: &Response) -> [ Header ]
response_body(resp: &Response) -> string
request_method(req: &Request) -> string
request_path(req: &Request) -> string
request_body(req: &Request) -> string
```

### Random — `stdlib/random.nx`
**Requires**: `PermRandom` (`--allow-random`)
**Port**: `Random` | **Handler**: `system_handler`

```nexus
import { Random }, * as rand_mod from stdlib/random.nx

Random.next_i64() -> i64
Random.range(min: i64, max: i64) -> i64
Random.next_bool() -> bool
```

### Clock — `stdlib/clock.nx`
**Requires**: `PermClock` (`--allow-clock`)
**Port**: `Clock` | **Handler**: `system_handler`

```nexus
import { Clock }, * as clock_mod from stdlib/clock.nx

Clock.sleep(ms: i64) -> unit
Clock.now() -> i64                // milliseconds since epoch
```

### Process — `stdlib/proc.nx`
**Requires**: `PermProc` (`--allow-proc`)
**Port**: `Proc` | **Handler**: `system_handler`

```nexus
import { Proc }, * as proc_mod from stdlib/proc.nx

Proc.exit(status: i64) -> unit
```

### Environment — `stdlib/env.nx`
**Requires**: `PermEnv` (`--allow-env`)
**Port**: `Env` | **Handler**: `system_handler`

```nexus
import { Env }, * as env_mod from stdlib/env.nx

Env.get(key: string) -> Option<string>
Env.set(key: string, value: string) -> unit
```

---

## Data Structure Modules (Pure)

### Option — `stdlib/option.nx`

```nexus
import { Option, Some, None } from stdlib/option.nx
import * as option from stdlib/option.nx

type Option<T> = Some(val: T) | None

option.is_some(opt: Option<T>) -> bool
option.is_none(opt: Option<T>) -> bool
option.unwrap_or(opt: Option<T>, default: T) -> T
option.map(opt: Option<T>, f: (val: T) -> U) -> Option<U>
option.and_then(opt: Option<T>, f: (val: T) -> Option<U>) -> Option<U>
option.or_else(opt: Option<T>, other: Option<T>) -> Option<T>
option.unwrap(opt: Option<T>) -> T throws { Exn }
option.expect(opt: Option<T>, msg: string) -> T throws { Exn }
```

### Result — `stdlib/result.nx`

```nexus
import { Result, Ok, Err } from stdlib/result.nx
import * as result from stdlib/result.nx

type Result<T, E> = Ok(val: T) | Err(err: E)

result.is_ok(res: Result<T, E>) -> bool
result.is_err(res: Result<T, E>) -> bool
result.unwrap_or(res: Result<T, E>, default: T) -> T
result.map(res: Result<T, E>, f: (val: T) -> U) -> Result<U, E>
result.map_err(res: Result<T, E>, f: (err: E) -> F) -> Result<T, F>
result.and_then(res: Result<T, E>, f: (val: T) -> Result<U, E>) -> Result<U, E>
result.from_exn(exn: Exn) -> Result<T, Exn>
result.to_exn(res: Result<T, Exn>) -> T throws { Exn }
```

### List — `stdlib/list.nx`

```nexus
import * as list from stdlib/list.nx

type List<T> = Nil | Cons(v: T, rest: List<T>)
// Alias: [ T ]

list.empty() -> [ T ]
list.cons(x: T, xs: [ T ]) -> [ T ]
list.is_empty(xs: [ T ]) -> bool
list.length(xs: [ T ]) -> i64
list.head(xs: [ T ]) -> T                          // diverges on Nil
list.tail(xs: [ T ]) -> [ T ]                      // diverges on Nil
list.last(xs: [ T ]) -> T                          // diverges on Nil
list.reverse(xs: [ T ]) -> [ T ]
list.concat(xs: [ T ], ys: [ T ]) -> [ T ]
list.take(xs: [ T ], n: i64) -> [ T ]
list.drop_n(xs: [ T ], n: i64) -> [ T ]
list.nth(xs: [ T ], n: i64) -> T                   // diverges on out-of-bounds
list.contains(xs: [ i64 ], val: i64) -> bool       // i64 only
list.fold_left(xs: [ T ], init: U, f: (acc: U, val: T) -> U) -> U
list.map_rev(xs: [ T ], f: (val: T) -> U) -> [ U ]
list.map(xs: [ T ], f: (val: T) -> U) -> [ U ]
```

### Tuple — `stdlib/tuple.nx`

```nexus
import { Pair } from stdlib/tuple.nx
import * as tuple from stdlib/tuple.nx

type Pair<A, B> = Pair(left: A, right: B)

tuple.fst(p: Pair<A, B>) -> A
tuple.snd(p: Pair<A, B>) -> B
```

### Array — `stdlib/array.nx` (Linear)

```nexus
import * as array from stdlib/array.nx

// Arrays are [| T |] — linear, mutable

array.length(arr: &[| T |]) -> i64
array.is_empty(arr: &[| T |]) -> bool
array.get(arr: &[| T |], idx: i64) -> T
array.set(arr: &[| T |], idx: i64, val: T) -> unit
array.head(arr: &[| T |]) -> T
array.last(arr: &[| T |]) -> T
array.fold_left(arr: &[| T |], init: U, f: (acc: U, val: T) -> U) -> U
array.any(arr: &[| T |], f: (val: T) -> bool) -> bool
array.all(arr: &[| T |], f: (val: T) -> bool) -> bool
array.find_index(arr: &[| T |], f: (val: T) -> bool) -> i64
array.for_each(arr: &[| T |], f: (val: T) -> unit) -> unit
array.map_in_place(arr: &[| T |], f: (val: T) -> T) -> unit
array.filter(arr: &[| T |], f: (val: T) -> bool) -> [| T |]
array.partition(arr: &[| T |], f: (val: T) -> bool) -> { matched: [| T |], rest: [| T |] }
array.zip_with(a: &[| T |], b: &[| U |], f: (a: T, b: U) -> V) -> [| V |]
array.zip(a: &[| T |], b: &[| U |]) -> [| Pair<T, U> |]
array.consume(arr: [| T |]) -> unit       // explicit linear consumption
```

### Set — `stdlib/set.nx` (Linear, Opaque)

```nexus
import { Set } from stdlib/set.nx
import * as set from stdlib/set.nx

opaque type Set = Set(id: i64)

set.empty() -> Set
set.insert(s: Set, val: i64) -> Set
set.contains(s: &Set, val: i64) -> bool
set.remove(s: Set, val: i64) -> Set
set.size(s: &Set) -> i64
set.from_list(xs: [ i64 ]) -> Set
set.to_list(s: &Set) -> [ i64 ]
set.union(a: Set, b: Set) -> Set
set.intersection(a: Set, b: Set) -> Set
set.difference(a: Set, b: Set) -> Set
set.free(s: Set) -> unit                  // explicit cleanup
```

### HashMap — `stdlib/hashmap.nx` (Linear, Opaque, i64→i64)

```nexus
import { HashMap, Lookup, Found, Missing } from stdlib/hashmap.nx
import * as hmap from stdlib/hashmap.nx

opaque type HashMap = HashMap(id: i64)
type Lookup = Found(value: i64) | Missing

hmap.empty() -> HashMap
hmap.put(m: HashMap, key: i64, value: i64) -> HashMap
hmap.get(m: &HashMap, key: i64) -> Lookup
hmap.get_or(m: &HashMap, key: i64, default: i64) -> i64
hmap.contains_key(m: &HashMap, key: i64) -> bool
hmap.remove(m: HashMap, key: i64) -> HashMap
hmap.size(m: &HashMap) -> i64
hmap.keys(m: &HashMap) -> [ i64 ]
hmap.values(m: &HashMap) -> [ i64 ]
hmap.free(m: HashMap) -> unit
```

### StringMap — `stdlib/stringmap.nx` (Linear, Opaque, string→i64)

```nexus
import { StringMap } from stdlib/stringmap.nx
import * as smap from stdlib/stringmap.nx

// Same API as HashMap but keys are string
smap.empty() -> StringMap
smap.put(m: StringMap, key: string, value: i64) -> StringMap
smap.get(m: &StringMap, key: string) -> Lookup
// ... same pattern as HashMap
smap.free(m: StringMap) -> unit
```

### ByteBuffer — `stdlib/bytebuffer.nx` (Linear, Opaque)

```nexus
import { ByteBuffer } from stdlib/bytebuffer.nx
import * as buf from stdlib/bytebuffer.nx

opaque type ByteBuffer = ByteBuffer(id: i64)

buf.empty() -> ByteBuffer
buf.push_byte(b: ByteBuffer, val: i64) -> ByteBuffer
buf.push_i32_le(b: ByteBuffer, val: i64) -> ByteBuffer
buf.push_i64_le(b: ByteBuffer, val: i64) -> ByteBuffer
buf.push_uleb128(b: ByteBuffer, val: i64) -> ByteBuffer
buf.push_sleb128(b: ByteBuffer, val: i64) -> ByteBuffer
buf.push_string(b: ByteBuffer, val: string) -> ByteBuffer
buf.push_buf(b: ByteBuffer, other: &ByteBuffer) -> ByteBuffer
buf.length(b: &ByteBuffer) -> i64
buf.get_byte(b: &ByteBuffer, idx: i64) -> i64
buf.to_string(b: &ByteBuffer) -> string
buf.write_file(b: ByteBuffer, path: string) -> unit throws { Exn }  // consumes
buf.free(b: ByteBuffer) -> unit
```

---

## Utility Modules (Pure)

### String — `stdlib/string.nx`

```nexus
import * as str from stdlib/string.nx

// Inspection
str.length(s: string) -> i64
str.contains(s: string, sub: string) -> bool
str.index_of(s: string, sub: string) -> i64          // -1 if not found
str.starts_with(s: string, prefix: string) -> bool
str.ends_with(s: string, suffix: string) -> bool
str.char_at(s: string, idx: i64) -> char
str.char_code(s: string, idx: i64) -> i64            // Unicode codepoint, -1 if OOB
str.substring(s: string, start: i64, len: i64) -> string

// Transformation
str.trim(s: string) -> string
str.to_upper(s: string) -> string
str.to_lower(s: string) -> string
str.replace(s: string, old: string, new: string) -> string
str.concat(a: string, b: string) -> string            // same as ++
str.repeat(s: string, n: i64) -> string
str.pad_left(s: string, len: i64, pad: string) -> string
str.pad_right(s: string, len: i64, pad: string) -> string
str.join(xs: [ string ], sep: string) -> string
str.split(s: string, sep: string) -> [ string ]

// Conversion
str.from_i64(val: i64) -> string
str.from_float(val: f64) -> string
str.from_bool(val: bool) -> string
str.from_char(c: char) -> string
str.from_char_code(code: i64) -> string               // Unicode codepoint → string
str.parse_i64(s: string) -> Option<i64>
```

### Math — `stdlib/math.nx`

```nexus
import * as math from stdlib/math.nx

// Integer
math.abs(n: i64) -> i64
math.max(a: i64, b: i64) -> i64
math.min(a: i64, b: i64) -> i64
math.mod_i64(a: i64, b: i64) -> i64

// Float
math.abs_float(n: f64) -> f64
math.sqrt(n: f64) -> f64
math.floor(n: f64) -> f64
math.ceil(n: f64) -> f64
math.pow(base: f64, exp: f64) -> f64
math.i64_to_float(n: i64) -> f64
math.float_to_i64(n: f64) -> i64

// Boolean
math.negate(b: bool) -> bool
```

### Exception Utilities — `stdlib/exn.nx`

```nexus
import * as exn from stdlib/exn.nx

exn.to_string(exn: Exn) -> string
exn.backtrace(exn: Exn) -> [ string ]    // frames with file:line info
```

### Char — `stdlib/char.nx`

```nexus
import * as char from stdlib/char.nx

char.ord(c: char) -> i64
char.is_upper(c: char) -> bool
char.is_lower(c: char) -> bool
char.is_alpha(c: char) -> bool
char.is_digit(c: char) -> bool
char.is_alnum(c: char) -> bool
char.is_hex_digit(c: char) -> bool
char.is_whitespace(c: char) -> bool
char.is_ident_start(c: char) -> bool
char.is_ident_char(c: char) -> bool
char.is_newline(c: char) -> bool
char.digit_value(c: char) -> i64
char.hex_digit_value(c: char) -> i64
```
