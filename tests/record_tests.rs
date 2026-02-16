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
fn test_anonymous_record() {
    let src = r#"
    fn main() -> unit effect { IO } do
        let r = { x: 1, y: "hello" }
        let i = r.x
        // let s = r.y // Type of s is Str. Unused variable? (No check yet)
        perform print(val: "i=" ++ to_string(val: i))
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_record_unification() {
    // Structural typing: Order should not matter
    let src = r#"
    fn take_record(r: { x: i64, y: i64 }) -> unit do
        return ()
    endfn

    fn main() -> unit do
        let r1 = { x: 1, y: 2 }
        let r2 = { y: 2, x: 1 } // Different order
        take_record(r: r1)
        take_record(r: r2)
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_record_fail() {
    let src = r#"
    fn main() -> unit do
        let r = { x: 1 }
        let y = r.y // Field missing
        return ()
    endfn
    "#;
    assert!(check(src).is_err());
}
