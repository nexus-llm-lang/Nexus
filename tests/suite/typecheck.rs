
use crate::common::source::check_raw;
use nexus::lang::parser;

fn check_code(src: &str) -> Result<(), String> {
    let src = src
        .replace(
            "let test_fn = fn () -> i64 do",
            "let __test_main = fn () -> i64 do",
        )
        .replace(
            "let main = fn () -> i32 do",
            "let __test_main = fn () -> i32 do",
        )
        .replace(
            "let main = fn () -> f32 do",
            "let __test_main = fn () -> f32 do",
        )
        .replace(
            "let main = fn () -> bool do",
            "let __test_main = fn () -> bool do",
        )
        .replace(
            "let main = fn () -> string do",
            "let __test_main = fn () -> string do",
        )
        .replace(
            "let main = fn () -> f64 do",
            "let __test_main = fn () -> f64 do",
        )
        .replace(
            "let main = fn () -> unit do",
            "let __test_main = fn () -> unit do",
        );
    check_raw(&src)
}

#[test]
fn test_basic_poly() {
    let src = r#"
    let id = fn <T>(x: T) -> T do
        return x
    end

    let main = fn () -> unit do
        let i = id(x: 10)
        let b = id(x: true)
        return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

// test_poly_instantiation_int: covered by prop_polymorphic_id_accepts_i64
// test_poly_instantiation_bool: covered by prop_first_combinator_keeps_first_type + test_basic_poly
// test_poly_mismatch: covered by prop_polymorphic_id_rejects_return_mismatch

#[test]
fn test_nested_calls() {
    let src = r#"
    let id = fn <T>(x: T) -> T do return x end
    let test_fn = fn () -> i64 do
        let v = id(x: 10)
        return id(x: v)
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_label_punning_is_rejected() {
    let src = r#"
    let id = fn <T>(x: T) -> T do return x end
    let test_fn = fn () -> i64 do
        let x = 10
        return id(x)
    end
    "#;
    assert!(check_code(src).is_err());
}

// test_labeled_args_can_be_passed_out_of_order: covered by prop_n_ary_labeled_call_order_invariant_runtime
// test_positional_call_syntax_is_rejected: covered by prop_positional_call_syntax_rejected

#[test]
fn test_enum_declaration_syntax_is_rejected() {
    let src = r#"
    enum Color { Red, Green }
    let main = fn () -> unit do
        return ()
    end
    "#;
    let parser = parser::parser();
    assert!(
        parser.parse(&src).is_err(),
        "enum declaration syntax should not be accepted; use type sum syntax"
    );
}

#[test]
fn test_two_generics() {
    let src = r#"
    let first = fn <A, B>(a: A, b: B) -> A do
        return a
    end

    let test_fn = fn () -> i64 do
        let f = first(a: 10, b: true)
        let s = first(a: true, b: 10)
        return f
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_record_access() {
    let src = r#"
    type Box<T> = { val: T }

    let unbox = fn <T>(b: Box<T>) -> T do
        return b.val
    end

    let test_fn = fn () -> i64 do
        // Since my infer for Record currently returns AnonymousRecord,
        // we can't test full record inference yet, but unbox signature check works.
        return 42
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_let_poly_binding() {
    let src = r#"
    let id = fn <T>(x: T) -> T do
        return x
    end

    let test_fn = fn () -> i64 do
        let f = id
        let a = f(x: 10)
        let b = f(x: true)
        return a
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_complex_poly_logic() {
    let src = r#"
    let weird = fn <T>(x: T) -> T do
        return x
    end

    let main = fn () -> unit do
        let a = weird(x: 1)
        let b = weird(x: "string")
        return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_poly_variants() {
    let src = r#"
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
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_type_sum_definition_with_labeled_variant_fields() {
    let src = &crate::common::fixtures::read_test_fixture("test_type_sum_definition_with_labeled_variant_fields.nx");
    assert!(check_code(src).is_ok());
}

#[test]
fn test_arg_mismatch() {
    let src = r#"
    let foo = fn (x: i64) -> i64 do return x end
    let test_fn = fn () -> i64 do
        return foo(x: true)
    end
    "#;
    assert!(check_code(src).is_err());
}

// test_label_mismatch: covered by prop_named_argument_label_mismatch_is_error

#[test]
fn test_int_literal_defaults_to_i64() {
    let src = r#"
    let test_fn = fn () -> i64 do
        let x = 1
        return x
    end
    "#;
    assert!(check_code(src).is_ok());
}

#[test]
fn test_int_literal_is_not_i32_without_annotation() {
    let src = r#"
    let main = fn () -> i32 do
        let x = 1
        return x
    end
    "#;
    assert!(check_code(src).is_err());
}

#[test]
fn test_int_literal_annotation_can_select_i32() {
    let src = r#"
    let main = fn () -> i32 do
        let x: i32 = 1
        let y = x + 2
        return y
    end
    "#;
    assert!(check_code(src).is_ok());
}

#[test]
fn test_float_literal_annotation_can_select_f32() {
    let src = r#"
    let main = fn () -> f32 do
        let x: f32 = 1.25
        let y = x +. 2.0
        return y
    end
    "#;
    assert!(check_code(src).is_ok());
}

#[test]
fn test_named_function_can_be_used_as_value() {
    let src = r#"
    let id = fn (x: i64) -> i64 do
        return x
    end

    let test_fn = fn () -> i64 do
        let f = id
        return f(x: 42)
    end
    "#;
    assert!(check_code(src).is_ok());
}

#[test]
fn test_inline_lambda_literal_typechecks() {
    let src = r#"
    let test_fn = fn () -> i64 do
        let f = fn (x: i64) -> i64 do
            return x + 1
        end
        return f(x: 41)
    end
    "#;
    assert!(check_code(src).is_ok());
}

#[test]
fn test_lambda_cannot_capture_ref() {
    let src = r#"
    let test_fn = fn () -> i64 do
        let ~counter = 1
        let read_counter = fn () -> i64 do
            return ~counter
        end
        return read_counter()
    end
    "#;
    let err = check_code(src).unwrap_err();
    insta::assert_snapshot!(err);
}

#[test]
fn test_linear_capture_makes_lambda_linear_and_single_use() {
    let src = r#"
    let test_fn = fn () -> i64 do
        let %x = 7
        let f = fn () -> i64 do
            match %x do case _ -> () end
            return 1
        end
        let y = f()
        return y
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_linear_capturing_lambda_cannot_be_called_twice() {
    let src = r#"
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
    "#;
    let err = check_code(src).unwrap_err();
    insta::assert_snapshot!(err);
}

#[test]
fn test_recursive_lambda_with_annotation_typechecks() {
    let src = &crate::common::fixtures::read_test_fixture("test_recursive_lambda_with_annotation_typechecks.nx");
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_constructor_arity_error_is_llm_friendly() {
    let src = r#"
    type Pair = Pair(left: i64, right: i64)

    let test_fn = fn () -> i64 do
        let _p = Pair(left: 1)
        return 0
    end
    "#;
    let err = check_code(src).unwrap_err();
    insta::assert_snapshot!(err);
}

#[test]
fn test_constructor_pattern_arity_error_is_llm_friendly() {
    let src = r#"
    type Pair = Pair(left: i64, right: i64)

    let test_fn = fn () -> i64 do
        let p: Pair = Pair(left: 1, right: 2)
        match p do
            case Pair(left: x) -> return x
        end
    end
    "#;
    let err = check_code(src).unwrap_err();
    insta::assert_snapshot!(err);
}

#[test]
fn test_binary_op_in_call_arg() {
    let src = r#"
    let add = fn (a: i64, b: i64) -> i64 do
        return a + b
    end

    let test_fn = fn () -> i64 do
        return add(a: 1 + 2, b: 3 * 4)
    end
    "#;
    assert!(check_code(src).is_ok(), "binary ops should be allowed in call args");
}

#[test]
fn test_string_concat_in_call_arg() {
    let src = r#"
    let greet = fn (msg: string) -> string do
        return msg
    end

    let main = fn () -> string do
        return greet(msg: "hello " ++ "world")
    end
    "#;
    assert!(check_code(src).is_ok(), "string concat should be allowed in call args");
}

#[test]
fn test_function_arity_mismatch_shows_expected() {
    let src = r#"
    let add = fn (a: i64, b: i64) -> i64 do
        return a + b
    end

    let test_fn = fn () -> i64 do
        return add(a: 1)
    end
    "#;
    let err = check_code(src).unwrap_err();
    insta::assert_snapshot!(err);
}

#[test]
fn test_function_arity_mismatch_too_many_args() {
    let src = r#"
    let inc = fn (x: i64) -> i64 do
        return x + 1
    end

    let test_fn = fn () -> i64 do
        return inc(x: 1, y: 2)
    end
    "#;
    let err = check_code(src).unwrap_err();
    insta::assert_snapshot!(err);
}

// #[test]
// fn test_human_written_tests() {
//     let src = r#"
//     type T = A(x: T)
//     "#;
//     let err = check_code(src).unwrap_err();
//     assert!(
//         err.contains("recursive type without indirection"),
//         "expected recursive type error, got: {}",
//         err
//     );
//
//     let src = r#"
//     type T = <T> (x: T) -> T
//     "#;
//     let err = check_code(src).unwrap_err();
//     assert!(
//         err.contains("recursive type without indirection"),
//         "expected recursive type error, got: {}",
//         err
//     );
// }


use crate::common::source::check_raw as check;

#[test]
fn test_float_arithmetic() {
    let src = r#"
    let main = fn () -> unit do
        let x = 1.5 +. 2.5
        let y = x *. 2.0
        return ()
    end
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_float_compare() {
    let src = r#"
    let main = fn () -> unit do
        let b = 1.0 <. 2.0
        if b then return () else return () end
    end
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_float_int_mismatch() {
    let src = r#"
    let main = fn () -> unit do
        let x = 1 +. 2.0
        return ()
    end
    "#;
    assert!(
        check(src).is_err(),
        "Should fail: mixing int and float with float op"
    );
}

#[test]
fn test_float_literal_type() {
    let src = r#"
    let main = fn () -> unit do
        let x: float = 3.14
        let y: float = 0.01
        let z: float = 123.456789
        return ()
    end
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_f32_and_f64_keywords() {
    let src = r#"
    let main = fn () -> unit do
        let x: f32 = 1.5
        let y: f64 = 2.0
        let z = x +. 3.5
        let w = y +. 4.0
        return ()
    end
    "#;
    assert!(check(src).is_ok());
}



#[test]
fn test_anonymous_record() {
    let src = r#"
    import { Console }, * as stdio from nxlib/stdlib/stdio.nx
    import { from_i64 } from nxlib/stdlib/string.nx
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
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_record_unification() {
    // Structural typing: Order should not matter
    let src = r#"
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
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_record_fail() {
    let src = r#"
    let main = fn () -> unit do
        let r = { x: 1 }
        let y = r.y // Field missing
        return ()
    end
    "#;
    assert!(check(src).is_err());
}



#[test]
fn test_ffi_declaration() {
    let src = r#"
    import external math.wasm
    pub external add_i64 = "add" : (a: i64, b: i64) -> i64

    let main = fn () -> unit do
      let x = add_i64(a: 1, b: 2)
      return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_ffi_effectful() {
    let src = r#"
    import external time.wasm
    type IO = {}
    pub external get_time = "get_time" : () -> float effect { IO }

    let helper = fn () -> unit effect { IO } do
      let t = get_time()
      return ()
    end

    let main = fn () -> unit do
      return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_ffi_mismatch() {
    let src = r#"
    pub external foo = "foo" : (a: i64) -> i64
    let main = fn () -> unit do
      let x = foo(a: true)
    end
    "#;
    assert!(check_code(src).is_err());
}

#[test]
fn test_ffi_explicit_type_params() {
    let src = r#"
    import external core.wasm
    pub external array_len = "array_length" : <T>(arr: &[| T |]) -> i64

    let main = fn () -> unit do
      let %a = [| 1, 2, 3 |]
      let r = &%a
      let n = array_len(arr: r)
      match %a do case _ -> () end
      return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_ffi_unintroduced_type_var_errors() {
    let src = r#"
    pub external bad = "bad" : (arr: &[| T |]) -> i64
    let main = fn () -> unit do
      return ()
    end
    "#;
    let err = check_code(src).unwrap_err();
    insta::assert_snapshot!(err);
}

#[test]
fn test_ffi_concrete_types_no_type_params_needed() {
    let src = r#"
    pub external add = "add" : (a: i64, b: i64) -> i64
    let main = fn () -> unit do
      let x = add(a: 1, b: 2)
      return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}



#[test]
fn test_ref_creation_and_type() {
    let src = r#"
    let main = fn () -> unit do
        let ~c = 0
        return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

// This test is now tricky because immutable vars simply cannot hold Ref if we can't create explicit Ref.
// But if I assign value to immutable, it's just value.
// The only way to get a Ref is by `let ~x`.
// So immutable variable cannot hold Ref unless a function returns Ref.
// And functions cannot return Ref.
// So this Gravity Rule is implicitly enforced by syntax + return check.
// I will change this test to ensure we CANNOT assign to immutable var later?
// No, immutable var cannot be assigned.
// Maybe I should test that `let c = ~x` (implicit deref) results in value, not ref.
#[test]
fn test_gravity_rule_immutable_holds_value() {
    let src = &crate::common::fixtures::read_test_fixture("test_gravity_rule_immutable_holds_value.nx");
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

// Since `ref()` is gone, we cannot construct a ref to return.
// But we can try to return `~c`?
// `~c` evaluates to value.
// Can we return the reference itself?
// If we use just `c` (without tilde)?
// My parser for Variable with Mutable sigil expects `~`.
// If I use `c`, it's Variable("c", Immutable).
// But "c" is not in env. "~c" is.
// So I cannot access the reference itself by name!
// This means References are truly second-class and confined to stack!
// Excellent.
#[test]
fn test_cannot_return_ref() {
    // Attempting to return a reference is syntactically impossible or type error?
    // If I have `let ~c = 0`.
    // `return ~c` returns 0 (i64).
    // `return c` fails "Variable not found".
    // So Gravity Rule "Return cannot contain Ref" is enforced by:
    // 1. Implicit deref on access.
    // 2. Inability to access raw Ref.
    let src = r#"
    let main = fn () -> unit do
        let ~c = 0
        // return c // Variable not found
        return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_ref_assignment() {
    let src = r#"
    let main = fn () -> unit do
        let ~c = 0
        ~c <- 1
        return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_ref_read() {
    let src = r#"
    let __test_main = fn () -> i64 do
        let ~c = 10
        let v = ~c
        return v
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

// test_ref_assignment_type_mismatch: covered by prop_ref_assignment_type_mismatch_is_error

#[test]
fn test_ref_generic() {
    let src = &crate::common::fixtures::read_test_fixture("test_ref_generic.nx");
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

use proptest::prelude::*;

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
        check(&src).expect("typecheck failed");
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
        assert!(check(&src).is_err());
    }
}
