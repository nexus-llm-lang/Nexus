/// Unification edge-case tests.
///
/// These target the tricky corners of type inference where implementations
/// commonly have bugs: occurs-check violations, row type unification,
/// polymorphism boundaries, and recursive type structures.
use crate::harness::{should_fail_typecheck, should_typecheck};

// ---------------------------------------------------------------------------
// Occurs check — prevent infinite types
// ---------------------------------------------------------------------------

#[test]
fn occurs_check_self_application_rejected() {
    // fn (x) -> x(x) creates the constraint T = T -> U, which is infinite.
    // The typechecker should reject this.
    should_fail_typecheck(
        r#"
    let self_apply = fn <T>(x: T) -> T do
        return x(x: x)
    end
    "#,
    );
}

#[test]
fn occurs_check_indirect_cycle_rejected() {
    // f(x) = x(f) creates T = (T -> U) -> V, an infinite type
    should_fail_typecheck(
        r#"
    let f = fn <T, U>(x: (y: T) -> U) -> U do
        return x(y: f)
    end
    "#,
    );
}

// ---------------------------------------------------------------------------
// Polymorphism boundaries
// ---------------------------------------------------------------------------

#[test]
fn let_polymorphism_basic() {
    // id should be usable at multiple types
    should_typecheck(
        r#"
    let id = fn <T>(x: T) -> T do return x end

    let main = fn () -> unit do
        let a: i64 = id(x: 42)
        let b: bool = id(x: true)
        let c: string = id(x: "hi")
        return ()
    end
    "#,
    );
}

#[test]
fn monomorphic_lambda_cannot_be_used_polymorphically() {
    // A lambda without explicit type params is monomorphic
    should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let f = fn (x: i64) -> i64 do return x end
        let a = f(x: 42)
        let b = f(x: true)
        return ()
    end
    "#,
    );
}

#[test]
fn generic_instantiation_preserves_constraints() {
    // first<A,B> returns A — calling with (i64, bool) should return i64, not bool
    should_fail_typecheck(
        r#"
    let first = fn <A, B>(a: A, b: B) -> A do return a end

    let main = fn () -> unit do
        let r: bool = first(a: 42, b: true)
        return ()
    end
    "#,
    );
}

#[test]
fn generic_applied_to_different_concrete_types() {
    should_typecheck(
        r#"
    type Box<T> = Box(val: T)

    let main = fn () -> unit do
        let bi: Box<i64> = Box(val: 42)
        let bs: Box<string> = Box(val: "hi")
        let bb: Box<bool> = Box(val: true)
        match bi do | Box(val: _) -> () end
        match bs do | Box(val: _) -> () end
        match bb do | Box(val: _) -> () end
        return ()
    end
    "#,
    );
}

#[test]
fn nested_generic_instantiation() {
    should_typecheck(
        r#"
    type Box<T> = Box(val: T)

    let main = fn () -> unit do
        let nested: Box<Box<i64>> = Box(val: Box(val: 42))
        match nested do
            | Box(val: Box(val: n)) -> return ()
        end
    end
    "#,
    );
}

// ---------------------------------------------------------------------------
// Row type / record unification
// ---------------------------------------------------------------------------

#[test]
fn record_field_order_irrelevant() {
    should_typecheck(
        r#"
    let take = fn (r: { x: i64, y: bool }) -> i64 do return r.x end

    let main = fn () -> unit do
        let _ = take(r: { y: true, x: 42 })
        return ()
    end
    "#,
    );
}

#[test]
fn record_missing_field_rejected() {
    should_fail_typecheck(
        r#"
    let take = fn (r: { x: i64, y: bool }) -> i64 do return r.x end

    let main = fn () -> unit do
        let _ = take(r: { x: 42 })
        return ()
    end
    "#,
    );
}

#[test]
fn record_extra_field_rejected() {
    should_fail_typecheck(
        r#"
    let take = fn (r: { x: i64 }) -> i64 do return r.x end

    let main = fn () -> unit do
        let _ = take(r: { x: 42, y: true })
        return ()
    end
    "#,
    );
}

#[test]
fn record_field_type_mismatch_rejected() {
    should_fail_typecheck(
        r#"
    let take = fn (r: { x: i64, y: bool }) -> i64 do return r.x end

    let main = fn () -> unit do
        let _ = take(r: { x: 42, y: "not a bool" })
        return ()
    end
    "#,
    );
}

#[test]
fn nested_record_unification() {
    should_typecheck(
        r#"
    type Inner = { a: i64 }
    type Outer = { inner: Inner, b: bool }

    let extract = fn (o: Outer) -> i64 do return o.inner.a end

    let main = fn () -> unit do
        let o: Outer = { inner: { a: 42 }, b: true }
        let _ = extract(o: o)
        return ()
    end
    "#,
    );
}

// ---------------------------------------------------------------------------
// Effect row unification
// ---------------------------------------------------------------------------

#[test]
fn effect_row_empty_is_pure() {
    should_typecheck(
        r#"
    let pure_fn = fn () -> unit throws {} do return () end

    let main = fn () -> unit do
        pure_fn()
        return ()
    end
    "#,
    );
}

#[test]
fn effect_row_subset_accepted() {
    // A function with throws {} can be called where throws { IO } is expected
    should_typecheck(
        r#"
    type IO = {}

    let pure_fn = fn () -> unit throws {} do return () end

    let impure_ctx = fn () -> unit throws { IO } do
        pure_fn()
        return ()
    end

    let main = fn () -> unit do return () end
    "#,
    );
}

#[test]
fn effect_row_superset_rejected() {
    // Cannot call impure function from pure context
    should_fail_typecheck(
        r#"
    type IO = {}

    let impure = fn () -> unit throws { IO } do return () end

    let pure_fn = fn () -> unit throws {} do
        impure()
        return ()
    end

    let main = fn () -> unit do return () end
    "#,
    );
}

// ---------------------------------------------------------------------------
// Type alias / named type interactions
// ---------------------------------------------------------------------------

#[test]
fn type_alias_is_nominally_distinct() {
    // In nexus, `type Age = i64` creates a *distinct* named type.
    // Arithmetic operators expect i64, not Age — this correctly fails.
    should_fail_typecheck(
        r#"
    type Age = i64

    let birthday = fn (age: Age) -> Age do
        return age + 1
    end
    "#,
    );
}

#[test]
fn type_alias_literal_is_also_rejected() {
    // Even literal assignment doesn't work — nexus type aliases are fully nominal
    should_fail_typecheck(
        r#"
    type Age = i64

    let main = fn () -> unit do
        let a: Age = 25
        return ()
    end
    "#,
    );
}

#[test]
fn recursive_enum_type() {
    should_typecheck(
        r#"
    type Expr = Lit(val: i64) | Add(left: Expr, right: Expr)

    let eval = fn (e: Expr) -> i64 do
        match e do
            | Lit(val: n) -> return n
            | Add(left: l, right: r) ->
                let a = eval(e: l)
                let b = eval(e: r)
                return a + b
        end
    end

    let main = fn () -> unit do
        let e = Add(left: Lit(val: 1), right: Add(left: Lit(val: 2), right: Lit(val: 3)))
        let result = eval(e: e)
        return ()
    end
    "#,
    );
}

// ---------------------------------------------------------------------------
// Multiple type variable interactions
// ---------------------------------------------------------------------------

#[test]
fn two_type_vars_independent() {
    should_typecheck(
        r#"
    let swap = fn <A, B>(a: A, b: B) -> { fst: B, snd: A } do
        return { fst: b, snd: a }
    end

    let main = fn () -> unit do
        let r = swap(a: 42, b: "hello")
        let s: string = r.fst
        let n: i64 = r.snd
        return ()
    end
    "#,
    );
}

#[test]
fn type_var_unifies_consistently() {
    // Both uses of T must unify to the same type
    should_fail_typecheck(
        r#"
    let pair_same = fn <T>(a: T, b: T) -> T do return a end

    let main = fn () -> unit do
        let _ = pair_same(a: 42, b: "string")
        return ()
    end
    "#,
    );
}

#[test]
fn compose_generic_functions() {
    should_typecheck(
        r#"
    let id = fn <T>(x: T) -> T do return x end
    let const_fn = fn <A, B>(a: A, b: B) -> A do return a end

    let main = fn () -> unit do
        let r = const_fn(a: id(x: 42), b: id(x: true))
        return ()
    end
    "#,
    );
}
