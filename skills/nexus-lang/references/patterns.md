# Nexus Idiomatic Code Patterns

## Program Structure

Every executable Nexus program needs a `main` function:

```nexus
// Minimal program
let main = fn () -> unit do
  return ()
end

// With I/O
import { Console }, * as stdio from stdlib/stdio.nx

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "Hello!")
  end
  return ()
end
```

## Import Patterns

```nexus
// Import port + module alias (most common for I/O)
import { Console }, * as stdio from stdlib/stdio.nx

// Import specific items
import { Option, Some, None } from stdlib/option.nx
import { Result, Ok, Err } from stdlib/result.nx

// Import as module alias (for utility functions)
import as list from stdlib/list.nx
import as str from stdlib/string.nx
import as math from stdlib/math.nx

// Combine both
import { Net, Request, Response }, * as net_mod from stdlib/net.nx
```

## Custom Port + Handler (Dependency Injection)

```nexus
// 1. Define domain types
type User = { id: i64, name: string, email: string }

// 2. Define port (interface)
port UserRepository do
  fn find_by_id(id: i64) -> Option<User>
  fn save(user: User) -> Result<unit, string>
end

// 3. Business logic depends on port
let register = fn (name: string, email: string) -> Result<unit, string> require { UserRepository, Logger } do
  let user = { id: 0, name: name, email: email }
  Logger.info(msg: "Registering: " ++ email)
  return UserRepository.save(user: user)
end

// 4. Handler for production
let db_repo = handler UserRepository require { PermFs } do
  fn find_by_id(id: i64) -> Option<User> do
    // ... real implementation
    return None
  end
  fn save(user: User) -> Result<unit, string> do
    // ... real implementation
    return Ok(val: ())
  end
end

// 5. Handler for testing
let mock_repo = handler UserRepository do
  fn find_by_id(id: i64) -> Option<User> do
    return Some(val: { id: id, name: "Test", email: "test@test.com" })
  end
  fn save(user: User) -> Result<unit, string> do
    return Ok(val: ())
  end
end
```

## Error Handling Patterns

### Result-based (prefer for recoverable errors)

```nexus
import { Result, Ok, Err } from stdlib/result.nx
import as result from stdlib/result.nx

let parse_config = fn (raw: string) -> Result<Config, string> do
  if str.length(s: raw) == 0 then
    return Err(err: "empty config")
  else
    // ... parse
    return Ok(val: config)
  end
end

// Chaining with and_then
let load_config = fn (path: string) -> Result<Config, string> require { Fs } throws { Exn } do
  let raw = Fs.read_to_string(path: path)
  return parse_config(raw: raw)
end
```

### Exception-based (for unrecoverable or I/O errors)

```nexus
exception ConfigError(msg: string)

let load_or_die = fn (path: string) -> Config require { Fs } throws { Exn } do
  let raw = Fs.read_to_string(path: path)
  let res = parse_config(raw: raw)
  match res do
    case Ok(val: c) -> return c
    case Err(err: msg) -> raise ConfigError(msg: msg)
  end
end

// Always catch in main
let main = fn () -> unit require { PermFs, PermConsole } do
  inject fs_mod.system_handler, stdio.system_handler do
    try
      let cfg = load_or_die(path: "config.txt")
      Console.println(val: "Config loaded")
    catch e ->
      match e do
        case ConfigError(msg: m) -> Console.println(val: "Config error: " ++ m)
        case _ -> Console.println(val: "Unknown error")
      end
    end
  end
  return ()
end
```

## List Processing

### Recursive traversal (standard pattern)

```nexus
let sum = fn (xs: [ i64 ]) -> i64 do
  match xs do
    case Nil -> return 0
    case Cons(v: v, rest: rest) -> return v + sum(xs: rest)
  end
end
```

### Tail-recursive with accumulator (for large lists)

```nexus
let sum_acc = fn (xs: [ i64 ], acc: i64) -> i64 do
  match xs do
    case Nil -> return acc
    case Cons(v: v, rest: rest) -> return sum_acc(xs: rest, acc: acc + v)
  end
end

let sum = fn (xs: [ i64 ]) -> i64 do
  return sum_acc(xs: xs, acc: 0)
end
```

### Building lists (cons + reverse)

```nexus
// Build in reverse order (O(1) per element), then reverse
let filter_go = fn (xs: [ i64 ], pred: (val: i64) -> bool, acc: [ i64 ]) -> [ i64 ] do
  match xs do
    case Nil -> return list.reverse(xs: acc)
    case Cons(v: v, rest: rest) ->
      if pred(val: v) then
        let next = Cons(v: v, rest: acc)
        return filter_go(xs: rest, pred: pred, acc: next)
      else
        return filter_go(xs: rest, pred: pred, acc: acc)
      end
  end
end

let filter = fn (xs: [ i64 ], pred: (val: i64) -> bool) -> [ i64 ] do
  return filter_go(xs: xs, pred: pred, acc: Nil)
end
```

### Using fold_left

```nexus
// Sum via fold
let sum = fn (xs: [ i64 ]) -> i64 do
  return list.fold_left(xs: xs, init: 0, f: fn (acc: i64, val: i64) -> i64 do
    return acc + val
  end)
end

// String join
let join = fn (xs: [ string ], sep: string) -> string do
  match xs do
    case Nil -> return ""
    case Cons(v: first, rest: rest) ->
      return list.fold_left(xs: rest, init: first, f: fn (acc: string, val: string) -> string do
        return acc ++ sep ++ val
      end)
  end
end
```

## Array Patterns

```nexus
// Create and mutate
let %arr = [| 0, 0, 0 |]
let view = &%arr
view[0] <- 10
view[1] <- 20
view[2] <- 30

// Read
let first = view[0]
let len = array.length(arr: &%arr)

// Iterate
array.for_each(arr: &%arr, f: fn (val: i64) -> unit do
  Console.println(val: str.from_i64(n: val))
  return ()
end)

// Cleanup
array.consume(arr: %arr)
```

## Linear Resource Management

```nexus
// File handle pattern
let process_file = fn (path: string) -> string require { Fs } throws { Exn } do
  let %handle = Fs.open_read(path: path)
  let content = Fs.read(handle: %handle)   // consumes %handle
  return content
end

// HashMap pattern
let count_words = fn (words: [ string ]) -> unit do
  let %map = smap.empty()
  // ... populate map ...
  let keys = smap.keys(m: &%map)
  smap.free(m: %map)    // must explicitly free
  return ()
end

// Set pattern
let unique = fn (xs: [ i64 ]) -> [ i64 ] do
  let %s = set.from_list(xs: xs)
  let result = set.to_list(s: &%s)
  set.free(s: %s)
  return result
end
```

## Concurrency Pattern

```nexus
// Parallel tasks with shared state
let main = fn () -> unit require { PermConsole, PermNet } do
  inject stdio.system_handler, net_mod.system_handler do
    let %results = [| "", "" |]

    conc do
      task fetch_a do
        let resp = Net.get(url: "https://api.example.com/a")
        let body = response_body(resp: &resp)
        let lock = &%results
        lock[0] <- body
      end
      task fetch_b do
        let resp = Net.get(url: "https://api.example.com/b")
        let body = response_body(resp: &resp)
        let lock = &%results
        lock[1] <- body
      end
    end

    let view = &%results
    Console.println(val: "A: " ++ view[0])
    Console.println(val: "B: " ++ view[1])
    array.consume(arr: %results)
  end
  return ()
end
```

## Web Server Pattern

```nexus
import { Net, Request, Server, request_method, request_path, request_body }, * as net_mod from stdlib/net.nx
import { Console }, * as stdio from stdlib/stdio.nx

let handle = fn (req: Request) -> unit require { Net, Console } throws { Exn } do
  let method = request_method(req: &req)
  let path = request_path(req: &req)
  Console.println(val: method ++ " " ++ path)

  if path == "/health" then
    Net.respond(req: req, status: 200, body: "ok")
  else
    Net.respond(req: req, status: 404, body: "not found")
  end
  return ()
end

let serve_loop = fn (server: &Server) -> unit require { Net, Console } throws { Exn } do
  let req = Net.accept(server: server)
  handle(req: req)
  return serve_loop(server: server)
end

let main = fn () -> unit require { PermNet, PermConsole } do
  inject net_mod.system_handler, stdio.system_handler do
    try
      let server = Net.listen(addr: "127.0.0.1:8080")
      Console.println(val: "Listening on :8080")
      serve_loop(server: &server)
      Net.stop(server: server)
    catch e ->
      Console.println(val: "Server error")
    end
  end
  return ()
end
```

## String Processing

```nexus
import as str from stdlib/string.nx

// String concatenation with ++
let greeting = "Hello, " ++ name ++ "!"

// Conversion
let num_str = str.from_i64(n: 42)
let parsed = str.parse_i64(s: "123")    // throws Exn

// Splitting and joining
let parts = str.split(s: csv_line, sep: ",")
let joined = str.join(xs: parts, sep: " | ")

// Substring
let first_five = str.substring(s: text, start: 0, len: 5)
```

## For Loop Pattern

```nexus
// For loop: exclusive upper bound [start, end)
for i = 0 to 10 do
  Console.println(val: str.from_i64(n: i))    // prints 0..9
end

// Loop variable is stack-confined mutable (implicit ~)
```

## While Loop Pattern

```nexus
let ~running = true
while ~running do
  let input = Console.read_line()
  if input == "quit" then
    ~running <- false
  else
    Console.println(val: "You said: " ++ input)
  end
end
```
