use chumsky::Parser;
use nexus::parser;
use nexus::typecheck::TypeChecker;

fn check_code(src: &str) -> Result<(), String> {
    let parser = parser::parser();
    let program = parser.parse(src).map_err(|e| format!("{:?}", e))?;

    let mut checker = TypeChecker::new();
    checker.check_program(&program)
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
        return id(x: id(x: 10))
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
        let b = weird(x: "string")
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
