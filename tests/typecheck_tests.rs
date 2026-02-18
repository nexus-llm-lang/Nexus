use chumsky::Parser;
use nexus::parser;
use nexus::typecheck::TypeChecker;

fn check_code(src: &str) -> Result<(), String> {
    let parser = parser::parser();
    let program = parser.parse(src).map_err(|e| format!("{:?}", e))?;

    let mut checker = TypeChecker::new();
    checker.check_program(&program).map_err(|e| e.message)
}

#[test]
fn test_basic_poly() {
    let src = r#"
    fn id<T>(x: T) -> T do
        return x
    endfn

    fn main() -> unit do
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

#[test]
fn test_poly_instantiation_int() {
    let src = r#"
    fn id<T>(x: T) -> T do return x endfn
    fn main() -> i64 do
        return id(x: 42)
    endfn
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_poly_instantiation_bool() {
    let src = r#"
    fn id<T>(x: T) -> T do return x endfn
    fn main() -> bool do
        return id(x: true)
    endfn
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_poly_mismatch() {
    let src = r#"
    fn id<T>(x: T) -> T do return x endfn
    fn main() -> bool do
        return id(x: 10)
    endfn
    "#;
    let res = check_code(src);
    assert!(res.is_err());
}

#[test]
fn test_nested_calls() {
    let src = r#"
    fn id<T>(x: T) -> T do return x endfn
    fn main() -> i64 do
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
fn test_labeled_arg_punning() {
    let src = r#"
    fn id<T>(x: T) -> T do return x endfn
    fn main() -> i64 do
        let x = 10
        return id(x)
    endfn
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_two_generics() {
    let src = r#"
    fn first<A, B>(a: A, b: B) -> A do
        return a
    endfn

    fn main() -> i64 do
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
    
    fn unbox<T>(b: Box<T>) -> T do
        return b.val
    endfn

    fn main() -> i64 do
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
    fn id<T>(x: T) -> T do
        return x
    endfn

    fn main() -> i64 do
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
    fn weird<T>(x: T) -> T do
        return x
    endfn

    fn main() -> unit do
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
    fn main() -> unit do
        let r1 = Ok(1)
        let r2 = Ok(true)
        // Check match on poly variants
        match r1 do
            case Ok(v) -> let x = v + 1
            case Err(e) -> ()
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
fn test_arg_mismatch() {
    let src = r#"
    fn foo(x: i64) -> i64 do return x endfn
    fn main() -> i64 do
        return foo(x: true)
    endfn
    "#;
    assert!(check_code(src).is_err());
}

#[test]
fn test_label_mismatch() {
    let src = r#"
    fn foo(x: i64) -> i64 do return x endfn
    fn main() -> i64 do
        return foo(y: 10) // Fails due to label 'y'
    endfn
    "#;
    assert!(check_code(src).is_err());
}

#[test]
fn test_int_literal_defaults_to_i64() {
    let src = r#"
    fn main() -> i64 do
        let x = 1
        return x
    endfn
    "#;
    assert!(check_code(src).is_ok());
}

#[test]
fn test_int_literal_is_not_i32_without_annotation() {
    let src = r#"
    fn main() -> i32 do
        let x = 1
        return x
    endfn
    "#;
    assert!(check_code(src).is_err());
}

#[test]
fn test_int_literal_annotation_can_select_i32() {
    let src = r#"
    fn main() -> i32 do
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
    fn main() -> f32 do
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
    fn id(x: i64) -> i64 do
        return x
    endfn

    fn main() -> i64 do
        let f = id
        return f(x: 42)
    endfn
    "#;
    assert!(check_code(src).is_ok());
}

#[test]
fn test_inline_lambda_literal_typechecks() {
    let src = r#"
    fn main() -> i64 do
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
    fn main() -> i64 do
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
    fn main() -> i64 do
        let %x = 7
        let f = fn () -> i64 do
            drop %x
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
    fn main() -> i64 do
        let %x = 7
        let f = fn () -> i64 do
            drop %x
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
    fn main() -> i64 do
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
