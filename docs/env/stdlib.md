# Standard Library

Import stdlib modules from `stdlib/`:

```nexus
import { Console }, * as stdio from stdlib/stdio.nx
```

## I/O Ports

I/O is capability-gated via ports. Each port has a `system_handler` that declares `require { PermX }`, propagating the permission to the caller when injected. Mock handlers without `require` need no runtime permissions.

### Console (`stdio.nx`)

Requires `PermConsole`. CLI flag: `--allow-console`.

```nexus
port Console do
    fn print(val: string) -> unit
    fn println(val: string) -> unit
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

**Port methods:**

```nexus
port Fs do
    // Query
    fn exists(path: string) -> bool
    fn read_to_string(path: string) -> string effect { Exn }

    // Mutating (raise on failure)
    fn write_string(path: string, content: string) -> unit effect { Exn }
    fn append_string(path: string, content: string) -> unit effect { Exn }
    fn remove_file(path: string) -> unit effect { Exn }
    fn create_dir_all(path: string) -> unit effect { Exn }
    fn read_dir(path: string) -> List<Handle> effect { Exn }

    // File descriptor operations (consume-and-return pattern)
    fn open_read(path: string) -> %Handle effect { Exn }
    fn open_write(path: string) -> %Handle effect { Exn }
    fn open_append(path: string) -> %Handle effect { Exn }
    fn read(handle: %Handle) -> { content: string, handle: %Handle }
    fn fd_write(handle: %Handle, content: string) -> { ok: bool, handle: %Handle }
    fn fd_path(handle: %Handle) -> { path: string, handle: %Handle }
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
type Response = Response(status: i64, body: string)
opaque type Server = Server(id: i64)     // linear server handle
type Request = Request(method: string, path: string, headers: string, body: string, req_id: i64)
```

**Port methods:**

```nexus
port Net do
    // HTTP client (all raise on failure)
    fn get(url: string) -> string effect { Exn }
    fn request(method: string, url: string, headers: List<Header>) -> string effect { Exn }
    fn request_with_body(method: string, url: string, headers: List<Header>, body: string) -> string effect { Exn }
    fn request_response(method: string, url: string, headers: List<Header>, body: string) -> Response effect { Exn }

    // HTTP server
    fn listen(addr: string) -> %Server effect { Exn }
    fn accept(server: &Server) -> Request
    fn respond(req: Request, status: i64, body: string) -> unit effect { Exn }
    fn respond_with_headers(req: Request, status: i64, headers: List<Header>, body: string) -> unit effect { Exn }
    fn stop(server: %Server) -> unit
end
```

**Helper functions:**

```nexus
fn header(name: string, value: string) -> Header
fn response_status(res: Response) -> i64
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

```nexus
port Proc do
    fn exit(status: i64) -> unit
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

`Env.get` returns `None()` when the variable is not set, avoiding exceptions for simple absence.

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
fn unwrap<T>(opt: Option<T>) -> T effect { Exn }
fn expect<T>(opt: Option<T>, msg: string) -> T effect { Exn }
```

### List (`list.nx`)

Immutable singly-linked list: `type List<T> = Nil | Cons(v: T, rest: List<T>)`

```nexus
type Partition<T> = Partition(matched: List<T>, rest: List<T>)

fn empty<T>() -> List<T>
fn cons<T>(x: T, xs: List<T>) -> List<T>
fn is_empty<T>(xs: List<T>) -> bool
fn length<T>(xs: List<T>) -> i64
fn head<T>(xs: List<T>) -> T
fn tail<T>(xs: List<T>) -> List<T>
fn last<T>(xs: List<T>) -> T
fn reverse<T>(xs: List<T>) -> List<T>
fn concat<T>(xs: List<T>, ys: List<T>) -> List<T>
fn take<T>(xs: List<T>, n: i64) -> List<T>
fn drop_n<T>(xs: List<T>, n: i64) -> List<T>
fn nth<T>(xs: List<T>, n: i64) -> T
fn contains(xs: List<i64>, val: i64) -> bool
fn fold_left<T, U>(xs: List<T>, init: U, f: (acc: U, val: T) -> U) -> U
fn map<T, U>(xs: List<T>, f: (val: T) -> U) -> List<U>
fn map_rev<T, U>(xs: List<T>, f: (val: T) -> U) -> List<U>
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
fn filter<T>(arr: &[| T |], pred: (val: T) -> bool) -> List<T>
fn partition<T>(arr: &[| T |], pred: (val: T) -> bool) -> Partition<T>
fn zip_with<A, B, C>(left: &[| A |], right: &[| B |], f: (left: A, right: B) -> C) -> List<C>
fn zip<A, B>(left: &[| A |], right: &[| B |]) -> List<Pair<A, B>>
fn consume<T>(%arr: [| T |], f: (val: %T) -> unit) -> unit
```

### Set (`set.nx`)

FFI-backed hash set of i64 values. Uses opaque linear handles backed by Rust `HashSet<i64>`.

```nexus
opaque type Set = Set(id: i64)    // linear — must be freed

fn empty() -> %Set
fn insert(set: %Set, val: i64) -> %Set
fn contains(set: &Set, val: i64) -> bool
fn remove(set: %Set, val: i64) -> %Set
fn size(set: &Set) -> i64
fn from_list(xs: List<i64>) -> %Set
fn to_list(set: &Set) -> List<i64>
fn union(left: &Set, right: &Set) -> %Set
fn intersection(left: &Set, right: &Set) -> %Set
fn difference(left: &Set, right: &Set) -> %Set
fn free(set: %Set) -> unit
```

### HashMap (`hashmap.nx`)

FFI-backed hash map from i64 keys to i64 values. Uses opaque linear handles backed by Rust `HashMap<i64, i64>`.

```nexus
opaque type HashMap = HashMap(id: i64)  // linear — must be freed
type Lookup = Found(value: i64) | Missing

fn empty() -> %HashMap
fn put(map: %HashMap, key: i64, value: i64) -> %HashMap
fn get(map: &HashMap, key: i64) -> Lookup
fn get_or(map: &HashMap, key: i64, default: i64) -> i64
fn contains_key(map: &HashMap, key: i64) -> bool
fn remove(map: %HashMap, key: i64) -> %HashMap
fn size(map: &HashMap) -> i64
fn keys(map: &HashMap) -> List<i64>
fn values(map: &HashMap) -> List<i64>
fn free(map: %HashMap) -> unit
```

## Utilities

### String (`string.nx`)

```nexus
// Inspection
fn length(s: string) -> i64
fn contains(s: string, sub: string) -> bool
fn index_of(s: string, sub: string) -> i64
fn starts_with(s: string, prefix: string) -> bool
fn ends_with(s: string, suffix: string) -> bool
fn char_at(s: string, idx: i64) -> string

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
fn join(xs: List<string>, sep: string) -> string
fn split(s: string, sep: string) -> List<string>

// Conversion
fn from_i64(val: i64) -> string
fn from_float(val: float) -> string
fn from_bool(val: bool) -> string
fn to_i64(s: string) -> i64
fn parse_i64(s: string) -> Option<i64>
```

### Math (`math.nx`)

```nexus
fn abs(val: i64) -> i64
fn max(a: i64, b: i64) -> i64
fn min(a: i64, b: i64) -> i64
fn mod_i64(a: i64, b: i64) -> i64
fn negate(val: bool) -> bool
fn abs_float(val: float) -> float
fn sqrt(val: float) -> float
fn floor(val: float) -> float
fn ceil(val: float) -> float
fn pow(base: float, exp: float) -> float
fn i64_to_float(val: i64) -> float
fn float_to_i64(val: float) -> i64
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
fn to_exn<T>(res: Result<T, Exn>) -> T effect { Exn }
```

### Exception Utilities (`exn.nx`)

```nexus
fn to_string(exn: Exn) -> string
fn backtrace(exn: Exn) -> [string]
```

`backtrace` returns call-stack frames captured at raise point (interpreter only; returns `[]` in WASM builds).

### Core (`core.nx`)

Legacy re-exports for backwards compatibility. Prefer `tuple.nx`, `list.nx`, `math.nx` for new code.

```nexus
type Pair<A, B> = Pair(left: A, right: B)
type Partition<T> = Partition(matched: List<T>, rest: List<T>)

fn fst<A, B>(p: Pair<A, B>) -> A
fn snd<A, B>(p: Pair<A, B>) -> B
fn negate(val: bool) -> bool
fn id<T>(val: T) -> T
```
