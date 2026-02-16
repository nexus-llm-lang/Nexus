use nexus::ast::*;
use nexus::typecheck::TypeChecker;

// Helper to create basic types
fn type_i64() -> Type {
    Type::I64
}
fn type_unit() -> Type {
    Type::Unit
}

// Helper to create expressions
fn expr_lit_int(i: i64) -> Expr {
    Expr::Literal(Literal::Int(i))
}
fn expr_lit_bool(b: bool) -> Expr {
    Expr::Literal(Literal::Bool(b))
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

// Test: Simple Identity Function Type Checking
#[test]
fn test_ast_identity_function() {
    // fn id<T>(x: T) -> T { return x }
    let func_id = Function {
        name: "id".to_string(),
        type_params: vec!["T".to_string()],
        params: vec![Param {
            name: "x".to_string(),
            sigil: Sigil::Immutable,
            typ: Type::UserDefined("T".to_string(), vec![]),
        }],
        ret_type: Type::UserDefined("T".to_string(), vec![]),
        effects: vec![],
        body: vec![Stmt::Return(expr_var("x"))],
    };

    let program = Program {
        definitions: vec![TopLevel::Function(func_id)],
    };

    let mut checker = TypeChecker::new();
    assert!(checker.check_program(&program).is_ok());
}

// Test: Let Polymorphism with AST
#[test]
fn test_ast_let_polymorphism() {
    // fn id<T>(x: T) -> T { return x }
    let func_id = Function {
        name: "id".to_string(),
        type_params: vec!["T".to_string()],
        params: vec![Param {
            name: "x".to_string(),
            sigil: Sigil::Immutable,
            typ: Type::UserDefined("T".to_string(), vec![]), // Parser produces UserDefined for T
        }],
        ret_type: Type::UserDefined("T".to_string(), vec![]),
        effects: vec![],
        body: vec![Stmt::Return(expr_var("x"))],
    };

    // fn main() -> i64 {
    //   let f = id
    //   let a = f(x: 10)
    //   let b = f(x: true)
    //   return a
    // }
    let func_main = Function {
        name: "main".to_string(),
        type_params: vec![],
        params: vec![],
        ret_type: type_i64(),
        effects: vec![],
        body: vec![
            Stmt::Let {
                name: "f".to_string(),
                sigil: Sigil::Immutable,
                typ: None,
                value: expr_var("id"),
            },
            Stmt::Let {
                name: "a".to_string(),
                sigil: Sigil::Immutable,
                typ: None,
                value: expr_call("f", vec![("x", expr_lit_int(10))]),
            },
            Stmt::Let {
                name: "b".to_string(),
                sigil: Sigil::Immutable,
                typ: None,
                value: expr_call("f", vec![("x", expr_lit_bool(true))]),
            },
            Stmt::Return(expr_var("a")),
        ],
    };

    let program = Program {
        definitions: vec![TopLevel::Function(func_id), TopLevel::Function(func_main)],
    };

    let mut checker = TypeChecker::new();
    match checker.check_program(&program) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

// Test: Type Mismatch Failure
#[test]
fn test_ast_type_mismatch() {
    // fn main() -> i64 { return true }
    let func_main = Function {
        name: "main".to_string(),
        type_params: vec![],
        params: vec![],
        ret_type: type_i64(),
        effects: vec![],
        body: vec![Stmt::Return(expr_lit_bool(true))],
    };

    let program = Program {
        definitions: vec![TopLevel::Function(func_main)],
    };

    let mut checker = TypeChecker::new();
    let res = checker.check_program(&program);
    assert!(res.is_err());
    assert_eq!(res.err(), Some("Type mismatch: Bool vs I64".to_string()));
}

// Test: Generics Mismatch
#[test]
fn test_ast_generics_mismatch() {
    // fn id<T>(x: T) -> T { return 10 } // Error: 10 is I64, T is generic
    let func_id = Function {
        name: "id".to_string(),
        type_params: vec!["T".to_string()],
        params: vec![Param {
            name: "x".to_string(),
            sigil: Sigil::Immutable,
            typ: Type::UserDefined("T".to_string(), vec![]),
        }],
        ret_type: Type::UserDefined("T".to_string(), vec![]),
        effects: vec![],
        body: vec![Stmt::Return(expr_lit_int(10))],
    };

    let program = Program {
        definitions: vec![TopLevel::Function(func_id)],
    };

    let mut checker = TypeChecker::new();
    let res = checker.check_program(&program);
    assert!(res.is_err());
    // Error message depends on implementation details of unification (I64 vs Var)
    let err_msg = res.err().unwrap();
    println!("Error: {}", err_msg);
    // Should be Type mismatch: I64 vs Var("T") (or similar internal name)
    // Actually convert_user_defined_to_var converts T to Var("T").
    // So it should be mismatch I64 vs Var("T").
    assert!(err_msg.contains("Type mismatch"));
}

// Test: Verify lack of Rank-2 Polymorphism
// We cannot even syntactically write a Rank-2 type signature in the current AST.
// But we can verify that a function parameter cannot be used polymorphically.
#[test]
fn test_rank2_usage_fails() {
    // fn poly_user<F>(f: F) -> unit {
    //     let _ = f(x: 10)
    //     let _ = f(x: true) // Should fail because f is instantiated to a monotype
    //     return ()
    // }
    let func_poly_user = Function {
        name: "poly_user".to_string(),
        type_params: vec!["F".to_string()],
        params: vec![Param {
            name: "f".to_string(),
            sigil: Sigil::Immutable,
            typ: Type::UserDefined("F".to_string(), vec![]), // F is a type variable
        }],
        ret_type: type_unit(),
        effects: vec![],
        body: vec![
            Stmt::Let {
                name: "res1".to_string(),
                sigil: Sigil::Immutable,
                typ: None,
                value: expr_call("f", vec![("x", expr_lit_int(10))]),
            },
            Stmt::Let {
                name: "res2".to_string(),
                sigil: Sigil::Immutable,
                typ: None,
                value: expr_call("f", vec![("x", expr_lit_bool(true))]),
            },
            Stmt::Return(Expr::Literal(Literal::Unit)),
        ],
    };

    let program = Program {
        definitions: vec![TopLevel::Function(func_poly_user)],
    };

    let mut checker = TypeChecker::new();
    let res = checker.check_program(&program);

    // This should fail because `f` cannot be both (Int -> ?) and (Bool -> ?)
    assert!(res.is_err());
    println!("Rank-2 Rejection Error: {}", res.err().unwrap());
}
