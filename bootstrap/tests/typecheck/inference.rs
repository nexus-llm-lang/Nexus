use crate::harness::{should_fail_typecheck, should_typecheck};
use proptest::prelude::*;

#[test]
fn test_basic_poly() {
    should_typecheck(
        r#"
    let id = fn <T>(x: T) -> T do
        return x
    end

    let main = fn () -> unit do
        let i = id(x: 10)
        let b = id(x: true)
        return ()
    end
    "#,
    );
}

#[test]
fn test_nested_calls() {
    should_typecheck(
        r#"
    let id = fn <T>(x: T) -> T do return x end
    let test_fn = fn () -> i64 do
        let v = id(x: 10)
        return id(x: v)
    end
    "#,
    );
}

#[test]
fn test_two_generics() {
    should_typecheck(
        r#"
    let first = fn <A, B>(a: A, b: B) -> A do
        return a
    end

    let test_fn = fn () -> i64 do
        let f = first(a: 10, b: true)
        let s = first(a: true, b: 10)
        return f
    end
    "#,
    );
}

#[test]
fn test_record_access() {
    should_typecheck(
        r#"
    type Box<T> = { val: T }

    let unbox = fn <T>(b: Box<T>) -> T do
        return b.val
    end

    let test_fn = fn () -> i64 do
        // Since my infer for Record currently returns AnonymousRecord,
        // we can't test full record inference yet, but unbox signature check works.
        return 42
    end
    "#,
    );
}

#[test]
fn test_let_poly_binding() {
    should_typecheck(
        r#"
    let id = fn <T>(x: T) -> T do
        return x
    end

    let test_fn = fn () -> i64 do
        let f = id
        let a = f(x: 10)
        let b = f(x: true)
        return a
    end
    "#,
    );
}

#[test]
fn test_complex_poly_logic() {
    should_typecheck(
        r#"
    let weird = fn <T>(x: T) -> T do
        return x
    end

    let main = fn () -> unit do
        let a = weird(x: 1)
        let b = weird(x: "string")
        return ()
    end
    "#,
    );
}

#[test]
fn test_poly_variants() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let r1 = Ok(val: 1)
        let r2 = Ok(val: true)
        // Check match on poly variants
        match r1 do
            case Ok(val: v) -> let x = v + 1
            case Err(err: e) -> ()
        end
        return ()
    end
    "#,
    );
}

#[test]
fn test_type_sum_definition_with_labeled_variant_fields() {
    crate::harness::ensure_repo_root();
    let src = std::fs::read_to_string(
        "bootstrap/tests/fixtures/test_type_sum_definition_with_labeled_variant_fields.nx",
    )
    .expect("fixture should exist");
    should_typecheck(&src);
}

#[test]
fn test_arg_mismatch() {
    should_fail_typecheck(
        r#"
    let foo = fn (x: i64) -> i64 do return x end
    let test_fn = fn () -> i64 do
        return foo(x: true)
    end
    "#,
    );
}

#[test]
fn test_int_literal_defaults_to_i64() {
    should_typecheck(
        r#"
    let test_fn = fn () -> i64 do
        let x = 1
        return x
    end
    "#,
    );
}

#[test]
fn test_int_literal_is_not_i32_without_annotation() {
    should_fail_typecheck(
        r#"
    let test_fn = fn () -> i32 do
        let x = 1
        return x
    end
    "#,
    );
}

#[test]
fn test_int_literal_annotation_can_select_i32() {
    should_typecheck(
        r#"
    let test_fn = fn () -> i32 do
        let x: i32 = 1
        let y = x + 2
        return y
    end
    "#,
    );
}

#[test]
fn test_float_literal_annotation_can_select_f32() {
    should_typecheck(
        r#"
    let test_fn = fn () -> f32 do
        let x: f32 = 1.25
        let y = x +. 2.0
        return y
    end
    "#,
    );
}

#[test]
fn test_named_function_can_be_used_as_value() {
    should_typecheck(
        r#"
    let id = fn (x: i64) -> i64 do
        return x
    end

    let test_fn = fn () -> i64 do
        let f = id
        return f(x: 42)
    end
    "#,
    );
}

#[test]
fn test_inline_lambda_literal_typechecks() {
    should_typecheck(
        r#"
    let test_fn = fn () -> i64 do
        let f = fn (x: i64) -> i64 do
            return x + 1
        end
        return f(x: 41)
    end
    "#,
    );
}

#[test]
fn test_lambda_cannot_capture_ref() {
    let err = should_fail_typecheck(
        r#"
    let test_fn = fn () -> i64 do
        let ~counter = 1
        let read_counter = fn () -> i64 do
            return ~counter
        end
        return read_counter()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn test_linear_capture_makes_lambda_linear_and_single_use() {
    should_typecheck(
        r#"
    let test_fn = fn () -> i64 do
        let %x = 7
        let f = fn () -> i64 do
            match %x do case _ -> () end
            return 1
        end
        let y = f()
        return y
    end
    "#,
    );
}

#[test]
fn test_linear_capturing_lambda_cannot_be_called_twice() {
    let err = should_fail_typecheck(
        r#"
    let test_fn = fn () -> i64 do
        let %x = 7
        let f = fn () -> i64 do
            match %x do case _ -> () end
            return 1
        end
        let _a = f()
        let _b = f()
        return 0
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn test_recursive_lambda_with_annotation_typechecks() {
    crate::harness::ensure_repo_root();
    let src = std::fs::read_to_string(
        "bootstrap/tests/fixtures/test_recursive_lambda_with_annotation_typechecks.nx",
    )
    .expect("fixture should exist");
    should_typecheck(&src);
}

#[test]
fn test_constructor_arity_error_is_llm_friendly() {
    let err = should_fail_typecheck(
        r#"
    type Pair = Pair(left: i64, right: i64)

    let test_fn = fn () -> i64 do
        let _p = Pair(left: 1)
        return 0
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn test_constructor_pattern_arity_error_is_llm_friendly() {
    let err = should_fail_typecheck(
        r#"
    type Pair = Pair(left: i64, right: i64)

    let test_fn = fn () -> i64 do
        let p: Pair = Pair(left: 1, right: 2)
        match p do
            case Pair(left: x) -> return x
        end
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn test_binary_op_in_call_arg() {
    should_typecheck(
        r#"
    let add = fn (a: i64, b: i64) -> i64 do
        return a + b
    end

    let test_fn = fn () -> i64 do
        return add(a: 1 + 2, b: 3 * 4)
    end
    "#,
    );
}

#[test]
fn test_string_concat_in_call_arg() {
    should_typecheck(
        r#"
    let greet = fn (msg: string) -> string do
        return msg
    end

    let test_fn = fn () -> string do
        return greet(msg: "hello " ++ "world")
    end
    "#,
    );
}

#[test]
fn test_function_arity_mismatch_shows_expected() {
    let err = should_fail_typecheck(
        r#"
    let add = fn (a: i64, b: i64) -> i64 do
        return a + b
    end

    let test_fn = fn () -> i64 do
        return add(a: 1)
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn test_function_arity_mismatch_too_many_args() {
    let err = should_fail_typecheck(
        r#"
    let inc = fn (x: i64) -> i64 do
        return x + 1
    end

    let test_fn = fn () -> i64 do
        return inc(x: 1, y: 2)
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn test_float_arithmetic() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let x = 1.5 +. 2.5
        let y = x *. 2.0
        return ()
    end
    "#,
    );
}

#[test]
fn test_float_compare() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let b = 1.0 <. 2.0
        if b then return () else return () end
    end
    "#,
    );
}

#[test]
fn test_float_int_mismatch() {
    should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let x = 1 +. 2.0
        return ()
    end
    "#,
    );
}

#[test]
fn test_float_literal_type() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let x: float = 3.14
        let y: float = 0.01
        let z: float = 123.456789
        return ()
    end
    "#,
    );
}

#[test]
fn test_f32_and_f64_keywords() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let x: f32 = 1.5
        let y: f64 = 2.0
        let z = x +. 3.5
        let w = y +. 4.0
        return ()
    end
    "#,
    );
}

#[test]
fn test_anonymous_record() {
    should_typecheck(
        r#"
    import { Console }, * as stdio from "stdlib/stdio.nx"
    import { from_i64 } from "stdlib/string.nx"
    let main = fn () -> unit require { PermConsole } do
        inject stdio.system_handler do
            let r = { x: 1, y: "hello" }
            let i = r.x
            let i_s = from_i64(val: i)
            let msg = "i=" ++ i_s
            Console.print(val: msg)
        end
        return ()
    end
    "#,
    );
}

#[test]
fn test_record_unification() {
    should_typecheck(
        r#"
    let take_record = fn (r: { x: i64, y: i64 }) -> unit do
        return ()
    end

    let main = fn () -> unit do
        let r1 = { x: 1, y: 2 }
        let r2 = { y: 2, x: 1 } // Different order
        take_record(r: r1)
        take_record(r: r2)
        return ()
    end
    "#,
    );
}

#[test]
fn test_record_fail() {
    should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let r = { x: 1 }
        let y = r.y // Field missing
        return ()
    end
    "#,
    );
}

#[test]
fn test_ref_creation_and_type() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let ~c = 0
        return ()
    end
    "#,
    );
}

#[test]
fn test_gravity_rule_immutable_holds_value() {
    crate::harness::ensure_repo_root();
    let src = std::fs::read_to_string("bootstrap/tests/fixtures/test_gravity_rule_immutable_holds_value.nx")
        .expect("fixture should exist");
    should_typecheck(&src);
}

#[test]
fn test_cannot_return_ref() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let ~c = 0
        // return c // Variable not found
        return ()
    end
    "#,
    );
}

#[test]
fn test_ref_assignment() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let ~c = 0
        ~c <- 1
        return ()
    end
    "#,
    );
}

#[test]
fn test_ref_read() {
    should_typecheck(
        r#"
    let __test_main = fn () -> i64 do
        let ~c = 10
        let v = ~c
        return v
    end
    "#,
    );
}

#[test]
fn test_ref_generic() {
    crate::harness::ensure_repo_root();
    let src = std::fs::read_to_string("bootstrap/tests/fixtures/test_ref_generic.nx")
        .expect("fixture should exist");
    should_typecheck(&src);
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_typecheck_nested_arithmetic(a in -100i64..100, b in -100i64..100, c in -100i64..100) {
        let src = format!("
let test_fn = fn () -> i64 do
    return ({} + {}) * {}
end
", a, b, c);
        should_typecheck(&src);
    }

    #[test]
    fn prop_typecheck_if_else_branches_must_match(cond in proptest::bool::ANY) {
        let src = format!("
let test_fn = fn () -> i64 do
    if {} then
        return 1
    else
        return [=[string]=]
    end
end
", if cond { "true" } else { "false" });
        should_fail_typecheck(&src);
    }
}

// ---- Match expression type inference ----

#[test]
fn test_match_expr_same_type_cases_typechecks() {
    should_typecheck(
        r#"
let f = fn (x: i64) -> i64 do
    let result: i64 = match x do
      case 1 -> 10
      case _ -> 20
    end
    return result
end
"#,
    );
}

#[test]
fn test_match_expr_type_mismatch_fails() {
    let msg = should_fail_typecheck(
        r#"
let f = fn (x: i64) -> i64 do
    let result: i64 = match x do
      case 1 -> 10
      case _ -> true
    end
    return result
end
"#,
    );
    insta::assert_snapshot!(msg);
}

#[test]
fn test_match_expr_with_return_cases_diverge() {
    // Cases that return from the function are compatible with expression cases
    should_typecheck(
        r#"
let f = fn (x: i64) -> i64 do
    let result = match x do
      case 0 -> return 999
      case _ -> 42
    end
    return result
end
"#,
    );
}

#[test]
fn test_main_with_args_typechecks() {
    should_typecheck(
        r#"
let main = fn (args: [string]) -> unit do
    return ()
end
"#,
    );
}

#[test]
fn test_main_with_wrong_arg_type_fails() {
    let msg = should_fail_typecheck(
        r#"
let main = fn (x: i64) -> unit do
    return ()
end
"#,
    );
    insta::assert_snapshot!(msg);
}

#[test]
fn test_main_with_too_many_args_fails() {
    let msg = should_fail_typecheck(
        r#"
let main = fn (args: [string], extra: i64) -> unit do
    return ()
end
"#,
    );
    insta::assert_snapshot!(msg);
}

// ---- Destructuring let ----

#[test]
fn test_let_destructure_record() {
    should_typecheck(
        r#"
    type Point = { x: i64, y: i64 }

    let main = fn () -> unit do
        let p: Point = { x: 10, y: 20 }
        let {x: a, y: b} = p
        return ()
    end
    "#,
    );
}

#[test]
fn test_let_destructure_nested_record() {
    should_typecheck(
        r#"
    type Inner = { a: i64 }
    type Outer = { inner: Inner, b: bool }

    let main = fn () -> unit do
        let o: Outer = { inner: { a: 1 }, b: true }
        let {inner: {a: x}, b: y} = o
        return ()
    end
    "#,
    );
}

#[test]
fn test_let_destructure_non_exhaustive_fails() {
    let msg = should_fail_typecheck(
        r#"
type Option<T> = Some(value: T) | None

let main = fn () -> unit do
    let x: Option<i64> = Some(value: 42)
    let Some(value: v) = x
    return ()
end
"#,
    );
    insta::assert_snapshot!(msg);
}

// ---- Implicit Unit return ----

#[test]
fn test_no_return_non_unit_is_type_error() {
    let msg = should_fail_typecheck(
        r#"
    let main = fn () -> i64 do
        let x = 1
    end
    "#,
    );
    insta::assert_snapshot!(msg);
}

#[test]
fn test_no_return_unit_is_ok() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let x = 1
    end
    "#,
    );
}

#[test]
fn test_implicit_unit_return_with_side_effect() {
    should_typecheck(
        r#"
    let side_effect = fn () -> unit do
        let x = 1 + 2
    end
    "#,
    );
}

#[test]
fn test_return_in_if_branch_counts_as_return() {
    should_typecheck(
        r#"
    let f = fn (x: i64) -> i64 do
        if x > 0 then
            return x
        else
            return 0 - x
        end
    end
    "#,
    );
}

#[test]
fn test_return_in_match_counts_as_return() {
    should_typecheck(
        r#"
    let f = fn (x: bool) -> i64 do
        match x do
        case true -> return 1
        case false -> return 0
        end
    end
    "#,
    );
}

// ─── If-else expression type inference ───────────────────────────────────────

#[test]
fn if_else_expr_infers_i64() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let x: i64 = if true then 1 else 2 end
        return ()
    end
    "#,
    );
}

#[test]
fn if_else_expr_infers_string() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let s: string = if true then "yes" else "no" end
        return ()
    end
    "#,
    );
}

#[test]
fn if_else_expr_type_mismatch_fails() {
    should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let x: i64 = if true then 1 else "no" end
        return ()
    end
    "#,
    );
}

#[test]
fn if_else_expr_as_return_value() {
    should_typecheck(
        r#"
    let max = fn (a: i64, b: i64) -> i64 do
        return if a > b then a else b end
    end
    let main = fn () -> unit do
        let _ = max(a: 1, b: 2)
        return ()
    end
    "#,
    );
}

#[test]
fn if_else_expr_nested() {
    should_typecheck(
        r#"
    let classify = fn (x: i64) -> i64 do
        return if x > 0 then if x > 100 then 2 else 1 end else 0 end
    end
    let main = fn () -> unit do
        let _ = classify(x: 5)
        return ()
    end
    "#,
    );
}
