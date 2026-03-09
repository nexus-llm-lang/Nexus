use crate::common::check::{should_fail_typecheck, should_typecheck};
use nexus::lang::ast::*;
use nexus::lang::typecheck::TypeChecker;
use proptest::prelude::*;

fn check_program(program: &Program) -> Result<(), String> {
    let mut checker = TypeChecker::new();
    checker.check_program(program).map_err(|e| e.message)
}

fn sp<T>(node: T) -> Spanned<T> {
    Spanned { node, span: 0..0 }
}

fn resource_program(consume_resource: bool) -> Program {
    let mut body = vec![sp(Stmt::Let {
        name: "r".to_string(),
        sigil: Sigil::Linear,
        typ: None,
        value: sp(Expr::Record(vec![(
            "id".to_string(),
            sp(Expr::Literal(Literal::Int(1))),
        )])),
    })];
    if consume_resource {
        body.push(sp(Stmt::Expr(sp(Expr::Match {
            target: Box::new(sp(Expr::Variable("r".to_string(), Sigil::Linear))),
            cases: vec![MatchCase {
                pattern: sp(Pattern::Wildcard),
                body: vec![],
            }],
        }))));
    }
    body.push(sp(Stmt::Return(sp(Expr::Literal(Literal::Unit)))));

    Program {
        definitions: vec![sp(TopLevel::Let(GlobalLet {
            name: "main".to_string(),
            is_public: false,
            typ: None,
            value: sp(Expr::Lambda {
                type_params: vec![],
                params: vec![],
                ret_type: Type::Unit,
                requires: Type::Row(vec![], None),
                effects: Type::Row(vec![], None),
                body,
            }),
        }))],
    }
}

#[test]
fn test_linear_basic_pass() {
    should_typecheck(
        r#"
    let consume = fn (x: %i64) -> unit do
        return ()
    end

    let main = fn () -> unit do
        let %x = 10
        consume(x: %x)
        return ()
    end
    "#,
    );
}

#[test]
fn test_linear_param_accepts_plain_value_via_weakening() {
    should_typecheck(
        r#"
    let consume = fn (x: %i64) -> i64 do
        return 1
    end

    let main = fn () -> unit do
        let y = consume(x: 10)
        return ()
    end
    "#,
    );
}

#[test]
fn test_linear_primitive_auto_drop_pass() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let %x = 10
        // No explicit consumption needed for primitives
        return ()
    end
    "#,
    );
}

#[test]
fn test_linear_primitive_wildcard_pass() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let %x = 10
        let _ = %x // Allowed for primitives
        return ()
    end
    "#,
    );
}

#[test]
fn test_linear_primitive_match_wildcard_pass() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let %x = 10
        match %x do
            case _ -> return () // Allowed for primitives
        end
    end
    "#,
    );
}

#[test]
fn test_linear_borrow_basic() {
    should_typecheck(
        r#"
    import { Console }, * as stdio from stdlib/stdio.nx
    import { from_i64 } from stdlib/string.nx
    let peek = fn (x: &i64) -> unit require { Console } do
        let msg = from_i64(val: x)
        Console.print(val: msg)
        return ()
    end

    let main = fn () -> unit require { PermConsole } do
        inject stdio.system_handler do
            let %x = 10
            let x_ref1 = &%x
            peek(x: x_ref1)
            let x_ref2 = &%x
            peek(x: x_ref2)
        end
        return ()
    end
    "#,
    );
}

#[test]
fn test_generic_drop_accepts_non_linear_primitives() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let x: i32 = 1
        let y: f64 = 2.0
        let s = "hello"
        return ()
    end
    "#,
    );
}

#[test]
fn test_generic_drop_user_defined_linear_consumes_once() {
    should_fail_typecheck(
        r#"
    type Token = {
        id: i64
    }

    let main = fn () -> unit do
        let %t: Token = { id: 1 }
        return ()
    end
    "#,
    );
}

#[test]
fn test_enum_constructor_with_linear_arg_requires_consumption() {
    let p = resource_program(false);
    assert!(check_program(&p).is_err());
}

#[test]
fn test_enum_constructor_with_linear_arg_can_be_consumed_once() {
    let p = resource_program(true);
    assert!(check_program(&p).is_ok());
}

#[test]
fn test_adt_with_linear_arg_is_promoted_to_linear() {
    should_fail_typecheck(
        r#"
    type Wrapper<T> = Wrap(val: T)

    let main = fn () -> unit do
        let %r = { id: 1 }
        let w = Wrap(val: %r)
        return ()
    end
    "#,
    );
}

#[test]
fn test_adt_with_linear_arg_consumed_once_passes() {
    should_typecheck(
        r#"
    type Wrapper<T> = Wrap(val: T)

    let main = fn () -> unit do
        let %r = { id: 1 }
        let w = Wrap(val: %r)
        match w do
            case Wrap(val: inner) ->
                match inner do case { id: _ } -> () end
        end
        return ()
    end
    "#,
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_linear_primitive_drops(x in 0i64..100) {
        let src = format!("
let test_fn = fn () -> i64 do
    let %a = {}
    match %a do case _ -> () end
    return 1
end
", x);
        should_typecheck(&src);
    }

    #[test]
    fn prop_linear_shadowing_requires_consumption(val in 0i64..100) {
        // Shadowing a linear variable makes the outer variable unconsumable, which is an error
        let src = format!("
let test_fn = fn () -> i64 do
    let %a = {}
    let %a = {}
    match %a do case _ -> () end
    match %a do case _ -> () end
    return 1
end
", val, val);
        should_fail_typecheck(&src);
    }
}
