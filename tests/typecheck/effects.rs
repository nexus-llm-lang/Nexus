use crate::harness::{should_fail_typecheck, should_typecheck};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Deterministic tests (from throws.rs)
// ---------------------------------------------------------------------------

#[test]
fn test_throws_propagation() {
    should_typecheck(
        r#"
    type IO = {}

    let f = fn () -> unit throws { IO } do
        return ()
    end

    let g = fn () -> unit throws { IO } do
        f()
    end

    let main = fn () -> unit do
        return ()
    end
    "#,
    );
}

#[test]
fn test_call_pure_from_impure() {
    should_typecheck(
        r#"
    type IO = {}
    let pure_fn = fn () -> unit do return () end
    let impure_fn = fn () -> unit throws { IO } do
        pure_fn()
    end
    let main = fn () -> unit do
        return ()
    end
    "#,
    );
}

#[test]
fn test_try_catch_removes_exn() {
    should_typecheck(
        r#"
    import { Console }, * as stdio from "stdlib/stdio.nx"
    import { from_i64 } from "stdlib/string.nx"
    exception Oops(string)

    let risky = fn () -> unit throws { Exn } do
        raise Oops("oops")
        return ()
    end

    let main = fn () -> unit require { PermConsole } do
        inject stdio.system_handler do
            try
                risky()
            catch e ->
                match e do
                    case Oops(msg) -> Console.print(val: msg)
                    case RuntimeError(msg) -> Console.print(val: msg)
                    case InvalidIndex(i) ->
                        let m = from_i64(val: i)
                        Console.print(val: m)
                end
            end
        end
        return ()
    end
    "#,
    );
}

#[test]
fn test_raise_requires_exn() {
    should_fail_typecheck(
        r#"
    let fail = fn () -> unit do
        raise "oops" // Should fail: no Exn in throws clause
        return ()
    end
    let main = fn () -> unit do
        return ()
    end
    "#,
    );
}

#[test]
fn test_main_cannot_declare_exn_throws() {
    should_fail_typecheck(
        r#"
    let main = fn () -> unit throws { Exn } do
        return ()
    end
    "#,
    );
}

#[test]
fn test_main_must_return_unit() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> i64 do
        return 0
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn test_main_throws_net_only_is_rejected() {
    let err = should_fail_typecheck(
        r#"
    type Net = {}
    let main = fn () -> unit throws { Net } do
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn test_main_require_known_perm_is_accepted() {
    should_typecheck(
        r#"
    let main = fn () -> unit require { PermFs } do
        return ()
    end
    "#,
    );
}

#[test]
fn test_main_require_unknown_port_is_rejected() {
    should_fail_typecheck(
        r#"
    port Custom do
      fn foo() -> unit
    end
    let main = fn () -> unit require { Custom } do
        return ()
    end
    "#,
    );
}

#[test]
fn test_main_require_port_name_is_rejected() {
    should_fail_typecheck(
        r#"
    let main = fn () -> unit require { Net } do
        return ()
    end
    "#,
    );
}

#[test]
fn test_throws_mismatch() {
    should_fail_typecheck(
        r#"
    type IO = {}

    let f = fn () -> unit throws { IO } do
        return ()
    end

    let g = fn () -> unit throws {} do // Pure
        f()
    end

    let main = fn () -> unit do
        return ()
    end
    "#,
    );
}

#[test]
fn test_throws_polymorphism() {
    should_typecheck(
        r#"
    type IO = {}

    let apply = fn <E>(f: () -> unit throws E) -> unit throws E do
        f()
    end

    let pure_fn = fn () -> unit throws {} do
        return ()
    end

    let impure_fn = fn () -> unit throws { IO } do
        return ()
    end

    let test_pure = fn () -> unit throws {} do
        apply(f: pure_fn)
    end

    let test_impure = fn () -> unit throws { IO } do
        apply(f: impure_fn)
    end

    let main = fn () -> unit do
        return ()
    end
    "#,
    );
}

#[test]
fn test_throws_polymorphism_mismatch() {
    should_fail_typecheck(
        r#"
    type IO = {}

    let apply = fn <E>(f: () -> unit throws E) -> unit throws E do
        f()
    end

    let impure_fn = fn () -> unit throws { IO } do
        return ()
    end

    let test_fail = fn () -> unit throws {} do // Declared Pure
        apply(f: impure_fn)     // Call is Impure (IO)
    end

    let main = fn () -> unit do
        return ()
    end
    "#,
    );
}

// ---------------------------------------------------------------------------
// Property-based tests (typecheck-only)
// ---------------------------------------------------------------------------

fn io_program(body: &str) -> String {
    format!(
        r#"
type IO = {{}}

let io = fn (x: i64) -> unit throws {{ IO }} do
    return ()
end

let helper = fn () -> unit throws {{ IO }} do
{body}
    return ()
end

let main = fn () -> unit do
    return ()
end
"#
    )
}

fn pure_program(body: &str) -> String {
    format!(
        r#"
let pure = fn (x: i64) -> unit do
    return ()
end

let main = fn () -> unit do
{body}
    return ()
end
"#
    )
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_polymorphic_id_accepts_i64(n in any::<i64>()) {
        let src = format!(
            r#"
let id = fn <T>(x: T) -> T do
    return x
end

let __test_main = fn () -> i64 do
    return id(x: {n})
end
"#
        );
        should_typecheck(&src);
    }

    #[test]
    fn prop_polymorphic_id_rejects_return_mismatch(n in any::<i64>()) {
        let src = format!(
            r#"
let id = fn <T>(x: T) -> T do
    return x
end

let __test_main = fn () -> bool do
    return id(x: {n})
end
"#
        );
        should_fail_typecheck(&src);
    }

    #[test]
    fn prop_effectful_call_with_perform_is_ok(calls in 0usize..8) {
        let mut body = String::new();
        for _ in 0..calls {
            body.push_str("    io(x: 0)\n");
        }
        let src = io_program(&body);
        should_typecheck(&src);
    }

    #[test]
    fn prop_pure_call_without_perform_is_ok(calls in 0usize..8) {
        let mut body = String::new();
        for _ in 0..calls {
            body.push_str("    pure(x: 0)\n");
        }
        let src = pure_program(&body);
        should_typecheck(&src);
    }

    #[test]
    fn prop_first_combinator_keeps_first_type(n in any::<i64>(), b in any::<bool>()) {
        let src = format!(
            r#"
let first = fn <A, B>(a: A, b: B) -> A do
    return a
end

let __test_main = fn () -> i64 do
    return first(a: {n}, b: {b})
end
"#
        );
        should_typecheck(&src);
    }

    #[test]
    fn prop_named_argument_label_mismatch_is_error(n in any::<i64>()) {
        let src = format!(
            r#"
let f = fn (x: i64) -> i64 do
    return x
end

let __test_main = fn () -> i64 do
    return f(y: {n})
end
"#
        );
        should_fail_typecheck(&src);
    }

    #[test]
    fn prop_declared_pure_function_cannot_perform_io(calls in 1usize..8) {
        let mut body = String::new();
        for _ in 0..calls {
            body.push_str("    io(x: 0)\n");
        }
        let src = format!(
            r#"
type IO = {{}}

let io = fn (x: i64) -> unit throws {{ IO }} do
    return ()
end

let pure_wrap = fn (x: i64) -> unit throws {{}} do
{body}
    return ()
end

let main = fn () -> unit do
    pure_wrap(x: 0)
    return ()
end
"#
        );
        should_fail_typecheck(&src);
    }

    #[test]
    fn prop_raise_without_exn_throws_is_error(msg in "[a-zA-Z0-9_]{1,16}") {
        let src = format!(
            r#"
exception Fail(val: string)

let fail = fn () -> unit do
    raise Fail(val: "{msg}")
    return ()
end
"#
        );
        should_fail_typecheck(&src);
    }

    #[test]
    fn prop_try_catch_with_io_handler_typechecks(msg in "[a-zA-Z0-9_]{1,16}") {
        let src = format!(
            r#"
import {{ Console }}, * as stdio from "stdlib/stdio.nx"
import {{ from_i64 }} from "stdlib/string.nx"
exception MsgError(val: string)

let risky = fn (msg: string) -> unit throws {{ Exn }} do
    raise MsgError(val: msg)
    return ()
end

let main = fn () -> unit require {{ PermConsole }} do
    inject stdio.system_handler do
        try
            risky(msg: "{msg}")
        catch e ->
            match e do
                case MsgError(val: m) -> Console.print(val: m)
                case RuntimeError(val: m) -> Console.print(val: m)
                case InvalidIndex(val: i) ->
                    let m = from_i64(val: i)
                    Console.print(val: m)
            end
        end
    end
    return ()
end
"#
        );
        should_typecheck(&src);
    }

    #[test]
    fn prop_linear_array_borrow_then_drop_is_ok(xs in proptest::collection::vec(-100i64..=100, 1usize..4)) {
        let elems = xs
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let src = format!(
            r#"
import * as array from "stdlib/array.nx"

let __test_main = fn () -> i64 do
    let %arr = [| {elems} |]
    let arr_ref = &%arr
    let n = array.length(arr: arr_ref)
    match %arr do case _ -> () end
    return n
end
"#
        );
        should_typecheck(&src);
    }

    #[test]
    fn prop_ref_write_then_read_typechecks(a in any::<i64>(), b in any::<i64>()) {
        let src = format!(
            r#"
let __test_main = fn () -> i64 do
    let ~c = {a}
    ~c <- {b}
    let v = ~c
    return v
end
"#
        );
        should_typecheck(&src);
    }

    #[test]
    fn prop_ref_assignment_type_mismatch_is_error(n in any::<i64>()) {
        let src = format!(
            r#"
let main = fn () -> unit do
    let ~c = {n}
    ~c <- true
    return ()
end
"#
        );
        should_fail_typecheck(&src);
    }

    #[test]
    fn prop_immutable_assignment_is_error(a in any::<i64>(), b in any::<i64>()) {
        let src = format!(
            r#"
let main = fn () -> unit do
    let c = {a}
    c <- {b}
    return ()
end
"#
        );
        should_fail_typecheck(&src);
    }

    #[test]
    fn prop_linear_value_must_be_consumed_once(n in any::<i64>()) {
        let src = format!(
            r#"
let main = fn () -> unit do
    let %x = {n}
    match %x do case _ -> () end
    return ()
end
"#
        );
        should_typecheck(&src);
    }

    #[test]
    fn prop_linear_primitive_auto_drop_is_ok(n in any::<i64>()) {
        let src = format!(
            r#"
let main = fn () -> unit do
    let %x = {n}
    return ()
end
"#
        );
        // Primitive linear values are auto-dropped at scope end
        should_typecheck(&src);
    }

    #[test]
    fn prop_linear_double_consume_is_error(n in any::<i64>()) {
        let src = format!(
            r#"
let main = fn () -> unit do
    let %x = {n}
    match %x do case _ -> () end
    match %x do case _ -> () end
    return ()
end
"#
        );
        should_fail_typecheck(&src);
    }

    #[test]
    fn prop_linear_cannot_be_stored_in_ref(n in any::<i64>()) {
        let src = format!(
            r#"
let main = fn () -> unit do
    let %x = {n}
    let ~r = %x
    return ()
end
"#
        );
        should_fail_typecheck(&src);
    }

    #[test]
    fn prop_linear_borrow_then_consume_is_ok(n in any::<i64>()) {
        let src = format!(
            r#"
let peek = fn (x: &i64) -> i64 do
    return x
end

let __test_main = fn () -> i64 do
    let %x = {n}
    let x_ref1 = &%x
    let a = peek(x: x_ref1)
    let x_ref2 = &%x
    let b = peek(x: x_ref2)
    match %x do case _ -> () end
    return a + b
end
"#
        );
        should_typecheck(&src);
    }

    #[test]
    fn prop_linear_param_accepts_plain_value_via_weakening(n in any::<i64>()) {
        let src = format!(
            r#"
let consume = fn (x: %i64) -> i64 do
    match x do case _ -> () end
    return 1
end

let main = fn () -> unit do
    let y = consume(x: {n})
    match y do case _ -> () end
    return ()
end
"#
        );
        should_typecheck(&src);
    }

    #[test]
    fn prop_linear_branch_mismatch_is_error(n in any::<i64>(), cond in any::<bool>()) {
        // if-else where then consumes %x but else does not (non-diverging)
        let src = format!(
            r#"
let main = fn () -> unit do
    let %x = {n}
    if {cond} then
        match %x do case _ -> () end
    else
        ()
    end
    return ()
end
"#
        );
        should_fail_typecheck(&src);
    }
}
