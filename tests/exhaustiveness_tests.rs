use nexus::ast::{Type, Program, Sigil};
use nexus::typecheck::{TypeChecker, TypeEnv};
use nexus::parser::parser;
use chumsky::Parser;

fn check(src: &str) -> Result<(), String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p)
}

#[test]
fn test_nested_result_exhaustive() {
    let src = r#"
    fn main() -> unit do
        let x: Result<Result<i64, i64>, i64> = Ok(Ok(1))
        match x do
            case Ok(Ok(v)) -> return ()
            case Ok(Err(e)) -> return ()
            case Err(e) -> return ()
        endmatch
    endfn
    "#;
    if let Err(e) = check(src) {
        panic!("Failed: {}", e);
    }
}

#[test]
fn test_nested_result_non_exhaustive() {
    let src = r#"
    fn main() -> unit do
        let x: Result<Result<i64, i64>, i64> = Ok(Ok(1))
        match x do
            case Ok(Ok(v)) -> return ()
            // Missing Ok(Err(_)) case
            case Err(e) -> return ()
        endmatch
    endfn
    "#;
    assert!(check(src).is_err(), "Should fail due to missing Ok(Err(_)) case");
}

#[test]
fn test_bool_exhaustive() {
    let src = r#"
    fn main() -> unit do
        let b = true
        match b do
            case true -> return ()
            case false -> return ()
        endmatch
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_bool_non_exhaustive() {
    let src = r#"
    fn main() -> unit do
        let b = true
        match b do
            case true -> return ()
            // Missing false
        endmatch
    endfn
    "#;
    assert!(check(src).is_err(), "Should fail due to missing false case");
}

#[test]
fn test_wildcard_exhaustive() {
    let src = r#"
    fn main() -> unit do
        let i = 10
        match i do
            case 0 -> return ()
            case _ -> return ()
        endmatch
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_int_non_exhaustive() {
    let src = r#"
    fn main() -> unit do
        let i = 10
        match i do
            case 0 -> return ()
            case 1 -> return ()
            // Missing wildcard for integer
        endmatch
    endfn
    "#;
    assert!(check(src).is_err(), "Should fail because int cannot be exhausted by literals");
}

#[test]
fn test_record_exhaustive() {
    let src = r#"
    fn main() -> unit do
        let r = { x: true, y: true }
        match r do
            case { x: true, y: true } -> return ()
            case { x: true, y: false } -> return ()
            case { x: false, _ } -> return ()
        endmatch
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_record_non_exhaustive() {
    let src = r#"
    fn main() -> unit do
        let r = { x: true, y: true }
        match r do
            case { x: true, y: true } -> return ()
            // Missing cases
        endmatch
    endfn
    "#;
    assert!(check(src).is_err(), "Should fail");
}
