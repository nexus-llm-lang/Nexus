use crate::harness::exec;
use proptest::prelude::*;

#[test]
fn codegen_bool_return_is_i32_flag() {
    exec(
        r#"
let main = fn () -> unit do
    if 10 < 11 then
        return ()
    else
        raise RuntimeError(val: "expected true")
    end
end
"#,
    );
}

#[test]
fn codegen_match_literal_statement_returns_correct_arm() {
    exec(
        r#"
let check = fn (x: i64) -> i64 do
    match x do
      case 1 -> return 10
      case 2 -> return 20
      case _ -> return 30
    end
    return 0
end

let main = fn () -> unit do
    let result = check(x: 2)
    if result != 20 then raise RuntimeError(val: "expected 20") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_match_variable_pattern_can_return_target_value() {
    exec(
        r#"
let check = fn (x: i64) -> i64 do
    match x do
      case v -> return v
    end
    return 0
end

let main = fn () -> unit do
    let result = check(x: 42)
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_match_literal_then_variable_fallback() {
    exec(
        r#"
let check = fn (x: i64) -> i64 do
    match x do
      case 0 -> return 0
      case other -> return other
    end
    return -1
end

let main = fn () -> unit do
    let result = check(x: 7)
    if result != 7 then raise RuntimeError(val: "expected 7") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_match_record_pattern_binds_fields() {
    exec(
        r#"
let check = fn (r: { x: i64, y: i64 }) -> i64 do
    match r do
      case { x: a, y: b } -> return a + b
    end
    return 0
end

let main = fn () -> unit do
    let r = { y: 2, x: 40 }
    let result = check(r: r)
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_try_catch_handles_raised_exception() {
    exec(
        r#"
exception Boom(i64)

let check = fn () -> i64 throws { Exn } do
    try
      let err = Boom(42)
      raise err
      return 1
    catch e ->
      return 7
    end
    return 0
end

let main = fn () -> unit throws { Exn } do
    let result = check()
    if result != 7 then raise RuntimeError(val: "expected 7") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_nested_try_catch_reraise_propagates_to_outer_catch() {
    exec(
        r#"
exception Boom(i64)

let check = fn () -> i64 throws { Exn } do
    try
      try
        raise Boom(1)
        return -1
      catch e ->
        raise e
        return -2
      end
      return -3
    catch outer ->
      return 9
    end
    return 0
end

let main = fn () -> unit throws { Exn } do
    let result = check()
    if result != 9 then raise RuntimeError(val: "expected 9") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_try_catch_match_constructor_wildcard_case() {
    exec(
        r#"
exception Boom(i64)

let check = fn () -> i64 throws { Exn } do
    try
      raise Boom(42)
      return -1
    catch
      case Boom(_) -> return 1
      case _ -> return 2
    end
    return 0
end

let main = fn () -> unit throws { Exn } do
    let result = check()
    if result != 1 then raise RuntimeError(val: "expected 1") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_try_catch_match_constructor_binds_payload() {
    exec(
        r#"
exception Boom(i64)

let check = fn () -> i64 throws { Exn } do
    try
      raise Boom(42)
      return -1
    catch
      case Boom(code) -> return code
      case _ -> return -2
    end
    return 0
end

let main = fn () -> unit throws { Exn } do
    let result = check()
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

// ---- Cross-function exception handling ----

#[test]
fn codegen_cross_function_raise_caught_by_caller_try_catch() {
    exec(
        r#"
exception Boom(i64)

let thrower = fn () -> unit throws { Exn } do
    raise Boom(42)
    return ()
end

let main = fn () -> unit throws { Exn } do
    try
        thrower()
        raise RuntimeError(val: "should not reach here")
    catch
          case Boom(code) ->
            if code != 42 then raise RuntimeError(val: "expected 42") end
            return ()
          case _ -> raise RuntimeError(val: "unexpected exception")
    end
    return ()
end
"#,
    );
}

#[test]
fn codegen_cross_function_raise_with_return_value_caught() {
    exec(
        r#"
exception Boom(i64)

let thrower = fn () -> i64 throws { Exn } do
    raise Boom(99)
    return 0
end

let main = fn () -> unit throws { Exn } do
    try
        let _ = thrower()
        raise RuntimeError(val: "should not reach here")
    catch
          case Boom(code) ->
            if code != 99 then raise RuntimeError(val: "expected 99") end
            return ()
          case _ -> raise RuntimeError(val: "unexpected exception")
    end
    return ()
end
"#,
    );
}

#[test]
fn codegen_cross_function_raise_propagates_through_intermediate() {
    exec(
        r#"
exception Boom(i64)

let deep_thrower = fn () -> unit throws { Exn } do
    raise Boom(7)
    return ()
end

let middle = fn () -> unit throws { Exn } do
    deep_thrower()
    return ()
end

let main = fn () -> unit throws { Exn } do
    try
        middle()
        raise RuntimeError(val: "should not reach here")
    catch
          case Boom(code) ->
            if code != 7 then raise RuntimeError(val: "expected 7") end
            return ()
          case _ -> raise RuntimeError(val: "unexpected exception")
    end
    return ()
end
"#,
    );
}

// ---- Match as expression ----

#[test]
fn codegen_match_expr_literal_cases() {
    exec(
        r#"
let classify = fn (x: i64) -> i64 do
    let result = match x do
      case 1 -> 10
      case 2 -> 20
      case _ -> 30
    end
    return result
end

let main = fn () -> unit do
    let r1 = classify(x: 1)
    if r1 != 10 then raise RuntimeError(val: "expected 10") end
    let r2 = classify(x: 2)
    if r2 != 20 then raise RuntimeError(val: "expected 20") end
    let r3 = classify(x: 99)
    if r3 != 30 then raise RuntimeError(val: "expected 30") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_match_expr_variable_binding() {
    exec(
        r#"
let double = fn (x: i64) -> i64 do
    let result = match x do
      case v -> v + v
    end
    return result
end

let main = fn () -> unit do
    let r = double(x: 21)
    if r != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_match_expr_record_pattern() {
    exec(
        r#"
let sum_fields = fn (r: { x: i64, y: i64 }) -> i64 do
    let result = match r do
      case { x: a, y: b } -> a + b
    end
    return result
end

let main = fn () -> unit do
    let r = sum_fields(r: { x: 40, y: 2 })
    if r != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_match_expr_with_function_call_in_case() {
    exec(
        r#"
let add = fn (a: i64, b: i64) -> i64 do
    return a + b
end

let compute = fn (x: i64) -> i64 do
    let result = match x do
      case 0 -> 0
      case n -> add(a: n, b: 100)
    end
    return result
end

let main = fn () -> unit do
    let r = compute(x: 5)
    if r != 105 then raise RuntimeError(val: "expected 105") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_match_expr_bool_cases() {
    exec(
        r#"
let to_int = fn (b: bool) -> i64 do
    let result = match b do
      case true -> 1
      case false -> 0
    end
    return result
end

let main = fn () -> unit do
    let r1 = to_int(b: true)
    if r1 != 1 then raise RuntimeError(val: "expected 1") end
    let r2 = to_int(b: false)
    if r2 != 0 then raise RuntimeError(val: "expected 0") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_match_expr_constructor_pattern() {
    exec(
        r#"
type Color = Red | Green | Blue

let to_code = fn (c: Color) -> i64 do
    let result = match c do
      case Red -> 1
      case Green -> 2
      case Blue -> 3
    end
    return result
end

let main = fn () -> unit do
    let r = to_code(c: Green)
    if r != 2 then raise RuntimeError(val: "expected 2") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_match_expr_nested_in_binop() {
    exec(
        r#"
let f = fn (x: i64) -> i64 do
    let a = match x do
      case 1 -> 10
      case _ -> 20
    end
    return a + 5
end

let main = fn () -> unit do
    let r = f(x: 1)
    if r != 15 then raise RuntimeError(val: "expected 15") end
    let r2 = f(x: 99)
    if r2 != 25 then raise RuntimeError(val: "expected 25") end
    return ()
end
"#,
    );
}

// ---- While loop ----

#[test]
fn codegen_while_loop_basic() {
    exec(
        r#"
let main = fn () -> unit do
    let ~sum = 0
    let ~i = 0
    while ~i < 5 do
        ~sum <- ~sum + ~i
        ~i <- ~i + 1
    end
    if ~sum != 10 then raise RuntimeError(val: "expected 10") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_while_loop_false_condition_never_executes() {
    exec(
        r#"
let main = fn () -> unit do
    let ~x = 0
    while false do
        ~x <- 999
    end
    if ~x != 0 then raise RuntimeError(val: "should not execute") end
    return ()
end
"#,
    );
}

// ---- For loop ----

#[test]
fn codegen_for_loop_basic() {
    exec(
        r#"
let main = fn () -> unit do
    let ~sum = 0
    for i = 0 to 5 do
        ~sum <- ~sum + i
    end
    if ~sum != 10 then raise RuntimeError(val: "expected 10") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_for_loop_empty_range() {
    exec(
        r#"
let main = fn () -> unit do
    let ~x = 0
    for i = 5 to 5 do
        ~x <- 999
    end
    if ~x != 0 then raise RuntimeError(val: "should not execute") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_for_loop_with_computation() {
    exec(
        r#"
let main = fn () -> unit do
    let ~product = 1
    for i = 1 to 6 do
        ~product <- ~product * i
    end
    if ~product != 120 then raise RuntimeError(val: "expected 120 (5!)") end
    return ()
end
"#,
    );
}

// ---- If-expression inside match-expression as let initializer ----

#[test]
fn codegen_if_expr_inside_match_expr_as_let_value() {
    exec(
        r#"
let pick = fn (flag: bool, a: i64, b: i64) -> i64 do
    let result = match flag do
      case true ->
        if a > b then a else b end
      case false -> 0
    end
    return result
end

let main = fn () -> unit do
    let r1 = pick(flag: true, a: 10, b: 20)
    if r1 != 20 then raise RuntimeError(val: "expected 20") end
    let r2 = pick(flag: true, a: 30, b: 5)
    if r2 != 30 then raise RuntimeError(val: "expected 30") end
    let r3 = pick(flag: false, a: 10, b: 20)
    if r3 != 0 then raise RuntimeError(val: "expected 0") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_if_expr_inside_nested_match_expr_as_let_value() {
    exec(
        r#"
type Wrapper = Wrapper(val: i64)
type Outer = OuterA(inner: Wrapper) | OuterB

let classify = fn (x: Outer, threshold: i64) -> i64 do
    let result = match x do
      case OuterA(inner: w) ->
        match w do case Wrapper(val: v) ->
          if v > threshold then v else threshold end
        end
      case OuterB -> 0
    end
    return result
end

let main = fn () -> unit do
    let r1 = classify(x: OuterA(inner: Wrapper(val: 100)), threshold: 50)
    if r1 != 100 then raise RuntimeError(val: "expected 100") end
    let r2 = classify(x: OuterA(inner: Wrapper(val: 10)), threshold: 50)
    if r2 != 50 then raise RuntimeError(val: "expected 50") end
    let r3 = classify(x: OuterB, threshold: 50)
    if r3 != 0 then raise RuntimeError(val: "expected 0") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_if_expr_with_call_inside_nested_match() {
    exec(
        r#"
type Wrapper = Wrapper(val: i64)
type Outer = OuterA(inner: Wrapper) | OuterB

let add10 = fn (x: i64) -> i64 do
    return x + 10
end

let compute = fn (x: Outer, flag: bool) -> i64 do
    let result = match x do
      case OuterA(inner: w) ->
        match w do case Wrapper(val: v) ->
          if flag then add10(x: v) else v end
        end
      case OuterB -> 0
    end
    return result
end

let main = fn () -> unit do
    let r1 = compute(x: OuterA(inner: Wrapper(val: 5)), flag: true)
    if r1 != 15 then raise RuntimeError(val: "expected 15") end
    let r2 = compute(x: OuterA(inner: Wrapper(val: 5)), flag: false)
    if r2 != 5 then raise RuntimeError(val: "expected 5") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_if_expr_returning_list_inside_nested_match() {
    exec(
        r#"
type Wrapper = Wrapper(name: string)
type Outer = OuterA(inner: Wrapper) | OuterB

let classify = fn (x: Outer, flag: bool) -> [string] do
    let result = match x do
      case OuterA(inner: w) ->
        match w do case Wrapper(name: n) ->
          if flag then Cons(v: n, rest: Nil) else Nil end
        end
      case OuterB -> Nil
    end
    return result
end

let main = fn () -> unit do
    let r1 = classify(x: OuterA(inner: Wrapper(name: "hello")), flag: true)
    match r1 do
      case Cons(v: s, rest: Nil) ->
        if s != "hello" then raise RuntimeError(val: "expected hello") end
      case _ -> raise RuntimeError(val: "expected Cons")
    end
    let r2 = classify(x: OuterA(inner: Wrapper(name: "hello")), flag: false)
    match r2 do
      case Nil -> return ()
      case _ -> raise RuntimeError(val: "expected Nil")
    end
    return ()
end
"#,
    );
}

// Repro: exact pattern from build_selective_rename_go in resolve.nx
// let result = match outer do
//   case OuterA(inner) ->
//     match inner do case Inner(name) ->
//       if cond then fn_call() else fn_call() end
//     end
//   case _ -> default_val
// end
#[test]
fn codegen_if_call_inside_double_nested_match_as_let() {
    exec(
        r#"
type Inner = Inner(name: string)
type Outer = OuterA(inner: Inner) | OuterB

let prepend = fn (s: string, prefix: string) -> string do
    return prefix ++ s
end

let identity = fn (s: string) -> string do
    return s
end

let classify = fn (x: Outer, flag: bool) -> string do
    let result = match x do
      case OuterA(inner: w) ->
        match w do case Inner(name: n) ->
          if flag then identity(s: n) else prepend(s: n, prefix: "pfx.") end
        end
      case OuterB -> "none"
    end
    return result
end

let main = fn () -> unit do
    let r1 = classify(x: OuterA(inner: Inner(name: "hello")), flag: true)
    if r1 != "hello" then raise RuntimeError(val: "expected hello") end
    let r2 = classify(x: OuterA(inner: Inner(name: "hello")), flag: false)
    if r2 != "pfx.hello" then raise RuntimeError(val: "expected pfx.hello") end
    let r3 = classify(x: OuterB, flag: true)
    if r3 != "none" then raise RuntimeError(val: "expected none") end
    return ()
end
"#,
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_codegen_simple_if(a in 0i64..10) {
        let expected = if a > 5 { 1i64 } else { 2 };
        let src = format!(r#"
let check = fn (x: i64) -> i64 do
    if x > 5 then
        return 1
    else
        return 2
    end
    return 0
end

let main = fn () -> unit do
    let result = check(x: {})
    if result != {} then raise RuntimeError(val: "if mismatch") end
    return ()
end
"#, a, expected);
        exec(&src);
    }
}

#[test]
fn codegen_stmt_after_nested_match_is_reachable() {
    exec(
        r#"
type Color = Red | Green | Blue
type Shape = Circle | Square

let describe = fn (c: Color, s: Shape) -> i64 do
    let x = 0
    match c do
      case Red ->
        match s do
          case Circle -> let x = 1
          case Square -> let x = 2
        end
      case Green ->
        let x = 10
      case Blue ->
        let x = 20
    end
    let after = 100
    return after
end

let main = fn () -> unit do
    let r = describe(c: Red(), s: Circle())
    if r != 100 then raise RuntimeError(val: "code after nested match not reached") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_recursive_call_after_match_is_reachable() {
    exec(
        r#"
type Color = Red | Green | Blue

let count = fn (xs: [Color], acc: i64) -> i64 do
    match xs do
      case Nil -> return acc
      case Cons(v: c, rest: rest) ->
        let inc = match c do
          case Red -> 1
          case Green -> 2
          case Blue -> 3
        end
        let next = acc + inc
        return count(xs: rest, acc: next)
    end
    return acc
end

let main = fn () -> unit do
    let items = Cons(v: Red(), rest: Cons(v: Blue(), rest: Cons(v: Green(), rest: Nil)))
    let total = count(xs: items, acc: 0)
    if total != 6 then raise RuntimeError(val: "expected 6") end
    return ()
end
"#,
    );
}

// ─── If-else expression tests ────────────────────────────────────────────────

#[test]
fn if_else_expr_returns_then_branch_value() {
    exec(
        r#"
let main = fn () -> unit do
    let x = if true then 42 else 0 end
    if x != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn if_else_expr_returns_else_branch_value() {
    exec(
        r#"
let main = fn () -> unit do
    let x = if false then 42 else 99 end
    if x != 99 then raise RuntimeError(val: "expected 99") end
    return ()
end
"#,
    );
}

#[test]
fn if_else_expr_with_condition_variable() {
    exec(
        r#"
let pick = fn (flag: bool, a: i64, b: i64) -> i64 do
    return if flag then a else b end
end
let main = fn () -> unit do
    let r1 = pick(flag: true, a: 10, b: 20)
    let r2 = pick(flag: false, a: 10, b: 20)
    if r1 != 10 then raise RuntimeError(val: "expected 10") end
    if r2 != 20 then raise RuntimeError(val: "expected 20") end
    return ()
end
"#,
    );
}

#[test]
fn if_else_expr_with_string_type() {
    exec(
        r#"
let greet = fn (formal: bool) -> string do
    return if formal then "Good day" else "Hey" end
end
let main = fn () -> unit do
    let g = greet(formal: true)
    if g != "Good day" then raise RuntimeError(val: "expected formal") end
    return ()
end
"#,
    );
}

#[test]
fn if_else_expr_nested() {
    exec(
        r#"
let classify = fn (x: i64) -> i64 do
    return if x > 0 then
        if x > 100 then 2 else 1 end
    else 0 end
end
let main = fn () -> unit do
    if classify(x: 200) != 2 then raise RuntimeError(val: "expected 2") end
    if classify(x: 50) != 1 then raise RuntimeError(val: "expected 1") end
    if classify(x: -5) != 0 then raise RuntimeError(val: "expected 0") end
    return ()
end
"#,
    );
}

#[test]
fn if_else_expr_in_let_binding() {
    exec(
        r#"
let main = fn () -> unit do
    let a = 10
    let b = 20
    let max_val = if a > b then a else b end
    if max_val != 20 then raise RuntimeError(val: "expected 20") end
    return ()
end
"#,
    );
}

// ---- return spreading: return if/match ----

#[test]
fn return_if_produces_correct_values() {
    exec(
        r#"
let pick = fn (flag: bool) -> i64 do
    return if flag then 42 else 99 end
end

let main = fn () -> unit do
    let a = pick(flag: true)
    let b = pick(flag: false)
    if a != 42 then raise RuntimeError(val: "expected 42") end
    if b != 99 then raise RuntimeError(val: "expected 99") end
    return ()
end
"#,
    );
}

#[test]
fn return_match_produces_correct_values() {
    exec(
        r#"
type Color = Red | Green | Blue

let to_num = fn (c: Color) -> i64 do
    return match c do
        case Red -> 1
        case Green -> 2
        case Blue -> 3
    end
end

let main = fn () -> unit do
    let r = to_num(c: Red)
    let g = to_num(c: Green)
    let b = to_num(c: Blue)
    if r != 1 then raise RuntimeError(val: "expected 1") end
    if g != 2 then raise RuntimeError(val: "expected 2") end
    if b != 3 then raise RuntimeError(val: "expected 3") end
    return ()
end
"#,
    );
}

#[test]
fn return_if_with_mutual_recursion_executes_correctly() {
    exec(
        r#"
let even = fn (n: i64) -> i64 do
    return if n == 0 then 1 else odd(n: n - 1) end
end

let odd = fn (n: i64) -> i64 do
    return if n == 0 then 0 else even(n: n - 1) end
end

let main = fn () -> unit do
    let r1 = even(n: 10)
    let r2 = odd(n: 10)
    if r1 != 1 then raise RuntimeError(val: "even(10) should be 1") end
    if r2 != 0 then raise RuntimeError(val: "odd(10) should be 0") end
    return ()
end
"#,
    );
}

#[test]
fn if_let_some_branch() {
    exec(
        r#"
type Option<T> = Some(value: T) | None

let main = fn () -> unit do
    let opt: Option<i64> = Some(value: 42)
    let result = if let Some(value: v) = opt then v else 0 end
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn if_let_none_branch() {
    exec(
        r#"
type Option<T> = Some(value: T) | None

let main = fn () -> unit do
    let opt: Option<i64> = None
    let result = if let Some(_) = opt then 1 else 0 end
    if result != 0 then raise RuntimeError(val: "expected 0") end
    return ()
end
"#,
    );
}

#[test]
fn if_let_statement_no_else() {
    exec(
        r#"
type Option<T> = Some(value: T) | None

let check = fn (opt: Option<i64>) -> i64 do
    let result = 0
    if let Some(value: v) = opt then
        return v
    end
    return result
end

let main = fn () -> unit do
    let r = check(opt: Some(value: 10))
    if r != 10 then raise RuntimeError(val: "expected 10") end
    return ()
end
"#,
    );
}

#[test]
fn if_let_record_pattern() {
    exec(
        r#"
type Pair = MkPair(fst: i64, snd: i64) | Empty

let main = fn () -> unit do
    let p: Pair = MkPair(fst: 3, snd: 4)
    let result = if let MkPair(fst: a, snd: b) = p then a + b else 0 end
    if result != 7 then raise RuntimeError(val: "expected 7") end
    return ()
end
"#,
    );
}

// ---------------------------------------------------------------------------
// Selective catch tests
// ---------------------------------------------------------------------------

#[test]
fn codegen_selective_catch_matches_correct_arm() {
    exec(
        r#"
exception Boom(i64)
exception Oops(i64)

let check = fn () -> i64 throws { Exn } do
    try
        raise Boom(42)
        return -1
    catch
        case Boom(code) -> return code
        case Oops(n) -> return n + 100
        case _ -> return -2
    end
    return -3
end

let main = fn () -> unit throws { Exn } do
    let result = check()
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_selective_catch_wildcard_catches_unmatched() {
    exec(
        r#"
exception Boom(i64)
exception Oops(i64)

let check = fn () -> i64 throws { Exn } do
    try
        raise Oops(99)
        return -1
    catch
        case Boom(code) -> return code
        case _ -> return 77
    end
    return -3
end

let main = fn () -> unit throws { Exn } do
    let result = check()
    if result != 77 then raise RuntimeError(val: "expected 77") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_selective_catch_labeled_fields() {
    exec(
        r#"
exception DbError(code: i64, msg: string)

let check = fn () -> i64 throws { Exn } do
    try
        raise DbError(code: 404, msg: "not found")
        return -1
    catch
        case DbError(code: c, msg: _) -> return c
        case _ -> return -2
    end
    return -3
end

let main = fn () -> unit throws { Exn } do
    let result = check()
    if result != 404 then raise RuntimeError(val: "expected 404") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_legacy_catch_still_works() {
    exec(
        r#"
exception Boom(i64)

let check = fn () -> i64 throws { Exn } do
    try
        raise Boom(55)
        return -1
    catch
        case Boom(code) -> return code
        case _ -> return -2
    end
    return -3
end

let main = fn () -> unit throws { Exn } do
    let result = check()
    if result != 55 then raise RuntimeError(val: "expected 55") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_selective_catch_nested_try_routes_exceptions() {
    exec(
        r#"
exception Boom(i64)
exception Oops(i64)

let check = fn () -> i64 throws { Exn } do
    try
        try
            raise Oops(10)
            return -1
        catch
            case Boom(code) -> return code
            case _ -> raise RuntimeError(val: "inner fallthrough")
        end
        return -2
    catch
        case RuntimeError(_) -> return 77
        case _ -> return -3
    end
    return -4
end

let main = fn () -> unit throws { Exn } do
    let result = check()
    if result != 77 then raise RuntimeError(val: "expected 77") end
    return ()
end
"#,
    );
}
