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

/// Hole-2 (throwable call across linear, issue nexus-7eex.2):
///
/// The minimal example from the issue: a linear is bound, then a throwable
/// call fires while the linear is still live. If the call raises, the
/// linear is dropped without ever being consumed. With Strategy A from the
/// issue, the typechecker rejects the program at the throwable-call site
/// with `linear value cannot live across throwable call`. Pre-fix this
/// program PASSed because `Expr::Raise` left `linear_vars` untouched and
/// `Expr::Call` did not look at the callee's throw row.
#[test]
fn test_linear_across_throwable_call_outside_try_rejects() {
    let err = should_fail_typecheck(
        r#"
    exception Oops(msg: string)
    type FileToken = { id: i64 }

    let open_file = fn () -> %FileToken do
        let h: FileToken = { id: 1 }
        let %lh = h
        return %lh
    end

    let may_throw = fn () -> unit throws { Exn } do
        raise Oops(msg: "boom")
        return ()
    end

    let close = fn (h: %FileToken) -> unit do
        match h do | { id: _ } -> () end
        return ()
    end

    let main = fn () -> unit throws { Exn } do
        let %x = open_file()
        may_throw()
        close(h: %x)
        return ()
    end
    "#,
    );
    let lower = err.to_lowercase();
    assert!(
        lower.contains("linear value cannot live across throwable call"),
        "expected linear-across-throwable-call error, got: {}",
        err
    );
    assert!(
        err.contains("E0540"),
        "expected error code E0540 in message, got: {}",
        err
    );
}

/// Companion positive test: consume the linear *before* the throwable
/// call. No linear is live at the call site, so the program typechecks.
#[test]
fn test_linear_consumed_before_throwable_call_passes() {
    should_typecheck(
        r#"
    exception Oops(msg: string)
    type FileToken = { id: i64 }

    let open_file = fn () -> %FileToken do
        let h: FileToken = { id: 1 }
        let %lh = h
        return %lh
    end

    let may_throw = fn () -> unit throws { Exn } do
        raise Oops(msg: "boom")
        return ()
    end

    let close = fn (h: %FileToken) -> unit do
        match h do | { id: _ } -> () end
        return ()
    end

    let main = fn () -> unit do
        let %x = open_file()
        close(h: %x)
        try
            may_throw()
        catch _ -> ()
        end
        return ()
    end
    "#,
    );
}

/// Negative-control: a *non*-throwable (`throws { }`) call may safely be
/// invoked while a linear is live — the linear cannot be lost via raise
/// because there is no raise. Guards against rejecting all calls.
#[test]
fn test_linear_across_pure_call_passes() {
    should_typecheck(
        r#"
    type FileToken = { id: i64 }

    let open_file = fn () -> %FileToken do
        let h: FileToken = { id: 1 }
        let %lh = h
        return %lh
    end

    let pure_call = fn () -> unit do
        return ()
    end

    let close = fn (h: %FileToken) -> unit do
        match h do | { id: _ } -> () end
        return ()
    end

    let main = fn () -> unit do
        let %x = open_file()
        pure_call()
        close(h: %x)
        return ()
    end
    "#,
    );
}

/// Inside a `try` body, a linear that already existed *before* the try is
/// the catch arm's responsibility (Strategy A from issue nexus-7eex.1). A
/// throwable call inside the try body, with that pre-try linear still
/// live, is therefore allowed: the catch arm — which inherits the pre-try
/// snapshot — must consume it on the raise path.
#[test]
fn test_linear_across_throwable_call_inside_try_with_catch_consume_passes() {
    should_typecheck(
        r#"
    exception Oops(msg: string)
    type FileToken = { id: i64 }

    let may_throw = fn () -> unit throws { Exn } do
        raise Oops(msg: "boom")
        return ()
    end

    let close = fn (h: %FileToken) -> unit do
        match h do | { id: _ } -> () end
        return ()
    end

    let main = fn () -> unit do
        let h: FileToken = { id: 1 }
        let %x = h
        try
            may_throw()
            close(h: %x)
        catch _ ->
            close(h: %x)
        end
        return ()
    end
    "#,
    );
}

/// A linear *created inside* the try body, before a throwable call, is not
/// covered by the catch arm (the arm only sees the pre-try snapshot). This
/// is still a leak on raise and must be rejected.
#[test]
fn test_linear_created_inside_try_across_throwable_call_rejects() {
    let err = should_fail_typecheck(
        r#"
    exception Oops(msg: string)
    type FileToken = { id: i64 }

    let open_file = fn () -> %FileToken do
        let h: FileToken = { id: 1 }
        let %lh = h
        return %lh
    end

    let may_throw = fn () -> unit throws { Exn } do
        raise Oops(msg: "boom")
        return ()
    end

    let close = fn (h: %FileToken) -> unit do
        match h do | { id: _ } -> () end
        return ()
    end

    let main = fn () -> unit do
        try
            let %x = open_file()
            may_throw()
            close(h: %x)
        catch _ -> ()
        end
        return ()
    end
    "#,
    );
    assert!(
        err.to_lowercase().contains("linear value cannot live across throwable call"),
        "expected linear-across-throwable-call error, got: {}",
        err
    );
}
