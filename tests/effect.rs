use chumsky::Parser;
use nexus::lang::parser::parser;
use nexus::lang::typecheck::TypeChecker;

fn check(src: &str) -> Result<(), String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)
}

#[test]
fn test_effect_propagation() {
    // f has IO effect. g calls f, so g must have IO effect.
    let src = r#"
    type IO = {} // Dummy type for effect

    let f = fn () -> unit effect { IO } do
        return ()
    endfn

    let g = fn () -> unit effect { IO } do
        perform f()
    endfn

    let main = fn () -> unit effect { IO } do
        perform g()
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_call_pure_from_impure() {
    let src = r#"
    type IO = {}
    let pure_fn = fn () -> unit do return () endfn
    let impure_fn = fn () -> unit effect { IO } do
        pure_fn() // Should be allowed without perform
        return ()
    endfn
    let main = fn () -> unit effect { IO } do
        perform impure_fn()
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_try_catch_removes_exn() {
    let src = r#"
    exception Oops(string)

    let risky = fn () -> unit effect { Exn } do
        raise Oops([=[oops]=])
        return ()
    endfn

    let main = fn () -> unit effect { IO } do
        try
            perform risky()
        catch e ->
            match e do
                case Oops(msg) -> perform print(val: msg)
                case RuntimeError(msg) -> perform print(val: msg)
                case InvalidIndex(i) ->
                    let m = i64_to_string(val: i)
                    perform print(val: m)
            endmatch
        endtry
        return ()
    endfn
    "#;
    if let Err(e) = check(src) {
        panic!("Typecheck failed: {}", e);
    }
}

#[test]
fn test_raise_requires_exn() {
    let src = r#"
    let fail = fn () -> unit do
        raise [=[oops]=] // Should fail: no Exn effect allowed
        return ()
    endfn
    "#;
    assert!(check(src).is_err());
}

#[test]
fn test_main_cannot_declare_exn_effect() {
    let src = r#"
    let main = fn () -> unit effect { IO, Exn } do
        return ()
    endfn
    "#;
    assert!(check(src).is_err());
}

#[test]
fn test_main_can_return_non_unit() {
    let src = r#"
    let main = fn () -> i64 do
        return 0
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_main_effect_net_only_is_rejected() {
    let src = r#"
    let main = fn () -> unit effect { Net } do
        return ()
    endfn
    "#;
    let err = check(src).expect_err("main with { Net } must fail");
    assert!(err.contains("main function effects must be one of"));
}

#[test]
fn test_main_effect_io_net_is_allowed() {
    let src = r#"
    let main = fn () -> unit effect { IO, Net } do
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_effect_mismatch() {
    // g is declared pure but calls f (IO). Should fail.
    let src = r#"
    type IO = {}

    let f = fn () -> unit effect { IO } do
        return ()
    endfn

    let g = fn () -> unit effect {} do // Pure
        perform f()
    endfn

    let main = fn () -> unit do
        return ()
    endfn
    "#;
    assert!(check(src).is_err(), "Should fail because g calls impure f");
}

#[test]
fn test_effect_polymorphism() {
    // apply is polymorphic in effect E.
    // Calling it with pure function -> result is pure.
    // Calling it with impure function -> result is impure.
    let src = r#"
    type IO = {}

    let apply = fn <E>(f: () -> unit effect E) -> unit effect E do
        perform f()
    endfn

    let pure_fn = fn () -> unit effect {} do
        return ()
    endfn

    let impure_fn = fn () -> unit effect { IO } do
        return ()
    endfn

    let test_pure = fn () -> unit effect {} do
        apply(f: pure_fn)
    endfn

    let test_impure = fn () -> unit effect { IO } do
        perform apply(f: impure_fn)
    endfn

    let main = fn () -> unit do
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_effect_polymorphism_mismatch() {
    // Calling apply with impure function in pure context.
    let src = r#"
    type IO = {}

    let apply = fn <E>(f: () -> unit effect E) -> unit effect E do
        perform f()
    endfn

    let impure_fn = fn () -> unit effect { IO } do
        return ()
    endfn

    let test_fail = fn () -> unit effect {} do // Declared Pure
        perform apply(f: impure_fn)     // Call is Impure (IO)
    endfn

    let main = fn () -> unit do
        return ()
    endfn
    "#;
    assert!(
        check(src).is_err(),
        "Should fail because apply instantiates E=IO, so call becomes IO"
    );
}
