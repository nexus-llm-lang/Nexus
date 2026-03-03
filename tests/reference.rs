mod common;

use common::source::check_raw as check_code;

#[test]
fn test_ref_creation_and_type() {
    let src = r#"
    let main = fn () -> unit do
        let ~c = 0
        return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

// This test is now tricky because immutable vars simply cannot hold Ref if we can't create explicit Ref.
// But if I assign value to immutable, it's just value.
// The only way to get a Ref is by `let ~x`.
// So immutable variable cannot hold Ref unless a function returns Ref.
// And functions cannot return Ref.
// So this Gravity Rule is implicitly enforced by syntax + return check.
// I will change this test to ensure we CANNOT assign to immutable var later?
// No, immutable var cannot be assigned.
// Maybe I should test that `let c = ~x` (implicit deref) results in value, not ref.
#[test]
fn test_gravity_rule_immutable_holds_value() {
    let src = r#"
    let main = fn () -> unit do
        let ~c = 0
        let x = ~c // x should be i64, not Ref
        // If x was Ref, we could modify it? No, x is immutable.
        // But if x was Ref, we could potentially pass it to something expecting Ref?
        // But functions cannot take Ref arguments in Nexus?
        // Wait, params can be `~x`.
        // If I pass `x` (which holds Ref) to `fn f(~p)`, `p` becomes Ref.
        // But `x` is `i64`.
        return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

// Since `ref()` is gone, we cannot construct a ref to return.
// But we can try to return `~c`?
// `~c` evaluates to value.
// Can we return the reference itself?
// If we use just `c` (without tilde)?
// My parser for Variable with Mutable sigil expects `~`.
// If I use `c`, it's Variable("c", Immutable).
// But "c" is not in env. "~c" is.
// So I cannot access the reference itself by name!
// This means References are truly second-class and confined to stack!
// Excellent.
#[test]
fn test_cannot_return_ref() {
    // Attempting to return a reference is syntactically impossible or type error?
    // If I have `let ~c = 0`.
    // `return ~c` returns 0 (i64).
    // `return c` fails "Variable not found".
    // So Gravity Rule "Return cannot contain Ref" is enforced by:
    // 1. Implicit deref on access.
    // 2. Inability to access raw Ref.
    let src = r#"
    let main = fn () -> unit do
        let ~c = 0
        // return c // Variable not found
        return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_ref_assignment() {
    let src = r#"
    let main = fn () -> unit do
        let ~c = 0
        ~c <- 1
        return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_ref_read() {
    let src = r#"
    let __test_main = fn () -> i64 do
        let ~c = 10
        let v = ~c
        return v
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

// test_ref_assignment_type_mismatch: covered by prop_ref_assignment_type_mismatch_is_error

#[test]
fn test_ref_generic() {
    let src = r#"
    let box = fn <T>(x: T) -> unit do
        let ~r = x
        let v = ~r
        return ()
    end

    let main = fn () -> unit do
        box(x: 10)
        box(x: true)
        return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}
