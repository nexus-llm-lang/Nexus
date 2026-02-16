use nexus::ast::{Type, Program, Sigil};
use nexus::typecheck::{TypeChecker, TypeEnv};
use nexus::parser::parser;
use chumsky::Parser;

fn check(src: &str) -> Result<(), String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)
}

#[test]
fn test_float_arithmetic() {
    let src = r#"
    fn main() -> unit do
        let x = 1.5 +. 2.5
        let y = x *. 2.0
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_float_compare() {
    let src = r#"
    fn main() -> unit do
        let b = 1.0 <. 2.0
        if b then return () else return () endif
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_float_int_mismatch() {
    let src = r#"
    fn main() -> unit do
        let x = 1 +. 2.0
        return ()
    endfn
    "#;
    assert!(check(src).is_err(), "Should fail: mixing int and float with float op");
}

#[test]
fn test_float_literal_type() {
    let src = r#"
    fn main() -> unit do
        let x: float = 3.14
        let y: float = 0.01
        let z: float = 123.456789
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}
