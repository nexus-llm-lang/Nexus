use crate::common::source::{check_raw as check, check_warnings};
use nexus::lang::ast::*;
use nexus::lang::typecheck::TypeChecker;

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
    let src = r#"
    let consume = fn (x: %i64) -> unit do
        return ()
    end

    let main = fn () -> unit do
        let %x = 10
        consume(x: %x)
        return ()
    end
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
        return 1
    end

    let main = fn () -> unit do
        let y = consume(x: 10)
        return ()
    end
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_linear_primitive_auto_drop_pass() {
    let src = r#"
    let main = fn () -> unit do
        let %x = 10
        // No explicit consumption needed for primitives
        return ()
    end
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_linear_primitive_wildcard_pass() {
    let src = r#"
    let main = fn () -> unit do
        let %x = 10
        let _ = %x // Allowed for primitives
        return ()
    end
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_linear_primitive_match_wildcard_pass() {
    let src = r#"
    let main = fn () -> unit do
        let %x = 10
        match %x do
            case _ -> return () // Allowed for primitives
        end
    end
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_linear_borrow_basic() {
    let src = r#"
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
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_generic_drop_accepts_non_linear_primitives() {
    let src = r#"
    let main = fn () -> unit do
        let x: i32 = 1
        let y: f64 = 2.0
        let s = "hello"
        return ()
    end
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
        return ()
    end
    "#;
    assert!(check(src).is_err());
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
fn test_linear_primitive_emits_unnecessary_warning() {
    let warnings = check_warnings(
        r#"
let main = fn () -> unit do
    let %x = 42
    return ()
end
"#,
    );
    assert!(
        warnings.iter().any(|w| w.contains("unnecessary")),
        "expected warning about unnecessary linear sigil on primitive, got: {:?}",
        warnings,
    );
}

#[test]
fn test_linear_record_does_not_emit_unnecessary_warning() {
    let warnings = check_warnings(
        r#"
    let main = fn () -> unit do
        let %r = { id: 1 }
        match %r do case _ -> () end
        return ()
    end
"#,
    );
    assert!(
        !warnings.iter().any(|w| w.contains("unnecessary")),
        "unexpected warning for linear record: {:?}",
        warnings,
    );
}

#[test]
fn test_adt_with_linear_arg_is_promoted_to_linear() {
    // An ADT that wraps a linear value should itself be linear,
    // even without explicit % on the outer binding.
    let src = r#"
    type Wrapper<T> = Wrap(val: T)

    let main = fn () -> unit do
        let %r = { id: 1 }
        let w = Wrap(val: %r)
        return ()
    end
    "#;
    assert!(
        check(src).is_err(),
        "Expected error: ADT wrapping linear value should require consumption"
    );
}

#[test]
fn test_adt_with_linear_arg_consumed_once_passes() {
    let src = r#"
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
    "#;
    match check(src) {
        Ok(_) => (),
        Err(e) => panic!("Expected OK but got: {}", e),
    }
}

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_linear_primitive_drops(x in 0i64..100) {
        let src = format!("
let main = fn () -> i64 do
    let %a = {}
    match %a do case _ -> () end
    return 1
end
", x);
        assert!(crate::common::source::check(&src).is_ok());
    }

    #[test]
    fn prop_linear_shadowing_requires_consumption(val in 0i64..100) {
        // Shadowing a linear variable makes the outer variable unconsumable, which is an error
        let src = format!("
let main = fn () -> i64 do
    let %a = {}
    let %a = {}
    match %a do case _ -> () end
    match %a do case _ -> () end
    return 1
end
", val, val);
        assert!(crate::common::source::check(&src).is_err());
    }
}
