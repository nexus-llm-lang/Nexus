use crate::common::wasm::exec;
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

let check = fn () -> i64 effect { Exn } do
    try
      let err = Boom(42)
      raise err
      return 1
    catch e ->
      return 7
    end
    return 0
end

let main = fn () -> unit effect { Exn } do
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

let check = fn () -> i64 effect { Exn } do
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

let main = fn () -> unit effect { Exn } do
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

let check = fn () -> i64 effect { Exn } do
    try
      raise Boom(42)
      return -1
    catch e ->
      match e do
        case Boom(_) -> return 1
        case _ -> return 2
      end
    end
    return 0
end

let main = fn () -> unit effect { Exn } do
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

let check = fn () -> i64 effect { Exn } do
    try
      raise Boom(42)
      return -1
    catch e ->
      match e do
        case Boom(code) -> return code
        case _ -> return -2
      end
    end
    return 0
end

let main = fn () -> unit effect { Exn } do
    let result = check()
    if result != 42 then raise RuntimeError(val: "expected 42") end
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
