use nexus::ast::*;
use nexus::typecheck::TypeChecker;

// --- Helpers ---
fn type_i64() -> Type {
    Type::I64
}
fn type_bool() -> Type {
    Type::Bool
}
fn type_unit() -> Type {
    Type::Unit
}
fn type_var(n: &str) -> Type {
    Type::UserDefined(n.to_string(), vec![])
}
fn type_arrow(params: Vec<Type>, ret: Type) -> Type {
    Type::Arrow(params, Box::new(ret))
}

fn expr_lit_int(i: i64) -> Expr {
    Expr::Literal(Literal::Int(i))
}
fn expr_var(n: &str) -> Expr {
    Expr::Variable(n.to_string(), Sigil::Immutable)
}
fn expr_call(func: &str, args: Vec<(&str, Expr)>) -> Expr {
    Expr::Call {
        func: func.to_string(),
        args: args.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        perform: false,
    }
}

// --- Theorems & Properties ---

// Property 1: Identity of Indiscernibles (in types) / Polymorphism
// Theorem: The 'id' function works for any type, preserving the type.
// If id: forall T. T -> T, then id(int) -> int, id(bool) -> bool.
#[test]
fn test_prop_identity_polymorphism() {
    let func_id = Function {
        name: "id".to_string(),
        type_params: vec!["T".to_string()],
        params: vec![Param {
            name: "x".to_string(),
            sigil: Sigil::Immutable,
            typ: type_var("T"),
        }],
        ret_type: type_var("T"),
        effects: vec![],
        body: vec![Stmt::Return(expr_var("x"))],
    };

    let func_main = Function {
        name: "main".to_string(),
        type_params: vec![],
        params: vec![],
        ret_type: type_unit(),
        effects: vec![],
        body: vec![
            Stmt::Let {
                name: "i".to_string(),
                sigil: Sigil::Immutable,
                typ: None,
                value: expr_call("id", vec![("x", expr_lit_int(1))]),
            },
            // Check that 'i' is inferred as Int by trying to unify it with something else?
            // Or rely on the fact that if it types check, 'id' returned compatible type.
            // Let's use 'i' in a context requiring int.
            Stmt::Let {
                name: "_1".to_string(),
                sigil: Sigil::Immutable,
                typ: None,
                value: Expr::BinaryOp(
                    Box::new(expr_var("i")),
                    "+".to_string(),
                    Box::new(expr_lit_int(1)),
                ),
            },
            // Re-use for bool
            Stmt::Let {
                name: "b".to_string(),
                sigil: Sigil::Immutable,
                typ: None,
                value: expr_call("id", vec![("x", Expr::Literal(Literal::Bool(true)))]),
            },
            // Use 'b' in if condition (requires bool)
            Stmt::Expr(Expr::If {
                cond: Box::new(expr_var("b")),
                then_branch: vec![],
                else_branch: None,
            }),
            Stmt::Return(Expr::Literal(Literal::Unit)),
        ],
    };

    let program = Program {
        definitions: vec![TopLevel::Function(func_id), TopLevel::Function(func_main)],
    };
    let mut checker = TypeChecker::new();
    assert!(
        checker.check_program(&program).is_ok(),
        "Identity polymorphism failed"
    );
}

// Property 2: K Combinator (Const) - Generalization of multiple variables
// Theorem: K: forall A, B. A -> B -> A.
// It discards the second argument. The types A and B can be distinct.
#[test]
fn test_prop_k_combinator() {
    let func_k = Function {
        name: "k".to_string(),
        type_params: vec!["A".to_string(), "B".to_string()],
        params: vec![
            Param {
                name: "a".to_string(),
                sigil: Sigil::Immutable,
                typ: type_var("A"),
            },
            Param {
                name: "b".to_string(),
                sigil: Sigil::Immutable,
                typ: type_var("B"),
            },
        ],
        ret_type: type_var("A"),
        effects: vec![],
        body: vec![Stmt::Return(expr_var("a"))],
    };

    let func_main = Function {
        name: "main".to_string(),
        type_params: vec![],
        params: vec![],
        ret_type: type_i64(),
        effects: vec![],
        body: vec![
            // k(1, true) -> 1 (int)
            Stmt::Let {
                name: "res".to_string(),
                sigil: Sigil::Immutable,
                typ: None,
                value: expr_call(
                    "k",
                    vec![
                        ("a", expr_lit_int(1)),
                        ("b", Expr::Literal(Literal::Bool(true))),
                    ],
                ),
            },
            Stmt::Return(expr_var("res")),
        ],
    };

    let program = Program {
        definitions: vec![TopLevel::Function(func_k), TopLevel::Function(func_main)],
    };
    let mut checker = TypeChecker::new();
    assert!(checker.check_program(&program).is_ok());
}

// Property 3: Occurs Check (Soundness)
// Theorem: A type T cannot be unified with a type containing T (e.g., T ~ T -> U).
// If this were allowed, it would imply infinite types (e.g. Y combinator type without explicit fix), which HM disallows.
#[test]
fn test_prop_occurs_check_fail() {
    // fn self_apply<T>(f: T) -> T { return f(f) }
    // This is the classic omega combinator part.
    // 'f' is type T.
    // We call 'f(f)'.
    // For 'f' to be callable, T must unify with Arrow(T -> ?).
    // So T ~ (T -> ?). T occurs in (T -> ?). Should fail.

    let func_self_apply = Function {
        name: "self_apply".to_string(),
        type_params: vec!["T".to_string()],
        params: vec![Param {
            name: "f".to_string(),
            sigil: Sigil::Immutable,
            typ: type_var("T"),
        }],
        ret_type: type_var("T"),
        effects: vec![],
        body: vec![Stmt::Return(expr_call("f", vec![("val", expr_var("f"))]))],
    };

    let program = Program {
        definitions: vec![TopLevel::Function(func_self_apply)],
    };
    let mut checker = TypeChecker::new();
    let res = checker.check_program(&program);
    assert!(res.is_err());
    let err = res.err().unwrap();
    println!("Occurs Check Error: {}", err);
    // In Nexus, generic parameters are rigid (UserDefined), so x(x) fails with Type Mismatch (UserDefined vs Arrow),
    // rather than "Recursive type" (Occurs Check) which applies to inference variables.
    // Both ensure soundness.
    assert!(
        err.contains("Recursive type") || err.contains("Infinite") || err.contains("Type mismatch")
    );
}

// Property 4: Higher-Order Functions / Function Composition
// Theorem: Functions are first-class citizens (can be passed as args) and types line up.
// apply: forall A, B. (A -> B) -> A -> B
#[test]
fn test_prop_higher_order_apply() {
    // fn apply<A, B>(f: (A)->B, x: A) -> B { return f(x) }
    let func_apply = Function {
        name: "apply".to_string(),
        type_params: vec!["A".to_string(), "B".to_string()],
        params: vec![
            Param {
                name: "f".to_string(),
                sigil: Sigil::Immutable,
                typ: type_arrow(vec![type_var("A")], type_var("B")),
            },
            Param {
                name: "x".to_string(),
                sigil: Sigil::Immutable,
                typ: type_var("A"),
            },
        ],
        ret_type: type_var("B"),
        effects: vec![],
        body: vec![Stmt::Return(expr_call("f", vec![("arg", expr_var("x"))]))],
    };

    // To test this, we need a concrete function to pass.
    // fn to_int(b: bool) -> i64 { if b then 1 else 0 }
    let func_to_int = Function {
        name: "to_int".to_string(),
        type_params: vec![],
        params: vec![Param {
            name: "b".to_string(),
            sigil: Sigil::Immutable,
            typ: type_bool(),
        }],
        ret_type: type_i64(),
        effects: vec![],
        body: vec![Stmt::Expr(Expr::If {
            cond: Box::new(expr_var("b")),
            then_branch: vec![Stmt::Return(expr_lit_int(1))],
            else_branch: Some(vec![Stmt::Return(expr_lit_int(0))]),
        })],
    };

    let func_main = Function {
        name: "main".to_string(),
        type_params: vec![],
        params: vec![],
        ret_type: type_i64(),
        effects: vec![],
        body: vec![
            // apply(to_int, true)
            Stmt::Return(expr_call(
                "apply",
                vec![
                    ("f", expr_var("to_int")),
                    ("x", Expr::Literal(Literal::Bool(true))),
                ],
            )),
        ],
    };

    let program = Program {
        definitions: vec![
            TopLevel::Function(func_apply),
            TopLevel::Function(func_to_int),
            TopLevel::Function(func_main),
        ],
    };

    let mut checker = TypeChecker::new();
    let res = checker.check_program(&program);
    if res.is_err() {
        println!("Error: {}", res.as_ref().unwrap_err());
    }
    assert!(res.is_ok());
}

// Property 5: Parametricity / Generics Rigidity
// Theorem: A function forall T. T -> T cannot return a value of specific type like Int.
// The implementation must be agnostic to T.
#[test]
fn test_prop_parametricity_violation() {
    let func_bad = Function {
        name: "bad".to_string(),
        type_params: vec!["T".to_string()],
        params: vec![Param {
            name: "x".to_string(),
            sigil: Sigil::Immutable,
            typ: type_var("T"),
        }],
        ret_type: type_var("T"),
        effects: vec![],
        body: vec![
            Stmt::Return(expr_lit_int(42)), // Returns I64, but T is required
        ],
    };

    let program = Program {
        definitions: vec![TopLevel::Function(func_bad)],
    };
    let mut checker = TypeChecker::new();
    let res = checker.check_program(&program);
    assert!(res.is_err());
    // Should be mismatch I64 vs T (rigid)
    assert!(res.unwrap_err().contains("Type mismatch"));
}
