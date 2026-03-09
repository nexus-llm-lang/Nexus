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
