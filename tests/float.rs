mod common;

use common::source::check_raw as check;

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
