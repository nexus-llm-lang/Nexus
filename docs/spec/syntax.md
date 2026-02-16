# Syntax

Nexus uses a clean, keyword-oriented syntax designed for readability and clarity for both humans and AI.

## Functions

Functions are defined using `fn`. They must specify argument types and return types.
Blocks are terminated with `endfn`.

```nexus
pub fn add(a: i64, b: i64) -> i64 do
  return a + b
endfn
```

- `pub` keyword makes the function public (visible to other modules).
- Arguments are labeled when calling: `add(a: 1, b: 2)`.

## Variables (`let`)

Immutable variables are defined with `let`.

```nexus
let x = 10
let name = "Nexus"
```

Variables are lexically scoped.

## Control Flow

### If-Else

```nexus
if condition then
  // ...
else
  // ...
endif
```

### Match

Pattern matching is supported via `match`.

```nexus
match result do
  case Ok(val) ->
    // ...
  case Err(e) ->
    // ...
endmatch
```

## Comments

Line comments start with `//`.

```nexus
// This is a comment
```

## Concurrency (`conc`)

Structured concurrency is supported via `conc` blocks with `task`.

```nexus
conc do
  task "worker1" do
    // ...
  endtask
  task "worker2" do
    // ...
  endtask
endconc
```
Tasks run concurrently and `conc` waits for all to complete.
