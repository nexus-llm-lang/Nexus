/// Property-based tests derived from the formal typing rules in
/// docs/spec/type-system-formal.md.
///
/// Each proptest block corresponds to a section of the formal spec and tests
/// the key properties (conjectures P1-P8) empirically.
use crate::harness::{exec, exec_with_stdlib, should_fail_typecheck, should_typecheck};
use proptest::prelude::*;

// ============================================================================
// Section 1: Core Expression Rules
// ============================================================================

// --- 1.1 Literals: T-Int, T-Bool, T-String, T-Unit, T-Float ---
// Property: literals always typecheck and their types are stable under binding.

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, failure_persistence: None, ..Default::default() })]

    /// [T-Int] Integer literals typecheck and execute.
    /// Tests P3 (soundness): typecheck => compile+run.
    #[test]
    fn prop_literal_int_sound(n in any::<i64>()) {
        let src = format!("let main = fn () -> unit do\n    let x: i64 = {n}\n    return ()\nend\n");
        should_typecheck(&src);
        exec(&src);
    }

    /// [T-Float] Float literals typecheck and execute.
    #[test]
    fn prop_literal_float_sound(
        // Avoid NaN/Inf which have non-standard display
        n in -1e10f64..1e10f64
    ) {
        let s = format!("{:.6}", n);
        let src = format!("let main = fn () -> unit do\n    let x: f64 = {s}\n    return ()\nend\n");
        should_typecheck(&src);
        exec(&src);
    }

    /// [T-Bool] Bool literals always typecheck.
    #[test]
    fn prop_literal_bool_sound(b in any::<bool>()) {
        let src = format!("let main = fn () -> unit do\n    let x: bool = {b}\n    return ()\nend\n");
        should_typecheck(&src);
        exec(&src);
    }
}

// --- 1.2 Variables: T-Var ---
// Property: instantiation produces independent copies.

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..Default::default() })]

    /// [T-Var] + [Gen] + [Inst]: A polymorphic function can be instantiated
    /// at different types in the same scope. Tests P8 (generalization).
    #[test]
    fn prop_polymorphic_instantiation_independent(a in any::<i64>(), b in any::<bool>()) {
        let src = format!(r#"
let id = fn <T>(x: T) -> T do return x end
let main = fn () -> unit do
    let i: i64 = id(x: {a})
    let b: bool = id(x: {b})
    return ()
end
"#);
        should_typecheck(&src);
        exec(&src);
    }

    /// [T-Var] Instantiation at incompatible types is rejected.
    #[test]
    fn prop_polymorphic_instantiation_mismatch(n in any::<i64>()) {
        let src = format!(r#"
let id = fn <T>(x: T) -> T do return x end
let main = fn () -> unit do
    let b: bool = id(x: {n})
    return ()
end
"#);
        should_fail_typecheck(&src);
    }
}

// --- 1.3 Binary Operations: T-Arith, T-Compare, T-Concat, T-Logic ---
// Property: operator return types are correct and operand types must match.

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, failure_persistence: None, ..Default::default() })]

    /// [T-Arith] Arithmetic on i64 produces i64. Tests P3.
    #[test]
    fn prop_arith_i64_sound(a in -1000i64..1000, b in 1i64..1000) {
        let src = format!(r#"
let main = fn () -> unit do
    let sum: i64 = {a} + {b}
    let diff: i64 = {a} - {b}
    let prod: i64 = {a} * {b}
    let quot: i64 = {a} / {b}
    return ()
end
"#);
        should_typecheck(&src);
        exec(&src);
    }

    /// [T-Arith] Mixed-type arithmetic is rejected (i64 + bool).
    #[test]
    fn prop_arith_type_mismatch_rejected(n in any::<i64>(), b in any::<bool>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let x = {n} + {b}
    return ()
end
"#);
        should_fail_typecheck(&src);
    }

    /// [T-Compare] Comparison returns bool. Tests P3.
    #[test]
    fn prop_compare_returns_bool(a in -100i64..100, b in -100i64..100) {
        let src = format!(r#"
let main = fn () -> unit do
    let lt: bool = {a} < {b}
    let eq: bool = {a} == {b}
    let ne: bool = {a} != {b}
    return ()
end
"#);
        should_typecheck(&src);
        exec_with_stdlib(&src);
    }

    /// [T-Concat] ++ requires strings, returns string. Tests P3.
    #[test]
    fn prop_concat_string_sound(
        a in "[a-zA-Z0-9]{0,10}",
        b in "[a-zA-Z0-9]{0,10}"
    ) {
        let src = format!(r#"
let main = fn () -> unit do
    let s: string = "{a}" ++ "{b}"
    return ()
end
"#);
        should_typecheck(&src);
        exec_with_stdlib(&src);
    }

    /// [T-Concat] ++ on non-strings is rejected.
    #[test]
    fn prop_concat_non_string_rejected(n in any::<i64>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let s = {n} ++ "hello"
    return ()
end
"#);
        should_fail_typecheck(&src);
    }
}

// --- 1.4 Function Application: T-App ---
// Property: argument types must unify with parameter types.

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..Default::default() })]

    /// [T-App] Correct argument types are accepted and execute. Tests P3.
    #[test]
    fn prop_app_correct_types_sound(a in any::<i64>(), b in any::<i64>()) {
        let src = format!(r#"
let add = fn (a: i64, b: i64) -> i64 do return a + b end
let main = fn () -> unit do
    let r: i64 = add(a: {a}, b: {b})
    return ()
end
"#);
        should_typecheck(&src);
        exec(&src);
    }

    /// [T-App] Wrong argument label is rejected.
    #[test]
    fn prop_app_wrong_label_rejected(n in any::<i64>()) {
        let src = format!(r#"
let f = fn (x: i64) -> i64 do return x end
let main = fn () -> unit do
    let r = f(y: {n})
    return ()
end
"#);
        should_fail_typecheck(&src);
    }

    /// [T-App] Arity mismatch (too few) is rejected.
    #[test]
    fn prop_app_arity_too_few(n in any::<i64>()) {
        let src = format!(r#"
let add = fn (a: i64, b: i64) -> i64 do return a + b end
let main = fn () -> unit do
    let r = add(a: {n})
    return ()
end
"#);
        should_fail_typecheck(&src);
    }
}

// --- 1.5 Constructor: T-Ctor ---
// Property: constructors with correct labels and types are accepted.

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..Default::default() })]

    /// [T-Ctor] Correct constructor use typechecks and runs. Tests P3.
    #[test]
    fn prop_ctor_option_sound(n in any::<i64>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let x: Option<i64> = Some(val: {n})
    let y: Option<i64> = None
    match x do
        case Some(val: v) -> return ()
        case None -> return ()
    end
end
"#);
        should_typecheck(&src);
        exec(&src);
    }

    /// [T-Ctor] Wrong field label in constructor is rejected.
    #[test]
    fn prop_ctor_wrong_label_rejected(n in any::<i64>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let x = Some(value: {n})
    return ()
end
"#);
        should_fail_typecheck(&src);
    }
}

// --- 1.8 If-Then-Else: T-If ---
// Property: condition must be bool, branches must be consistent.

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..Default::default() })]

    /// [T-If] If with bool condition typechecks and runs. Tests P3.
    #[test]
    fn prop_if_bool_cond_sound(cond in any::<bool>(), a in any::<i64>(), b in any::<i64>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let r: i64 = if {cond} then {a} else {b} end
    return ()
end
"#);
        should_typecheck(&src);
        exec(&src);
    }

    /// [T-If] Non-bool condition is rejected.
    #[test]
    fn prop_if_non_bool_cond_rejected(n in any::<i64>()) {
        let src = format!(r#"
let main = fn () -> unit do
    if {n} then return () else return () end
end
"#);
        should_fail_typecheck(&src);
    }

    /// [T-If] Branch type mismatch in if-expression is rejected.
    #[test]
    fn prop_if_branch_type_mismatch_rejected(n in any::<i64>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let x: i64 = if true then {n} else "nope" end
    return ()
end
"#);
        should_fail_typecheck(&src);
    }
}

// ============================================================================
// Section 1.13: Lazy Thunks and Force
// ============================================================================

// --- T-Force-Lazy, T-Force-NonLazy, T-Var-Lazy-Consume, L-Lazy-Must-Consume ---

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..Default::default() })]

    /// [T-Force-Lazy] Force unwraps @T to T. Tests P3.
    #[test]
    fn prop_lazy_force_unwraps(n in any::<i64>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let @x = {n}
    let v: i64 = @x
    return ()
end
"#);
        should_typecheck(&src);
        exec(&src);
    }

    /// [T-Var-Lazy-Consume] @T is always linear — consumed on force. Tests P4.
    #[test]
    fn prop_lazy_consumed_on_force(n in any::<i64>()) {
        // Single force: ok
        let src = format!(r#"
let main = fn () -> unit do
    let @x = {n}
    let v = @x
    return ()
end
"#);
        should_typecheck(&src);
        exec(&src);
    }

    /// [T-Var-Lazy-Consume] Double force of @T is rejected (one-shot). Tests P4.
    #[test]
    fn prop_lazy_double_force_rejected(n in any::<i64>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let @x = {n}
    let a = @x
    let b = @x
    return ()
end
"#);
        should_fail_typecheck(&src);
    }

    /// [L-Lazy-Must-Consume] Unconsumed @T is a compile error (even for primitives). Tests P4.
    #[test]
    fn prop_lazy_unconsumed_rejected(n in any::<i64>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let @x = {n}
    return ()
end
"#);
        should_fail_typecheck(&src);
    }
}

// --- T-Let-Lazy: @T in let binding ---

#[test]
fn prop_lazy_type_annotation() {
    should_typecheck(
        r#"
let main = fn () -> unit do
    let @x = 42
    let v: i64 = @x
    return ()
end
"#,
    );
}

// --- L-Lazy-Closure: closure capturing @T becomes linear ---

#[test]
fn prop_lazy_capture_makes_closure_linear() {
    // Closure captures @x, becomes linear, single call ok
    should_typecheck(
        r#"
let test = fn () -> i64 do
    let @x = 42
    let f = fn () -> i64 do return @x end
    return f()
end
"#,
    );
}

#[test]
fn prop_lazy_capture_closure_double_call_rejected() {
    // Closure captures @x, calling twice is rejected
    should_fail_typecheck(
        r#"
let test = fn () -> i64 do
    let @x = 42
    let f = fn () -> i64 do return @x end
    let _a = f()
    let _b = f()
    return 0
end
"#,
    );
}

// --- T-Force-NonLazy: @x on a non-@-bound variable is an error ---
// Note: @x is parsed as a variable lookup with lazy sigil, so it
// only works when the variable was bound with `let @x = ...`.
// Force on a non-lazy expression works via Expr::Force wrapping.

#[test]
fn prop_force_non_lazy_variable_is_error() {
    should_fail_typecheck(
        r#"
let main = fn () -> unit do
    let x = 42
    let v: i64 = @x
    return ()
end
"#,
    );
}

// --- Lazy with computation (not just literals) ---

#[test]
fn prop_lazy_deferred_computation() {
    should_typecheck(
        r#"
let expensive = fn (n: i64) -> i64 do return n * n end

let main = fn () -> unit do
    let @result = expensive(n: 42)
    let v: i64 = @result
    return ()
end
"#,
    );
}

// --- Lazy branch consistency (same as linear) ---

#[test]
fn prop_lazy_branch_consistency_required() {
    // Must consume @x in both branches or neither
    should_fail_typecheck(
        r#"
let main = fn () -> unit do
    let @x = 42
    if true then
        let _ = @x
    else
        ()
    end
    return ()
end
"#,
    );
}

#[test]
fn prop_lazy_branch_consistent_both_force() {
    should_typecheck(
        r#"
let main = fn () -> unit do
    let @x = 42
    if true then
        let _ = @x
    else
        let _ = @x
    end
    return ()
end
"#,
    );
}

// ============================================================================
// Section 4: Linear Type Rules
// ============================================================================

// --- 4.1 Consumption: L-Consume, L-Double-Consume ---

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..Default::default() })]

    /// [L-Consume] Single consumption of linear value succeeds. Tests P4.
    #[test]
    fn prop_linear_single_consume_sound(n in any::<i64>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let %x = {{ id: {n} }}
    match %x do case _ -> () end
    return ()
end
"#);
        should_typecheck(&src);
        exec(&src);
    }

    /// [L-Double-Consume] Double consumption is rejected. Tests P4.
    #[test]
    fn prop_linear_double_consume_rejected(n in any::<i64>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let %x = {{ id: {n} }}
    match %x do case _ -> () end
    match %x do case _ -> () end
    return ()
end
"#);
        should_fail_typecheck(&src);
    }
}

// --- 4.2 Scope Exit: L-Leak ---

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..Default::default() })]

    /// [L-Leak] Unconsumed linear record is rejected. Tests P4.
    #[test]
    fn prop_linear_unconsumed_rejected(n in any::<i64>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let %x = {{ id: {n} }}
    return ()
end
"#);
        should_fail_typecheck(&src);
    }

    /// [L-Leak] Unconsumed linear primitive is auto-dropped (OK).
    #[test]
    fn prop_linear_primitive_auto_drop(n in any::<i64>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let %x = {n}
    return ()
end
"#);
        should_typecheck(&src);
    }
}

// --- 4.3 Linearity Weakening: L-Weaken ---

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..Default::default() })]

    /// [L-Weaken] Non-linear value can be passed to linear parameter.
    #[test]
    fn prop_linear_weakening_accepted(n in any::<i64>()) {
        let src = format!(r#"
let consume = fn (x: %i64) -> i64 do
    match x do case _ -> () end
    return 1
end
let main = fn () -> unit do
    let y = consume(x: {n})
    return ()
end
"#);
        should_typecheck(&src);
        exec(&src);
    }
}

// --- 4.4 Branch Consistency: L-Branch-Mismatch ---

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, failure_persistence: None, ..Default::default() })]

    /// [L-Branch-Mismatch] Consuming linear in only one branch is rejected. Tests P4.
    #[test]
    fn prop_linear_branch_mismatch_rejected(n in any::<i64>(), cond in any::<bool>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let %x = {{ id: {n} }}
    if {cond} then
        match %x do case _ -> () end
    else
        ()
    end
    return ()
end
"#);
        should_fail_typecheck(&src);
    }

    /// [L-Branch-Mismatch] Consuming in both branches is accepted. Tests P4.
    #[test]
    fn prop_linear_branch_consistent_accepted(n in any::<i64>(), cond in any::<bool>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let %x = {{ id: {n} }}
    if {cond} then
        match %x do case _ -> () end
    else
        match %x do case _ -> () end
    end
    return ()
end
"#);
        should_typecheck(&src);
        exec(&src);
    }
}

// --- 4.5 Lambda Capture: L-Linear-Closure ---

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, failure_persistence: None, ..Default::default() })]

    /// [L-Linear-Closure] Lambda capturing linear is linear (single-use). Tests P4.
    #[test]
    fn prop_linear_closure_single_use(n in any::<i64>()) {
        // Single call: ok
        let src = format!(r#"
let test = fn () -> i64 do
    let %x = {n}
    let f = fn () -> i64 do
        match %x do case _ -> () end
        return 1
    end
    return f()
end
"#);
        should_typecheck(&src);
    }

    /// [L-Linear-Closure] Calling linear closure twice is rejected.
    #[test]
    fn prop_linear_closure_double_call_rejected(n in any::<i64>()) {
        let src = format!(r#"
let test = fn () -> i64 do
    let %x = {n}
    let f = fn () -> i64 do
        match %x do case _ -> () end
        return 1
    end
    let _a = f()
    let _b = f()
    return 0
end
"#);
        should_fail_typecheck(&src);
    }
}

// ============================================================================
// Section 5: Effect System Rules
// ============================================================================

// --- 5.2 Effect Propagation: E-Call ---

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..Default::default() })]

    /// [E-Call] Impure function called from impure context is accepted.
    #[test]
    fn prop_effect_propagation_sound(calls in 1usize..5) {
        let mut body = String::new();
        for _ in 0..calls {
            body.push_str("    io(x: 0)\n");
        }
        let src = format!(r#"
type IO = {{}}
let io = fn (x: i64) -> unit throws {{ IO }} do return () end
let f = fn () -> unit throws {{ IO }} do
{body}    return ()
end
let main = fn () -> unit do return () end
"#);
        should_typecheck(&src);
    }

    /// [E-Call] Impure function called from pure context is rejected. Tests P5.
    #[test]
    fn prop_effect_leak_rejected(calls in 1usize..5) {
        let mut body = String::new();
        for _ in 0..calls {
            body.push_str("    io(x: 0)\n");
        }
        let src = format!(r#"
type IO = {{}}
let io = fn (x: i64) -> unit throws {{ IO }} do return () end
let pure_f = fn () -> unit throws {{}} do
{body}    return ()
end
let main = fn () -> unit do return () end
"#);
        should_fail_typecheck(&src);
    }
}

// --- 5.3 Effect Subsumption: E-Pure-Subsume ---

#[test]
fn prop_pure_callable_from_impure_context() {
    should_typecheck(
        r#"
type IO = {}
let pure_fn = fn () -> unit do return () end
let impure_fn = fn () -> unit throws { IO } do
    pure_fn()
    return ()
end
let main = fn () -> unit do return () end
"#,
    );
}

// --- 5.4 Effect Discharge: E-TryCatch-Discharge ---

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, failure_persistence: None, ..Default::default() })]

    /// [E-TryCatch-Discharge] try/catch discharges Exn from throws. Tests P5.
    #[test]
    fn prop_try_catch_discharges_exn(msg in "[a-zA-Z]{1,10}") {
        let src = format!(r#"
exception TestError(val: string)

let risky = fn () -> unit throws {{ Exn }} do
    raise TestError(val: "{msg}")
    return ()
end

let main = fn () -> unit do
    try
        risky()
    catch
        case TestError(val: m) -> ()
        case _ -> ()
    end
    return ()
end
"#);
        should_typecheck(&src);
        exec(&src);
    }

    /// [E-TryCatch-Discharge] raise without throws { Exn } is rejected.
    #[test]
    fn prop_raise_without_throws_rejected(msg in "[a-zA-Z]{1,10}") {
        let src = format!(r#"
exception TestError(val: string)

let f = fn () -> unit do
    raise TestError(val: "{msg}")
    return ()
end
"#);
        should_fail_typecheck(&src);
    }
}

// --- 5.6 Main Function Constraints: E-Main ---

#[test]
fn prop_main_must_return_unit() {
    should_fail_typecheck(
        r#"
let main = fn () -> i64 do return 0 end
"#,
    );
}

#[test]
fn prop_main_cannot_throw() {
    should_fail_typecheck(
        r#"
exception Boom(i64)
let main = fn () -> unit throws { Exn } do
    raise Boom(42)
    return ()
end
"#,
    );
}

// ============================================================================
// Section 6: Unification Rules
// ============================================================================

// --- 6.1 Core Unification: U-Refl, U-Var, U-Occurs ---

#[test]
fn prop_unify_reflexive() {
    // Same type on both sides of annotation always works.
    should_typecheck(
        r#"
let main = fn () -> unit do
    let x: i64 = 42
    let y: bool = true
    let z: string = "hi"
    return ()
end
"#,
    );
}

#[test]
fn prop_occurs_check_rejects_self_application() {
    // T = T -> U is an infinite type.
    should_fail_typecheck(
        r#"
let self_apply = fn <T>(x: T) -> T do
    return x(x: x)
end
"#,
    );
}

// --- 6.2 Numeric Literal Unification: U-IntLit, U-FloatLit ---

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..Default::default() })]

    /// [U-IntLit] Integer literals unify with i32 and i64.
    #[test]
    fn prop_intlit_unifies_i64(n in any::<i64>()) {
        let src = format!(r#"
let f = fn (x: i64) -> i64 do return x end
let main = fn () -> unit do
    let r = f(x: {n})
    return ()
end
"#);
        should_typecheck(&src);
    }

    /// [U-IntLit] Integer literal does NOT unify with string.
    #[test]
    fn prop_intlit_no_unify_string(n in any::<i64>()) {
        let src = format!(r#"
let f = fn (x: string) -> string do return x end
let main = fn () -> unit do
    let r = f(x: {n})
    return ()
end
"#);
        should_fail_typecheck(&src);
    }
}

// --- 6.3 Structural Unification: U-Arrow, U-Record ---

#[test]
fn prop_arrow_label_mismatch_rejected() {
    // Parameter labels must match in function types.
    should_fail_typecheck(
        r#"
let apply = fn (f: (x: i64) -> i64, val: i64) -> i64 do
    return f(x: val)
end

let g = fn (y: i64) -> i64 do return y end

let main = fn () -> unit do
    let r = apply(f: g, val: 1)
    return ()
end
"#,
    );
}

#[test]
fn prop_record_field_order_irrelevant() {
    // Record unification is order-independent.
    should_typecheck(
        r#"
let take = fn (r: { a: i64, b: bool }) -> i64 do return r.a end
let main = fn () -> unit do
    let _ = take(r: { b: true, a: 42 })
    return ()
end
"#,
    );
}

#[test]
fn prop_record_missing_field_rejected() {
    should_fail_typecheck(
        r#"
let take = fn (r: { a: i64, b: bool }) -> i64 do return r.a end
let main = fn () -> unit do
    let _ = take(r: { a: 42 })
    return ()
end
"#,
    );
}

// --- 6.4 Row Unification: U-Row-Open ---

#[test]
fn prop_row_polymorphic_effect() {
    // Effect variable allows calling both pure and impure functions.
    should_typecheck(
        r#"
type IO = {}

let apply = fn <E>(f: () -> unit throws E) -> unit throws E do
    f()
end

let pure_fn = fn () -> unit throws {} do return () end
let impure_fn = fn () -> unit throws { IO } do return () end

let test_pure = fn () -> unit throws {} do
    apply(f: pure_fn)
end

let test_impure = fn () -> unit throws { IO } do
    apply(f: impure_fn)
end

let main = fn () -> unit do return () end
"#,
    );
}

// --- 6.5 Borrow Weakening: U-Borrow-Read ---

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, failure_persistence: None, ..Default::default() })]

    /// [U-Borrow-Read] A borrow can be read as its underlying type.
    #[test]
    fn prop_borrow_read_as_underlying(n in any::<i64>()) {
        let src = format!(r#"
let peek = fn (x: &i64) -> i64 do return x end
let main = fn () -> unit do
    let %r = {n}
    let ref = &%r
    let v: i64 = peek(x: ref)
    match %r do case _ -> () end
    return ()
end
"#);
        should_typecheck(&src);
        exec(&src);
    }
}

// ============================================================================
// Section 7: Generalization and Instantiation
// ============================================================================

// --- [Gen] + [Inst]: let-polymorphism ---

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..Default::default() })]

    /// [Gen] Free variables not in environment are generalized.
    /// The same function can be used at i64, bool, string. Tests P8.
    #[test]
    fn prop_generalization_multi_instantiation(
        n in any::<i64>(), b in any::<bool>()
    ) {
        let src = format!(r#"
let const_fn = fn <A, B>(a: A, b: B) -> A do return a end
let main = fn () -> unit do
    let r1: i64 = const_fn(a: {n}, b: true)
    let r2: bool = const_fn(a: {b}, b: 42)
    let r3: string = const_fn(a: "hi", b: {n})
    return ()
end
"#);
        should_typecheck(&src);
        exec(&src);
    }

    /// [Inst] Monomorphic lambdas cannot be used polymorphically.
    #[test]
    fn prop_monomorphic_lambda_not_polymorphic(n in any::<i64>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let f = fn (x: i64) -> i64 do return x end
    let a = f(x: {n})
    let b = f(x: true)
    return ()
end
"#);
        should_fail_typecheck(&src);
    }
}

// ============================================================================
// Section 8: Exhaustiveness (Conjecture P6)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, failure_persistence: None, ..Default::default() })]

    /// [P6] Exhaustive match on bool covers all cases. Tests P6 + P3.
    #[test]
    fn prop_exhaustive_bool_sound(b in any::<bool>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let x = {b}
    match x do
        case true -> return ()
        case false -> return ()
    end
end
"#);
        should_typecheck(&src);
        exec(&src);
    }

    /// [P6] Missing bool case is non-exhaustive.
    #[test]
    fn prop_non_exhaustive_bool_rejected(b in any::<bool>()) {
        let src = format!(r#"
let main = fn () -> unit do
    let x = {b}
    match x do
        case true -> return ()
    end
end
"#);
        should_fail_typecheck(&src);
    }
}

// Exhaustiveness for enum with N variants: any proper subset is non-exhaustive.

#[test]
fn prop_exhaustive_option_requires_both_cases() {
    // Missing None case
    should_fail_typecheck(
        r#"
let main = fn () -> unit do
    let x: Option<i64> = Some(val: 1)
    match x do
        case Some(val: v) -> return ()
    end
end
"#,
    );

    // Both cases: ok
    should_typecheck(
        r#"
let main = fn () -> unit do
    let x: Option<i64> = Some(val: 1)
    match x do
        case Some(val: v) -> return ()
        case None -> return ()
    end
end
"#,
    );
}

#[test]
fn prop_exhaustive_nested_result_option() {
    // Nested ADTs require covering all combinations.
    should_typecheck(
        r#"
let main = fn () -> unit do
    let x: Result<Option<i64>, string> = Ok(val: Some(val: 1))
    match x do
        case Ok(val: Some(val: v)) -> return ()
        case Ok(val: None) -> return ()
        case Err(err: e) -> return ()
    end
end
"#,
    );

    // Missing Ok(val: None) is non-exhaustive
    should_fail_typecheck(
        r#"
let main = fn () -> unit do
    let x: Result<Option<i64>, string> = Ok(val: Some(val: 1))
    match x do
        case Ok(val: Some(val: v)) -> return ()
        case Err(err: e) -> return ()
    end
end
"#,
    );
}

// ============================================================================
// Section 9: Composite Properties (P3 — Soundness)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..Default::default() })]

    /// [P3] Programs combining generics + pattern matching + recursion
    /// typecheck and run without traps.
    #[test]
    fn prop_soundness_generic_list_sum(
        xs in proptest::collection::vec(-100i64..=100, 0usize..5)
    ) {
        let elems = xs.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
        let src = format!(r#"
let sum = fn (xs: [i64]) -> i64 do
    match xs do
        case Nil -> return 0
        case Cons(v: h, rest: t) ->
            let rest_sum = sum(xs: t)
            return h + rest_sum
    end
end

let main = fn () -> unit do
    let xs = [{elems}]
    let s = sum(xs: xs)
    return ()
end
"#);
        should_typecheck(&src);
        exec(&src);
    }

    /// [P3] Programs combining records + generics typecheck and run.
    #[test]
    fn prop_soundness_generic_record(a in any::<i64>(), b in any::<bool>()) {
        let src = format!(r#"
type Pair<A, B> = Pair(fst: A, snd: B)

let fst = fn <A, B>(p: Pair<A, B>) -> A do
    match p do
        case Pair(fst: a, snd: _) -> return a
    end
end

let main = fn () -> unit do
    let p = Pair(fst: {a}, snd: {b})
    let r: i64 = fst(p: p)
    return ()
end
"#);
        should_typecheck(&src);
        exec(&src);
    }

    /// [P3] While loops with ref cells typecheck and run.
    #[test]
    fn prop_soundness_while_ref(limit in 0i64..20) {
        let src = format!(r#"
let main = fn () -> unit do
    let ~i = 0
    while ~i < {limit} do
        ~i <- ~i + 1
    end
    return ()
end
"#);
        should_typecheck(&src);
        exec_with_stdlib(&src);
    }
}
