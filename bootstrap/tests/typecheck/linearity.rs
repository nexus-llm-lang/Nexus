use crate::harness::{should_fail_typecheck, should_typecheck};

/// Hole-1 (catch-arm linear starting point):
///
/// A `try` body may transfer control to a `catch` arm via `raise` at any
/// point inside the try body. With strategy A from issue nexus-7eex.1,
/// each catch arm inherits `env.linear_vars` — the snapshot taken *before*
/// the try body — so any linear that was already in scope at the start of
/// the try and was consumed only on the success path is still considered
/// live in the catch arm. If a catch arm leaves it unconsumed, the
/// per-arm linear-set check fires.
///
/// Pre-strategy-A the catch arm inherited `et.linear_vars` (post-success
/// state where `t` was already consumed), and the unconsumed-on-throw
/// case was silently accepted. This negative test pinned that gap.
#[test]
fn test_try_catch_arm_starts_from_pre_try_linear_set() {
    let err = should_fail_typecheck(
        r#"
    exception Oops(msg: string)
    type Token = { id: i64 }

    let may_throw = fn () -> unit throws { Exn } do
        raise Oops(msg: "boom")
        return ()
    end

    let main = fn () -> unit do
        let %t: Token = { id: 1 }
        try
            may_throw()
            match %t do | { id: _ } -> () end
        catch _ -> ()
        end
        return ()
    end
    "#,
    );
    assert!(
        err.to_lowercase().contains("linear"),
        "expected a linearity-related error, got: {}",
        err
    );
}

/// Companion positive test: when a linear bound *before* `try` is consumed
/// by both the try body and the catch arm, the linear sets agree across
/// paths and the program typechecks. Guards against an over-zealous patch
/// that rejects all try/catch with linears in scope.
#[test]
fn test_try_catch_arm_consumes_pre_try_linear_pass() {
    should_typecheck(
        r#"
    exception Oops(msg: string)
    type Token = { id: i64 }

    let may_throw = fn () -> unit throws { Exn } do
        raise Oops(msg: "boom")
        return ()
    end

    let main = fn () -> unit do
        let %t: Token = { id: 1 }
        try
            may_throw()
            match %t do | { id: _ } -> () end
        catch _ ->
            match %t do | { id: _ } -> () end
        end
        return ()
    end
    "#,
    );
}
