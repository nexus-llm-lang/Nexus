use crate::harness::{should_fail_parse, should_typecheck};
use nexus::lang::ast::{CatchArm, Expr, Pattern, Stmt, TopLevel, Type};
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
    pub import from "examples/math.nx"
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

#[test]
fn parse_import_as_alias() {
    let src = r#"
    import { foo as bar, baz } from "examples/math.nx"
    let main = fn () -> unit do
        return ()
    end
    "#;
    let program = parser::parser().parse(src).unwrap();
    let import = program
        .definitions
        .iter()
        .find_map(|def| {
            if let TopLevel::Import(imp) = &def.node {
                Some(imp)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(import.items.len(), 2);
    assert_eq!(import.items[0].name, "foo");
    assert_eq!(import.items[0].alias.as_deref(), Some("bar"));
    assert_eq!(import.items[1].name, "baz");
    assert_eq!(import.items[1].alias, None);
}

#[test]
fn parse_linear_list_literal() {
    let src = r#"
    let main = fn () -> unit do
        let %xs: %[i64] = %[1, 2, 3]
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
        "Parser should produce Expr::List for %[1,2,3] syntax"
    );
}

#[test]
fn parse_empty_linear_list_literal() {
    let src = r#"
    let main = fn () -> unit do
        let %xs: %[i64] = %[]
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
        "Parser should produce Expr::List for %[] syntax"
    );
}

#[test]
fn parse_selective_catch_multi_arm() {
    let src = r#"
    exception Boom(i64)

    let main = fn () -> i64 do
      try
        raise Boom(42)
      catch
        case Boom(code) -> return code
        case _ -> return -1
      end
      return 0
    end
    "#;
    let program = parser::parser().parse(src).unwrap();
    // Find the try statement and verify it has 2 catch arms
    let gl = program
        .definitions
        .iter()
        .find_map(|def| {
            if let TopLevel::Let(gl) = &def.node {
                Some(gl)
            } else {
                None
            }
        })
        .unwrap();
    let Expr::Lambda { body, .. } = &gl.value.node else {
        panic!("expected lambda")
    };
    let Stmt::Try { catch_arms, .. } = &body[0].node else {
        panic!("expected try")
    };
    assert_eq!(catch_arms.len(), 2, "expected 2 catch arms");
    assert!(matches!(&catch_arms[0].pattern.node, Pattern::Constructor(name, _) if name.occ() == "Boom"));
    assert!(matches!(&catch_arms[1].pattern.node, Pattern::Wildcard));
}

#[test]
fn parse_legacy_catch_still_works() {
    let src = r#"
    exception Boom(i64)

    let main = fn () -> i64 do
      try
        raise Boom(42)
      catch e ->
        return 0
      end
      return 1
    end
    "#;
    let program = parser::parser().parse(src).unwrap();
    let gl = program
        .definitions
        .iter()
        .find_map(|def| {
            if let TopLevel::Let(gl) = &def.node {
                Some(gl)
            } else {
                None
            }
        })
        .unwrap();
    let Expr::Lambda { body, .. } = &gl.value.node else {
        panic!("expected lambda")
    };
    let Stmt::Try { catch_arms, .. } = &body[0].node else {
        panic!("expected try")
    };
    assert_eq!(
        catch_arms.len(),
        1,
        "legacy catch should produce single arm"
    );
    assert!(matches!(&catch_arms[0].pattern.node, Pattern::Variable(name, _) if name == "e"));
}

#[test]
fn parse_exception_group_def() {
    let src = r#"
    exception NotFound
    exception PermDenied
    exception group IOError = NotFound | PermDenied

    let main = fn () -> unit do
      return ()
    end
    "#;
    let program = parser::parser().parse(src).unwrap();
    let eg = program
        .definitions
        .iter()
        .find_map(|def| {
            if let TopLevel::ExceptionGroup(eg) = &def.node {
                Some(eg)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(eg.name, "IOError");
    assert_eq!(eg.members, vec!["NotFound", "PermDenied"]);
}
