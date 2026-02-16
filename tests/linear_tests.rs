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
fn test_linear_basic_pass() {
    let src = r#"
    fn consume(x: %i64) -> unit do
        drop_i64(val: x)
        return ()
    endfn

    fn main() -> unit do
        let %x = 10
        consume(x: %x)
        return ()
    endfn
    "#;
    match check(src) {
        Ok(_) => (),
        Err(e) => panic!("Failed: {}", e),
    }
}

#[test]
fn test_linear_wildcard_fail() {
    let src = r#"
    fn main() -> unit do
        let %x = 10
        let _ = %x // Consumes %x, but binds to _, which cannot be used
        // _ is linear, so it must be used, but cannot be referred to.
        // Thus, it should fail at end of scope.
        return ()
    endfn
    "#;
    assert!(check(src).is_err(), "Should fail because _ (bound to linear) is unused");
}

#[test]
fn test_linear_in_ref_fail() {
    let src = r#"
    fn main() -> unit do
        let %x = 10
        let ~r = %x // Creating Ref<Linear<I64>> should be forbidden
        return ()
    endfn
    "#;
    assert!(check(src).is_err(), "Should fail because Ref cannot contain Linear type");
}

#[test]
fn test_linear_match_wildcard_fail() {
    let src = r#"
    fn main() -> unit do
        let %x = 10
        match %x do
            case _ -> return () // Implicitly drops %x
        endmatch
    endfn
    "#;
    assert!(check(src).is_err(), "Should fail because wildcard match drops linear value");
}

#[test]
fn test_linear_borrow_basic() {
    let src = r#"
    fn peek(x: &i64) -> unit effect { IO } do
        perform print_i64(val: x)
        return ()
    endfn

    fn main() -> unit effect { IO } do
        let %x = 10
        perform peek(x: borrow %x)
        perform peek(x: borrow %x) // Borrow again
        drop_i64(val: %x)    // Finally consume
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_linear_unused_fail() {
    let src = r#"
    fn main() -> unit do
        let %x = 10
        // %x is not used
        return ()
    endfn
    "#;
    // Current implementation doesn't enforce linearity yet, so this might pass.
    // We expect it to FAIL once implemented.
    assert!(check(src).is_err(), "Should fail because %x is unused");
}

#[test]
fn test_linear_double_use_fail() {
    let src = r#"
    fn consume(x: %i64) -> unit do
        return ()
    endfn

    fn main() -> unit do
        let %x = 10
        consume(x: %x)
        consume(x: %x) // Double use
    endfn
    "#;
    assert!(check(src).is_err(), "Should fail because %x is used twice");
}

#[test]
fn test_linear_branch_mismatch() {
    let src = r#"
    fn consume(x: %i64) -> unit do
        return ()
    endfn

    fn main() -> unit do
        let %x = 10
        if true then
            consume(x: %x)
        else
            // %x not consumed here
            return ()
        endif
    endfn
    "#;
    assert!(check(src).is_err(), "Should fail because %x is not consumed in else branch");
}
