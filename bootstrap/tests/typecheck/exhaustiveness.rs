use crate::harness::{should_fail_typecheck, should_typecheck};
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

fn color_program_with_cases(case_ctors: &[&str]) -> Program {
    let cases = case_ctors
        .iter()
        .map(|ctor| MatchCase {
            pattern: sp(Pattern::Constructor(RdrName::Unqual((*ctor).to_string()), vec![])),
            body: vec![sp(Stmt::Return(sp(Expr::Literal(Literal::Unit))))],
        })
        .collect();

    Program {
        source_file: None,
        source_text: None,
        definitions: vec![
            sp(TopLevel::Enum(EnumDef {
                name: "Color".to_string(),
                is_public: false,
                is_opaque: false,
                type_params: vec![],
                variants: vec![
                    VariantDef {
                        name: "Red".to_string(),
                        fields: vec![],
                    },
                    VariantDef {
                        name: "Green".to_string(),
                        fields: vec![],
                    },
                    VariantDef {
                        name: "Blue".to_string(),
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
                    requires: Type::Row(vec![], None),
                    throws: Type::Row(vec![], None),
                    body: vec![
                        sp(Stmt::Let {
                            name: "c".to_string(),
                            sigil: Sigil::Immutable,
                            typ: None,
                            value: sp(Expr::Constructor(RdrName::Unqual("Red".to_string()), vec![])),
                        }),
                        sp(Stmt::Expr(sp(Expr::Match {
                            target: Box::new(sp(Expr::Variable(RdrName::Unqual("c".to_string()), Sigil::Immutable))),
                            cases,
                        }))),
                        sp(Stmt::Return(sp(Expr::Literal(Literal::Unit)))),
                    ],
                }),
            })),
        ],
    }
}

#[test]
fn test_nested_result_exhaustive() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let x: Result<Result<i64, i64>, i64> = Ok(val: Ok(val: 1))
        match x do
            case Ok(val: Ok(val: v)) -> return ()
            case Ok(val: Err(err: e)) -> return ()
            case Err(err: e) -> return ()
        end
    end
    "#,
    );
}

#[test]
fn test_nested_result_non_exhaustive() {
    should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let x: Result<Result<i64, i64>, i64> = Ok(val: Ok(val: 1))
        match x do
            case Ok(val: Ok(val: v)) -> return ()
            // Missing Ok(val: Err(err: _)) case
            case Err(err: e) -> return ()
        end
    end
    "#,
    );
}

#[test]
fn test_bool_exhaustive() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let b = true
        match b do
            case true -> return ()
            case false -> return ()
        end
    end
    "#,
    );
}

#[test]
fn test_bool_non_exhaustive() {
    should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let b = true
        match b do
            case true -> return ()
            // Missing false
        end
    end
    "#,
    );
}

#[test]
fn test_wildcard_exhaustive() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let i = 10
        match i do
            case 0 -> return ()
            case _ -> return ()
        end
    end
    "#,
    );
}

#[test]
fn test_int_non_exhaustive() {
    should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let i = 10
        match i do
            case 0 -> return ()
            case 1 -> return ()
            // Missing wildcard for integer
        end
    end
    "#,
    );
}

#[test]
fn test_record_exhaustive() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let r = { x: true, y: true }
        match r do
            case { x: true, y: true } -> return ()
            case { x: true, y: false } -> return ()
            case { x: false, _ } -> return ()
        end
    end
    "#,
    );
}

#[test]
fn test_record_non_exhaustive() {
    should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let r = { x: true, y: true }
        match r do
            case { x: true, y: true } -> return ()
            // Missing cases
        end
    end
    "#,
    );
}

#[test]
fn test_enum_exhaustive() {
    let p = color_program_with_cases(&["Red", "Green", "Blue"]);
    assert!(check_program(&p).is_ok());
}

#[test]
fn test_enum_non_exhaustive() {
    let p = color_program_with_cases(&["Red"]);
    assert!(
        check_program(&p).is_err(),
        "Should fail due to missing Green and Blue"
    );
}

#[test]
fn test_or_pattern_covers_all_constructors_is_exhaustive() {
    should_typecheck(
        r#"
    type Color = Red | Green | Blue

    let main = fn () -> unit do
        let c = Red
        match c do
            case Red | Green -> return ()
            case Blue -> return ()
        end
    end
    "#,
    );
}

#[test]
fn test_or_pattern_missing_constructor_is_non_exhaustive() {
    should_fail_typecheck(
        r#"
    type Color = Red | Green | Blue

    let main = fn () -> unit do
        let c = Red
        match c do
            case Red | Green -> return ()
        end
    end
    "#,
    );
}

#[test]
fn test_or_pattern_alternatives_must_bind_same_variables() {
    should_fail_typecheck(
        r#"
    type Either = Left(v: i64) | Right(v: i64)

    let main = fn () -> unit do
        let e = Left(v: 1)
        match e do
            case Left(v: v) | Right(v: w) -> return ()
        end
    end
    "#,
    );
}

#[test]
fn test_or_pattern_alternatives_with_same_binding_typechecks() {
    should_typecheck(
        r#"
    type Either = Left(v: i64) | Right(v: i64)

    let extract = fn (e: Either) -> i64 do
        match e do
            case Left(v: v) | Right(v: v) -> return v
        end
        return 0
    end

    let main = fn () -> unit do
        let _ = extract(e: Left(v: 1))
        return ()
    end
    "#,
    );
}

const COLOR_VARIANTS: [&str; 3] = ["Red", "Green", "Blue"];

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 16,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_enum_any_proper_subset_is_non_exhaustive(mask in 1u8..8) {
        let cases: Vec<&str> = COLOR_VARIANTS
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, v)| *v)
            .collect();
        let p = color_program_with_cases(&cases);
        let result = check_program(&p);
        if mask == 0b111 {
            prop_assert!(result.is_ok(), "all variants should be exhaustive");
        } else {
            prop_assert!(result.is_err(), "subset {:?} should be non-exhaustive", cases);
        }
    }

    #[test]
    fn prop_bool_exhaustiveness(has_true in any::<bool>(), has_false in any::<bool>()) {
        // Skip the case where neither is present (empty match is a different concern)
        prop_assume!(has_true || has_false);

        let mut cases = String::new();
        if has_true {
            cases.push_str("            case true -> return ()\n");
        }
        if has_false {
            cases.push_str("            case false -> return ()\n");
        }
        let src = format!(
            r#"
    let main = fn () -> unit do
        let b = true
        match b do
{cases}        end
    end
"#
        );
        if has_true && has_false {
            should_typecheck(&src);
        } else {
            should_fail_typecheck(&src);
        }
    }
}
