use chumsky::Parser;
use nexus::parser::parser;
use nexus::typecheck::TypeChecker;

fn check(src: &str) -> Result<(), String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)
}

#[test]
fn test_linear_basic_pass() {
    let src = r#"
    fn consume(x: %i64) -> unit do
        drop x
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
fn test_linear_param_accepts_plain_value_via_weakening() {
    let src = r#"
    fn consume(x: %i64) -> i64 do
        drop x
        return 1
    endfn

    fn main() -> i64 do
        return consume(x: 10)
    endfn
    "#;
    assert!(check(src).is_ok());
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
    assert!(
        check(src).is_err(),
        "Should fail because _ (bound to linear) is unused"
    );
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
    assert!(
        check(src).is_err(),
        "Should fail because Ref cannot contain Linear type"
    );
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
    assert!(
        check(src).is_err(),
        "Should fail because wildcard match drops linear value"
    );
}

#[test]
fn test_linear_borrow_basic() {
    let src = r#"
    fn peek(x: &i64) -> unit effect { IO } do
        let msg = i64_to_string(val: x)
        perform print(val: msg)
        return ()
    endfn

    fn main() -> unit effect { IO } do
        let %x = 10
        let x_ref1 = borrow %x
        perform peek(x: x_ref1)
        let x_ref2 = borrow %x
        perform peek(x: x_ref2) // Borrow again
        drop %x    // Finally consume
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
    assert!(
        check(src).is_err(),
        "Should fail because %x is not consumed in else branch"
    );
}

#[test]
fn test_generic_drop_accepts_non_linear_primitives() {
    let src = r#"
    fn main() -> unit do
        let x: i32 = 1
        let y: f64 = 2.0
        let s = [=[hello]=]
        drop x
        drop y
        drop s
        drop true
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_generic_drop_user_defined_linear_consumes_once() {
    let src = r#"
    type Token = {
        id: i64
    }

    fn main() -> unit do
        let %t: Token = { id: 1 }
        drop %t
        drop %t
        return ()
    endfn
    "#;
    assert!(check(src).is_err());
}

#[test]
fn test_enum_constructor_with_linear_arg_requires_consumption() {
    let src = r#"
    enum Resource {
        Open([| i64 |]),
        Closed
    }

    fn main() -> unit do
        let r = Open([| 1, 2, 3 |])
        return ()
    endfn
    "#;
    assert!(check(src).is_err());
}

#[test]
fn test_enum_constructor_with_linear_arg_can_be_dropped_once() {
    let src = r#"
    enum Resource {
        Open([| i64 |]),
        Closed
    }

    fn main() -> unit do
        let r = Open([| 1, 2, 3 |])
        drop r
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}
