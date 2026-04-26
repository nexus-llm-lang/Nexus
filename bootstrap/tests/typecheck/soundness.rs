/// Soundness bridge tests: programs that typecheck must also compile and execute
/// without trapping due to type errors.
///
/// The property under test:
///   forall p. typecheck(p) = Ok => compile_and_run(p) != TypeError
///
/// These tests close the gap between typecheck-only tests and execution tests
/// by systematically verifying that the typechecker's "accept" judgement
/// is consistent with runtime behavior.
use crate::harness::{exec, exec_with_stdlib, should_typecheck};

// ---------------------------------------------------------------------------
// Basic type soundness: accepted programs run without type traps
// ---------------------------------------------------------------------------

#[test]
fn soundness_polymorphic_id() {
    let src = r#"
    let id = fn <T>(x: T) -> T do return x end

    let main = fn () -> unit do
        let a = id(x: 42)
        let b = id(x: true)
        let c = id(x: "hello")
        return ()
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_generic_pair() {
    let src = r#"
    type Pair<A, B> = Pair(fst: A, snd: B)

    let main = fn () -> unit do
        let p = Pair(fst: 1, snd: true)
        match p do
            | Pair(fst: a, snd: b) -> return ()
        end
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_option_some_none() {
    let src = r#"
    let main = fn () -> unit do
        let x: Option<i64> = Some(val: 42)
        let y: Option<i64> = None
        match x do
            | Some(val: v) -> return ()
            | None -> return ()
        end
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_result_ok_err() {
    let src = r#"
    let main = fn () -> unit do
        let x: Result<i64, string> = Ok(val: 42)
        let y: Result<i64, string> = Err(err: "oops")
        match x do
            | Ok(val: v) -> return ()
            | Err(err: e) -> return ()
        end
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_nested_match() {
    let src = r#"
    let main = fn () -> unit do
        let x: Result<Option<i64>, string> = Ok(val: Some(val: 10))
        match x do
            | Ok(val: Some(val: v)) -> return ()
            | Ok(val: None) -> return ()
            | Err(err: e) -> return ()
        end
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_anonymous_record_field_access() {
    // Anonymous records support dot-access
    let src = r#"
    let main = fn () -> unit do
        let r = { x: 1, y: true, z: "hi" }
        let a = r.x
        let b = r.y
        let c = r.z
        return ()
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_named_record_destructure() {
    // Named type records use destructuring for field access
    let src = r#"
    type Point = { x: i64, y: i64 }

    let main = fn () -> unit do
        let p: Point = { x: 10, y: 20 }
        let { x: a, y: b } = p
        return ()
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_higher_order_function() {
    let src = r#"
    let apply = fn <A, B>(f: (x: A) -> B, x: A) -> B do
        return f(x: x)
    end

    let inc = fn (x: i64) -> i64 do return x + 1 end

    let main = fn () -> unit do
        let result = apply(f: inc, x: 41)
        return ()
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_recursive_list() {
    let src = r#"
    let main = fn () -> unit do
        let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Nil)))
        match xs do
            | Cons(v: h, rest: t) -> return ()
            | Nil -> return ()
        end
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_if_else_expression() {
    // Comparison operators may need stdlib, use exec_with_stdlib
    let src = r#"
    let max = fn (a: i64, b: i64) -> i64 do
        return if a > b then a else b end
    end

    let main = fn () -> unit do
        let m = max(a: 10, b: 20)
        return ()
    end
    "#;
    should_typecheck(src);
    exec_with_stdlib(src);
}

#[test]
fn soundness_while_loop() {
    let src = r#"
    let main = fn () -> unit do
        let ~i = 0
        while ~i < 5 do
            ~i <- ~i + 1
        end
        return ()
    end
    "#;
    should_typecheck(src);
    exec_with_stdlib(src);
}

#[test]
fn soundness_string_concat() {
    let src = r#"
    let main = fn () -> unit do
        let s = "hello" ++ " " ++ "world"
        return ()
    end
    "#;
    should_typecheck(src);
    exec_with_stdlib(src);
}

#[test]
fn soundness_linear_resource_borrow_and_consume() {
    let src = r#"
    let peek = fn (x: &i64) -> i64 do return x end

    let main = fn () -> unit do
        let %r = 42
        let r_ref = &%r
        let v = peek(x: r_ref)
        match %r do | _ -> () end
        return ()
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_ref_cell_mutation() {
    let src = r#"
    let main = fn () -> unit do
        let ~counter = 0
        ~counter <- ~counter + 1
        ~counter <- ~counter + 1
        ~counter <- ~counter + 1
        return ()
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_float_arithmetic() {
    let src = r#"
    let main = fn () -> unit do
        let x = 1.5 +. 2.5
        let y = x *. 2.0
        let z = y -. 1.0
        let w = z /. 2.0
        return ()
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_exception_try_catch() {
    let src = r#"
    exception Boom(code: i64)

    let main = fn () -> unit do
        try
            raise Boom(code: 42)
        catch
            | Boom(code: code) -> return ()
            | _ -> return ()
        end
        return ()
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_array_create_and_borrow() {
    let src = r#"
    import * as array from "std:array"

    let main = fn () -> unit do
        let %arr = [| 1, 2, 3 |]
        let arr_ref = &%arr
        let n = array.length(arr: arr_ref)
        match %arr do | _ -> () end
        return ()
    end
    "#;
    should_typecheck(src);
    exec_with_stdlib(src);
}

#[test]
fn soundness_list_operations() {
    let src = r#"
    import * as list from "std:list"

    let main = fn () -> unit do
        let xs = [1, 2, 3]
        let n = list.length(xs: xs)
        return ()
    end
    "#;
    should_typecheck(src);
    exec_with_stdlib(src);
}

#[test]
fn soundness_string_stdlib() {
    let src = r#"
    import { from_i64, length } from "std:string_ops"

    let main = fn () -> unit do
        let s = from_i64(val: 42)
        let n = length(s: s)
        return ()
    end
    "#;
    should_typecheck(src);
    exec_with_stdlib(src);
}

#[test]
fn soundness_implicit_unit_return() {
    let src = r#"
    let side_effect = fn () -> unit do
        let x = 1 + 2
    end

    let main = fn () -> unit do
        side_effect()
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_match_all_return_diverge() {
    let src = r#"
    let f = fn (x: i64) -> i64 do
        match x do
            | 0 -> return 100
            | _ -> return 200
        end
    end

    let main = fn () -> unit do
        let r = f(x: 0)
        return ()
    end
    "#;
    should_typecheck(src);
    exec(src);
}

#[test]
fn soundness_generic_box_destructure() {
    // Use destructuring for named record types, not dot access
    let src = r#"
    type Box<T> = { val: T }

    let unbox = fn <T>(b: Box<T>) -> T do
        let { val: v } = b
        return v
    end

    let main = fn () -> unit do
        let b: Box<i64> = { val: 42 }
        let v = unbox(b: b)
        return ()
    end
    "#;
    should_typecheck(src);
    exec(src);
}

// ---------------------------------------------------------------------------
// Proptest: random well-typed programs must compile and run
// ---------------------------------------------------------------------------

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_soundness_arithmetic(a in -1000i64..1000, b in -1000i64..1000) {
        let src = format!(r#"
let main = fn () -> unit do
    let x = {} + {}
    let y = {} - {}
    let z = {} * {}
    return ()
end
"#, a, b, a, b, a, b);
        should_typecheck(&src);
        exec(&src);
    }

    #[test]
    fn prop_soundness_bool_match(val in any::<bool>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let b = {}
    match b do
        | true -> return ()
        | false -> return ()
    end
end
"#, val);
        should_typecheck(&src);
        exec(&src);
    }

    #[test]
    fn prop_soundness_option_roundtrip(n in any::<i64>()) {
        let src = format!(r#"
let extract = fn (opt: Option<i64>) -> i64 do
    match opt do
        | Some(val: v) -> return v
        | None -> return 0
    end
end

let main = fn () -> unit do
    let x = extract(opt: Some(val: {n}))
    let y = extract(opt: None)
    return ()
end
"#, n = n);
        should_typecheck(&src);
        exec(&src);
    }

    #[test]
    fn prop_soundness_nested_if_else(a in -100i64..100, b in -100i64..100, c in -100i64..100) {
        let src = format!(r#"
let classify = fn (x: i64) -> i64 do
    return if x > 0 then
        if x > 50 then 2 else 1 end
    else
        0
    end
end

let main = fn () -> unit do
    let r1 = classify(x: {a})
    let r2 = classify(x: {b})
    let r3 = classify(x: {c})
    return ()
end
"#, a = a, b = b, c = c);
        should_typecheck(&src);
        exec_with_stdlib(&src);
    }
}
