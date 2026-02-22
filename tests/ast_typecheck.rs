use nexus::lang::ast::*;
use nexus::lang::typecheck::TypeChecker;

fn spanned<T>(node: T) -> Spanned<T> {
    Spanned { node, span: 0..0 }
}

fn expr_lit_int(i: i64) -> Spanned<Expr> {
    spanned(Expr::Literal(Literal::Int(i)))
}
fn expr_lit_bool(b: bool) -> Spanned<Expr> {
    spanned(Expr::Literal(Literal::Bool(b)))
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
fn test_basic_poly() {
    let program = Program {
        definitions: vec![spanned(TopLevel::Let(GlobalLet {
            name: "id".to_string(),
            is_public: false,
            typ: None,
            value: spanned(Expr::Lambda {
                type_params: vec!["T".to_string()],
                params: vec![Param {
                    name: "x".to_string(),
                    sigil: Sigil::Immutable,
                    typ: Type::UserDefined("T".to_string(), vec![]),
                }],
                ret_type: Type::UserDefined("T".to_string(), vec![]),
                effects: Type::Row(vec![], None),
                body: vec![spanned(Stmt::Return(expr_var("x")))],
            }),
        }))],
    };
    let mut checker = TypeChecker::new();
    assert!(checker.check_program(&program).is_ok());
}

#[test]
fn test_complex_typecheck() {
    let program = Program {
        definitions: vec![
            spanned(TopLevel::Let(GlobalLet {
                name: "id".to_string(),
                is_public: false,
                typ: None,
                value: spanned(Expr::Lambda {
                    type_params: vec!["T".to_string()],
                    params: vec![Param {
                        name: "x".to_string(),
                        sigil: Sigil::Immutable,
                        typ: Type::UserDefined("T".to_string(), vec![]),
                    }],
                    ret_type: Type::UserDefined("T".to_string(), vec![]),
                    effects: Type::Row(vec![], None),
                    body: vec![spanned(Stmt::Return(expr_var("x")))],
                }),
            })),
            spanned(TopLevel::Let(GlobalLet {
                name: "main".to_string(),
                is_public: false,
                typ: None,
                value: spanned(Expr::Lambda {
                    type_params: vec![],
                    params: vec![],
                    ret_type: Type::Unit,
                    effects: Type::Row(vec![], None),
                    body: vec![
                        spanned(Stmt::Let {
                            name: "f".to_string(),
                            sigil: Sigil::Immutable,
                            typ: None,
                            value: expr_var("id"),
                        }),
                        spanned(Stmt::Let {
                            name: "res1".to_string(),
                            sigil: Sigil::Immutable,
                            typ: None,
                            value: expr_call("f", vec![("x", expr_lit_int(10))]),
                        }),
                        spanned(Stmt::Let {
                            name: "res2".to_string(),
                            sigil: Sigil::Immutable,
                            typ: None,
                            value: expr_call("f", vec![("x", expr_lit_bool(true))]),
                        }),
                        spanned(Stmt::Return(spanned(Expr::Literal(Literal::Unit)))),
                    ],
                }),
            })),
        ],
    };
    let mut checker = TypeChecker::new();
    assert!(checker.check_program(&program).is_ok());
}

#[test]
fn test_mismatch_fail() {
    let program = Program {
        definitions: vec![spanned(TopLevel::Let(GlobalLet {
            name: "main".to_string(),
            is_public: false,
            typ: None,
            value: spanned(Expr::Lambda {
                type_params: vec![],
                params: vec![],
                ret_type: Type::I64,
                effects: Type::Row(vec![], None),
                body: vec![spanned(Stmt::Return(expr_lit_bool(true)))],
            }),
        }))],
    };
    let mut checker = TypeChecker::new();
    let res = checker.check_program(&program);
    assert!(res.is_err());
}

#[test]
fn test_labeled_call_out_of_order_typechecks() {
    let program = Program {
        definitions: vec![
            spanned(TopLevel::Let(GlobalLet {
                name: "sub".to_string(),
                is_public: false,
                typ: None,
                value: spanned(Expr::Lambda {
                    type_params: vec![],
                    params: vec![
                        Param {
                            name: "a".to_string(),
                            sigil: Sigil::Immutable,
                            typ: Type::I64,
                        },
                        Param {
                            name: "b".to_string(),
                            sigil: Sigil::Immutable,
                            typ: Type::I64,
                        },
                    ],
                    ret_type: Type::I64,
                    effects: Type::Row(vec![], None),
                    body: vec![spanned(Stmt::Return(spanned(Expr::BinaryOp(
                        Box::new(expr_var("a")),
                        "-".to_string(),
                        Box::new(expr_var("b")),
                    ))))],
                }),
            })),
            spanned(TopLevel::Let(GlobalLet {
                name: "main".to_string(),
                is_public: false,
                typ: None,
                value: spanned(Expr::Lambda {
                    type_params: vec![],
                    params: vec![],
                    ret_type: Type::Unit,
                    effects: Type::Row(vec![], None),
                    body: vec![
                        spanned(Stmt::Let {
                            name: "x".to_string(),
                            sigil: Sigil::Immutable,
                            typ: None,
                            value: expr_call(
                                "sub",
                                vec![("b", expr_lit_int(2)), ("a", expr_lit_int(10))],
                            ),
                        }),
                        spanned(Stmt::Return(spanned(Expr::Literal(Literal::Unit)))),
                    ],
                }),
            })),
        ],
    };
    let mut checker = TypeChecker::new();
    assert!(checker.check_program(&program).is_ok());
}
