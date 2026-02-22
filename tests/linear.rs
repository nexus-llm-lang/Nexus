use chumsky::Parser;
use nexus::lang::ast::*;
use nexus::lang::parser::parser;
use nexus::lang::typecheck::TypeChecker;

fn check(src: &str) -> Result<(), String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)
}

fn check_program(program: &Program) -> Result<(), String> {
    let mut checker = TypeChecker::new();
    checker.check_program(program).map_err(|e| e.message)
}

fn sp<T>(node: T) -> Spanned<T> {
    Spanned { node, span: 0..0 }
}

fn resource_program(drop_resource: bool) -> Program {
    let mut body = vec![sp(Stmt::Let {
        name: "r".to_string(),
        sigil: Sigil::Immutable,
        typ: None,
        value: sp(Expr::Constructor(
            "Open".to_string(),
            vec![(
                None,
                sp(Expr::Array(vec![
                    sp(Expr::Literal(Literal::Int(1))),
                    sp(Expr::Literal(Literal::Int(2))),
                    sp(Expr::Literal(Literal::Int(3))),
                ])),
            )],
        )),
    })];
    if drop_resource {
        body.push(sp(Stmt::Drop(sp(Expr::Variable(
            "r".to_string(),
            Sigil::Immutable,
        )))));
    }
    body.push(sp(Stmt::Return(sp(Expr::Literal(Literal::Unit)))));

    Program {
        definitions: vec![
            sp(TopLevel::Enum(EnumDef {
                name: "Resource".to_string(),
                is_public: false,
                type_params: vec![],
                variants: vec![
                    VariantDef {
                        name: "Open".to_string(),
                        fields: vec![(None, Type::Array(Box::new(Type::I64)))],
                    },
                    VariantDef {
                        name: "Closed".to_string(),
                        fields: vec![],
                    },
                ],
            })),
            sp(TopLevel::Let(GlobalLet {
                name: "main".to_string(),
                is_public: false,
                typ: None,
                value: sp(Expr::Lambda {
                    type_params: vec![],
                    params: vec![],
                    ret_type: Type::Unit,
                    effects: Type::Row(vec![], None),
                    body,
                }),
            })),
        ],
    }
}

#[test]
fn test_linear_basic_pass() {
    let src = r#"
    let consume = fn (x: %i64) -> unit do
        drop x
        return ()
    endfn

    let main = fn () -> unit do
        let %x = 10
        consume(x: %x)
        return ()
    endfn
    "#;
    match check(src) {
        Ok(_) => (),
        Err(e) => panic!("Failed: {}", e),
    }
}

#[test]
fn test_linear_param_accepts_plain_value_via_weakening() {
    let src = r#"
    let consume = fn (x: %i64) -> i64 do
        drop x
        return 1
    endfn

    let main = fn () -> unit do
        let y = consume(x: 10)
        drop y
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_linear_wildcard_fail() {
    let src = r#"
    let main = fn () -> unit do
        let %x = 10
        let _ = %x // Consumes %x, but binds to _, which cannot be used
        // _ is linear, so it must be used, but cannot be referred to.
        // Thus, it should fail at end of scope.
        return ()
    endfn
    "#;
    assert!(
        check(src).is_err(),
        "Should fail because _ (bound to linear) is unused"
    );
}

// test_linear_in_ref_fail: covered by prop_linear_cannot_be_stored_in_ref

#[test]
fn test_linear_match_wildcard_fail() {
    let src = r#"
    let main = fn () -> unit do
        let %x = 10
        match %x do
            case _ -> return () // Implicitly drops %x
        endmatch
    endfn
    "#;
    assert!(
        check(src).is_err(),
        "Should fail because wildcard match drops linear value"
    );
}

#[test]
fn test_linear_borrow_basic() {
    let src = r#"
    let peek = fn (x: &i64) -> unit effect { IO } do
        let msg = i64_to_string(val: x)
        perform print(val: msg)
        return ()
    endfn

    let main = fn () -> unit effect { IO } do
        let %x = 10
        let x_ref1 = borrow %x
        perform peek(x: x_ref1)
        let x_ref2 = borrow %x
        perform peek(x: x_ref2) // Borrow again
        drop %x    // Finally consume
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}

// test_linear_unused_fail: covered by prop_linear_unused_is_error
// test_linear_double_use_fail: covered by prop_linear_double_consume_is_error
// test_linear_branch_mismatch: covered by prop_linear_branch_mismatch_is_error

#[test]
fn test_generic_drop_accepts_non_linear_primitives() {
    let src = r#"
    let main = fn () -> unit do
        let x: i32 = 1
        let y: f64 = 2.0
        let s = [=[hello]=]
        drop x
        drop y
        drop s
        drop true
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_generic_drop_user_defined_linear_consumes_once() {
    let src = r#"
    type Token = {
        id: i64
    }

    let main = fn () -> unit do
        let %t: Token = { id: 1 }
        drop %t
        drop %t
        return ()
    endfn
    "#;
    assert!(check(src).is_err());
}

#[test]
fn test_enum_constructor_with_linear_arg_requires_consumption() {
    let p = resource_program(false);
    assert!(check_program(&p).is_err());
}

#[test]
fn test_enum_constructor_with_linear_arg_can_be_dropped_once() {
    let p = resource_program(true);
    assert!(check_program(&p).is_ok());
}
