use chumsky::Parser;
use nexus::lang::parser;
use nexus::lang::typecheck::TypeChecker;

fn check_code(src: &str) -> Result<(), String> {
    let src = src
        .replace(
            "let main = fn () -> i64 do",
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
    let parser = parser::parser();
    let program = parser.parse(src).map_err(|e| format!("{:?}", e))?;

    let mut checker = TypeChecker::new();
    checker.check_program(&program).map_err(|e| e.message)
}

#[test]
fn test_basic_poly() {
    let src = r#"
    let id = fn <T>(x: T) -> T do
        return x
    endfn

    let main = fn () -> unit do
        let i = id(x: 10)
        let b = id(x: true)
        return ()
    endfn
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
    let id = fn <T>(x: T) -> T do return x endfn
    let main = fn () -> i64 do
        let v = id(x: 10)
        return id(x: v)
    endfn
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_label_punning_is_rejected() {
    let src = r#"
    let id = fn <T>(x: T) -> T do return x endfn
    let main = fn () -> i64 do
        let x = 10
        return id(x)
    endfn
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
    endfn
    "#;
    let parser = parser::parser();
    assert!(
        parser.parse(src).is_err(),
        "enum declaration syntax should not be accepted; use type sum syntax"
    );
}

#[test]
fn test_two_generics() {
    let src = r#"
    let first = fn <A, B>(a: A, b: B) -> A do
        return a
    endfn

    let main = fn () -> i64 do
        let f = first(a: 10, b: true)
        let s = first(a: true, b: 10)
        return f
    endfn
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
    endfn

    let main = fn () -> i64 do
        // Since my infer for Record currently returns AnonymousRecord, 
        // we can't test full record inference yet, but unbox signature check works.
        return 42
    endfn
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
    endfn

    let main = fn () -> i64 do
        let f = id
        let a = f(x: 10)
        let b = f(x: true)
        return a
    endfn
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
    endfn

    let main = fn () -> unit do
        let a = weird(x: 1)
        let b = weird(x: [=[string]=])
        return ()
    endfn
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
        endmatch
        return ()
    endfn
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_type_sum_definition_with_labeled_variant_fields() {
    let src = r#"
    type Result<T, E> = Ok(val: T) | Err(err: E)

    let unwrap_or = fn (r: Result<i64, i64>, fallback: i64) -> i64 do
        match r do
            case Ok(val: v) -> return v
            case Err(err: e) -> return fallback
        endmatch
    endfn

    let main = fn () -> i64 do
        let a: Result<i64, i64> = Ok(val: 10)
        let b: Result<i64, i64> = Err(err: 1)
        let x = unwrap_or(r: a, fallback: 0)
        let y = unwrap_or(r: b, fallback: 7)
        return x + y
    endfn
    "#;
    assert!(check_code(src).is_ok());
}

#[test]
fn test_arg_mismatch() {
    let src = r#"
    let foo = fn (x: i64) -> i64 do return x endfn
    let main = fn () -> i64 do
        return foo(x: true)
    endfn
    "#;
    assert!(check_code(src).is_err());
}

// test_label_mismatch: covered by prop_named_argument_label_mismatch_is_error

#[test]
fn test_int_literal_defaults_to_i64() {
    let src = r#"
    let main = fn () -> i64 do
        let x = 1
        return x
    endfn
    "#;
    assert!(check_code(src).is_ok());
}

#[test]
fn test_int_literal_is_not_i32_without_annotation() {
    let src = r#"
    let main = fn () -> i32 do
        let x = 1
        return x
    endfn
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
    endfn
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
    endfn
    "#;
    assert!(check_code(src).is_ok());
}

#[test]
fn test_named_function_can_be_used_as_value() {
    let src = r#"
    let id = fn (x: i64) -> i64 do
        return x
    endfn

    let main = fn () -> i64 do
        let f = id
        return f(x: 42)
    endfn
    "#;
    assert!(check_code(src).is_ok());
}

#[test]
fn test_inline_lambda_literal_typechecks() {
    let src = r#"
    let main = fn () -> i64 do
        let f = fn (x: i64) -> i64 do
            return x + 1
        endfn
        return f(x: 41)
    endfn
    "#;
    assert!(check_code(src).is_ok());
}

#[test]
fn test_lambda_cannot_capture_ref() {
    let src = r#"
    let main = fn () -> i64 do
        let ~counter = 1
        let read_counter = fn () -> i64 do
            return ~counter
        endfn
        return read_counter()
    endfn
    "#;
    let err = check_code(src).unwrap_err();
    assert!(
        err.contains("capture Ref"),
        "expected capture Ref error, got: {}",
        err
    );
}

#[test]
fn test_linear_capture_makes_lambda_linear_and_single_use() {
    let src = r#"
    let main = fn () -> i64 do
        let %x = 7
        let f = fn () -> i64 do
            match %x do case _ -> () endmatch
            return 1
        endfn
        let y = f()
        return y
    endfn
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_linear_capturing_lambda_cannot_be_called_twice() {
    let src = r#"
    let main = fn () -> i64 do
        let %x = 7
        let f = fn () -> i64 do
            match %x do case _ -> () endmatch
            return 1
        endfn
        let _a = f()
        let _b = f()
        return 0
    endfn
    "#;
    let err = check_code(src).unwrap_err();
    assert!(
        err.contains("already consumed"),
        "expected linear consume error, got: {}",
        err
    );
}

#[test]
fn test_recursive_lambda_with_annotation_typechecks() {
    let src = r#"
    let main = fn () -> i64 do
        let fact: (n: i64) -> i64 = fn (n: i64) -> i64 do
            if n == 0 then
                return 1
            else
                let n1 = n - 1
                let rec = fact(n: n1)
                return n * rec
            endif
        endfn
        return fact(n: 5)
    endfn
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_constructor_arity_error_is_llm_friendly() {
    let src = r#"
    type Pair = Pair(left: i64, right: i64)

    let main = fn () -> i64 do
        let _p = Pair(left: 1)
        return 0
    endfn
    "#;
    let err = check_code(src).unwrap_err();
    assert!(
        err.contains("Arity mismatch in constructor `Pair`"),
        "expected contextual constructor arity error, got: {}",
        err
    );
    assert!(
        err.contains("Expected fields:"),
        "expected expected-fields section, got: {}",
        err
    );
    assert!(
        err.contains("Provided arguments:"),
        "expected provided-arguments section, got: {}",
        err
    );
}

#[test]
fn test_constructor_pattern_arity_error_is_llm_friendly() {
    let src = r#"
    type Pair = Pair(left: i64, right: i64)

    let main = fn () -> i64 do
        let p: Pair = Pair(left: 1, right: 2)
        match p do
            case Pair(left: x) -> return x
        endmatch
    endfn
    "#;
    let err = check_code(src).unwrap_err();
    assert!(
        err.contains("Arity mismatch in pattern `Pair`"),
        "expected contextual pattern arity error, got: {}",
        err
    );
    assert!(
        err.contains("Provided pattern arguments:"),
        "expected provided-pattern-arguments section, got: {}",
        err
    );
}

#[test]
fn test_binary_op_in_call_arg() {
    let src = r#"
    let add = fn (a: i64, b: i64) -> i64 do
        return a + b
    endfn

    let main = fn () -> i64 do
        return add(a: 1 + 2, b: 3 * 4)
    endfn
    "#;
    assert!(check_code(src).is_ok(), "binary ops should be allowed in call args");
}

#[test]
fn test_string_concat_in_call_arg() {
    let src = r#"
    let greet = fn (msg: string) -> string do
        return msg
    endfn

    let main = fn () -> string do
        return greet(msg: [=[hello ]=] ++ [=[world]=])
    endfn
    "#;
    assert!(check_code(src).is_ok(), "string concat should be allowed in call args");
}

#[test]
fn test_function_arity_mismatch_shows_expected() {
    let src = r#"
    let add = fn (a: i64, b: i64) -> i64 do
        return a + b
    endfn

    let main = fn () -> i64 do
        return add(a: 1)
    endfn
    "#;
    let err = check_code(src).unwrap_err();
    assert!(
        err.contains("expected 2") || err.contains("Expected"),
        "arity mismatch error should mention expected count, got: {}",
        err
    );
    assert!(
        err.contains("got 1") || err.contains("Provided"),
        "arity mismatch error should mention provided count, got: {}",
        err
    );
}

#[test]
fn test_function_arity_mismatch_too_many_args() {
    let src = r#"
    let inc = fn (x: i64) -> i64 do
        return x + 1
    endfn

    let main = fn () -> i64 do
        return inc(x: 1, y: 2)
    endfn
    "#;
    let err = check_code(src).unwrap_err();
    assert!(
        err.contains("expected 1") || err.contains("Expected"),
        "arity mismatch error should mention expected count, got: {}",
        err
    );
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
