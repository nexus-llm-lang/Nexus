use crate::harness::should_fail_parse;
use nexus::lang::ast::{Expr, Type};
use nexus::lang::parser;
use std::fs;

#[test]
fn parse_enum_declaration_syntax_is_rejected() {
    should_fail_parse(
        r#"
    enum Color { Red, Green }
    let main = fn () -> unit do
        return ()
    end
    "#,
    );
}

#[test]
fn parse_empty_fn_body_is_error() {
    should_fail_parse(
        r#"
    let main = fn () -> unit do end
    "#,
    );
}

#[test]
fn parse_pub_import_syntax_is_rejected() {
    should_fail_parse(
        r#"
    pub import from examples/math.nx
    let main = fn () -> i64 do
      return 0
    end
    "#,
    );
}

#[test]
fn parse_all_examples() {
    for entry in fs::read_dir("examples").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().map_or(false, |e| e == "nx") {
            let src = fs::read_to_string(&path).unwrap();
            parser::parser()
                .parse(&src)
                .unwrap_or_else(|e| panic!("{}: parse error: {:?}", path.display(), e));
        }
    }
}

#[test]
fn parse_list_type_is_builtin() {
    let src = r#"
    let main = fn () -> unit do
        let xs: [i64] = [1, 2, 3]
        return ()
    end
    "#;
    let program = parser::parser().parse(src).unwrap();
    let found_list_type = program.definitions.iter().any(|def| {
        if let nexus::lang::ast::TopLevel::Let(gl) = &def.node {
            if let Expr::Lambda { body, .. } = &gl.value.node {
                body.iter().any(|stmt| {
                    if let nexus::lang::ast::Stmt::Let { typ: Some(t), .. } = &stmt.node {
                        matches!(t, Type::List(_))
                    } else {
                        false
                    }
                })
            } else {
                false
            }
        } else {
            false
        }
    });
    assert!(
        found_list_type,
        "Parser should produce Type::List for [i64] syntax"
    );
}

#[test]
fn parse_list_expr_is_builtin() {
    let src = r#"
    let main = fn () -> unit do
        let xs = [1, 2, 3]
        return ()
    end
    "#;
    let program = parser::parser().parse(src).unwrap();
    let found_list_expr = program.definitions.iter().any(|def| {
        if let nexus::lang::ast::TopLevel::Let(gl) = &def.node {
            if let Expr::Lambda { body, .. } = &gl.value.node {
                body.iter().any(|stmt| {
                    if let nexus::lang::ast::Stmt::Let { value, .. } = &stmt.node {
                        matches!(value.node, Expr::List(_))
                    } else {
                        false
                    }
                })
            } else {
                false
            }
        } else {
            false
        }
    });
    assert!(
        found_list_expr,
        "Parser should produce Expr::List for [1,2,3] syntax"
    );
}
