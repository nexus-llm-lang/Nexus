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
fn test_effect_propagation() {
    // f has IO effect. g calls f, so g must have IO effect.
    let src = r#"
    type IO = {} // Dummy type for effect

    fn f() -> unit effect { IO } do
        return ()
    endfn

    fn g() -> unit effect { IO } do
        perform f()
    endfn

    fn main() -> unit effect { IO } do
        perform g()
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_effect_mismatch() {
    // g is declared pure but calls f (IO). Should fail.
    let src = r#"
    type IO = {}

    fn f() -> unit effect { IO } do
        return ()
    endfn

    fn g() -> unit effect {} do // Pure
        perform f()
    endfn

    fn main() -> unit do
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

    fn apply<E>(f: () -> unit effect E) -> unit effect E do
        perform f()
    endfn

    fn pure_fn() -> unit effect {} do
        return ()
    endfn

    fn impure_fn() -> unit effect { IO } do
        return ()
    endfn

    fn test_pure() -> unit effect {} do
        perform apply(f: pure_fn)
    endfn

    fn test_impure() -> unit effect { IO } do
        perform apply(f: impure_fn)
    endfn

    fn main() -> unit do
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

    fn apply<E>(f: () -> unit effect E) -> unit effect E do
        perform f()
    endfn

    fn impure_fn() -> unit effect { IO } do
        return ()
    endfn

    fn test_fail() -> unit effect {} do // Declared Pure
        perform apply(f: impure_fn)     // Call is Impure (IO)
    endfn

    fn main() -> unit do
        return ()
    endfn
    "#;
    assert!(check(src).is_err(), "Should fail because apply instantiates E=IO, so call becomes IO");
}
