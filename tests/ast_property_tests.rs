use nexus::ast::*;
use nexus::typecheck::TypeChecker;

// --- Helpers ---
fn spanned<T>(node: T) -> Spanned<T> {
    Spanned { node, span: 0..0 }
}

fn type_i64() -> Type { Type::I64 }
fn type_bool() -> Type { Type::Bool }
fn type_unit() -> Type { Type::Unit }
fn type_var(n: &str) -> Type { Type::UserDefined(n.to_string(), vec![]) }
fn type_arrow(params: Vec<Type>, ret: Type) -> Type {
    let labeled: Vec<(String, Type)> = params.into_iter().enumerate().map(|(i, t)| (format!("arg{}", i), t)).collect();
    Type::Arrow(labeled, Box::new(ret), Box::new(Type::Row(vec![], None)))
}

fn expr_lit_int(i: i64) -> Spanned<Expr> {
    spanned(Expr::Literal(Literal::Int(i)))
}
fn expr_var(n: &str) -> Spanned<Expr> {
    spanned(Expr::Variable(n.to_string(), Sigil::Immutable))
}
fn expr_call(func: &str, args: Vec<(&str, Spanned<Expr>)>) -> Spanned<Expr> {
    spanned(Expr::Call {
        func: func.to_string(),
        args: args.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        perform: false,
    })
}

#[test]
fn test_prop_identity_polymorphism() {
    let func_id = Function {
        name: "id".to_string(),
        is_public: false,
        type_params: vec!["T".to_string()],
        params: vec![Param {
            name: "x".to_string(),
            sigil: Sigil::Immutable,
            typ: type_var("T"),
        }],
        ret_type: type_var("T"),
        effects: Type::Row(vec![], None),
        body: vec![spanned(Stmt::Return(expr_var("x")))],
    };

    let func_main = Function {
        name: "main".to_string(),
        is_public: false,
        type_params: vec![],
        params: vec![],
        ret_type: type_unit(),
        effects: Type::Row(vec![], None),
        body: vec![
            spanned(Stmt::Let {
                name: "i".to_string(),
                sigil: Sigil::Immutable,
                typ: None,
                value: expr_call("id", vec![("x", expr_lit_int(1))]),
            }),
            spanned(Stmt::Let {
                name: "_1".to_string(),
                sigil: Sigil::Immutable,
                typ: None,
                value: spanned(Expr::BinaryOp(
                    Box::new(expr_var("i")),
                    "+".to_string(),
                    Box::new(expr_lit_int(1)),
                )),
            }),
            spanned(Stmt::Let {
                name: "b".to_string(),
                sigil: Sigil::Immutable,
                typ: None,
                value: expr_call("id", vec![("x", spanned(Expr::Literal(Literal::Bool(true))))]),
            }),
            spanned(Stmt::Expr(spanned(Expr::If {
                cond: Box::new(expr_var("b")),
                then_branch: vec![],
                else_branch: None,
            }))),
            spanned(Stmt::Return(spanned(Expr::Literal(Literal::Unit)))),
        ],
    };

    let program = Program {
        definitions: vec![spanned(TopLevel::Function(func_id)), spanned(TopLevel::Function(func_main))],
    };
    let mut checker = TypeChecker::new();
    let res = checker.check_program(&program);
    if let Err(e) = &res {
        println!("Type check failed: {} at {:?}", e.message, e.span);
    }
    assert!(res.is_ok());
}

#[test]
fn test_prop_k_combinator() {
    let func_k = Function {
        name: "k".to_string(),
        is_public: false,
        type_params: vec!["A".to_string(), "B".to_string()],
        params: vec![
            Param { name: "a".to_string(), sigil: Sigil::Immutable, typ: type_var("A") },
            Param { name: "b".to_string(), sigil: Sigil::Immutable, typ: type_var("B") },
        ],
        ret_type: type_var("A"),
        effects: Type::Row(vec![], None),
        body: vec![spanned(Stmt::Return(expr_var("a")))],
    };

    let func_main = Function {
        name: "main".to_string(),
        is_public: false,
        type_params: vec![],
        params: vec![],
        ret_type: type_unit(),
        effects: Type::Row(vec![], None),
        body: vec![
            spanned(Stmt::Let {
                name: "res".to_string(),
                sigil: Sigil::Immutable,
                typ: None,
                value: expr_call("k", vec![("a", expr_lit_int(1)), ("b", spanned(Expr::Literal(Literal::Bool(true))))]),
            }),
            spanned(Stmt::Return(spanned(Expr::Literal(Literal::Unit)))),
        ],
    };

    let program = Program {
        definitions: vec![spanned(TopLevel::Function(func_k)), spanned(TopLevel::Function(func_main))],
    };
    let mut checker = TypeChecker::new();
    let res = checker.check_program(&program);
    if let Err(e) = &res {
        println!("Type check failed: {} at {:?}", e.message, e.span);
    }
    assert!(res.is_ok());
}

#[test]
fn test_prop_occurs_check_fail() {
    let func_self_apply = Function {
        name: "self_apply".to_string(),
        is_public: false,
        type_params: vec!["T".to_string()],
        params: vec![Param { name: "f".to_string(), sigil: Sigil::Immutable, typ: type_var("T") }],
        ret_type: type_var("T"),
        effects: Type::Row(vec![], None),
        body: vec![spanned(Stmt::Return(expr_call("f", vec![("val", expr_var("f"))])))],
    };

    let program = Program {
        definitions: vec![spanned(TopLevel::Function(func_self_apply))],
    };
    let mut checker = TypeChecker::new();
    let res = checker.check_program(&program);
    assert!(res.is_err());
    let err = res.err().unwrap();
    assert!(err.message.contains("Recursive") || err.message.contains("Infinite") || err.message.contains("Mismatch"));
}

#[test]
fn test_prop_higher_order_apply() {
    let func_apply = Function {
        name: "apply".to_string(),
        is_public: false,
        type_params: vec!["A".to_string(), "B".to_string()],
        params: vec![
            Param { name: "f".to_string(), sigil: Sigil::Immutable, typ: type_arrow(vec![type_var("A")], type_var("B")) },
            Param { name: "x".to_string(), sigil: Sigil::Immutable, typ: type_var("A") },
        ],
        ret_type: type_var("B"),
        effects: Type::Row(vec![], None),
        body: vec![spanned(Stmt::Return(expr_call("f", vec![("arg0", expr_var("x"))])))],
    };

    let func_to_int = Function {
        name: "to_int".to_string(),
        is_public: false,
        type_params: vec![],
        params: vec![Param { name: "arg0".to_string(), sigil: Sigil::Immutable, typ: type_bool() }],
        ret_type: type_i64(),
        effects: Type::Row(vec![], None),
        body: vec![spanned(Stmt::Expr(spanned(Expr::If {
            cond: Box::new(expr_var("arg0")),
            then_branch: vec![spanned(Stmt::Return(expr_lit_int(1)))],
            else_branch: Some(vec![spanned(Stmt::Return(expr_lit_int(0)))]),
        })))],
    };

    let func_main = Function {
        name: "main".to_string(),
        is_public: false,
        type_params: vec![],
        params: vec![],
        ret_type: type_i64(),
        effects: Type::Row(vec![], None),
        body: vec![
            spanned(Stmt::Return(expr_call("apply", vec![("f", expr_var("to_int")), ("x", spanned(Expr::Literal(Literal::Bool(true))))]))),
        ],
    };

    let program = Program {
        definitions: vec![spanned(TopLevel::Function(func_apply)), spanned(TopLevel::Function(func_to_int)), spanned(TopLevel::Function(func_main))],
    };

    let mut checker = TypeChecker::new();
    let res = checker.check_program(&program);
    if let Err(e) = &res {
        println!("Type check failed: {} at {:?}", e.message, e.span);
    }
    assert!(res.is_ok());
}

#[test]
fn test_prop_parametricity_violation() {
    let func_bad = Function {
        name: "bad".to_string(),
        is_public: false,
        type_params: vec!["T".to_string()],
        params: vec![Param { name: "x".to_string(), sigil: Sigil::Immutable, typ: type_var("T") }],
        ret_type: type_var("T"),
        effects: Type::Row(vec![], None),
        body: vec![spanned(Stmt::Return(expr_lit_int(42)))],
    };

    let program = Program { definitions: vec![spanned(TopLevel::Function(func_bad))] };
    let mut checker = TypeChecker::new();
    let res = checker.check_program(&program);
    assert!(res.is_err());
    assert!(res.unwrap_err().message.contains("Mismatch"));
}
