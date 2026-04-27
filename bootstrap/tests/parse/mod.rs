use crate::harness::{should_fail_parse, should_typecheck};
use nexus::lang::ast::{CatchArm, Expr, Pattern, RdrName, Sigil, Stmt, TopLevel, Type};
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
    crate::harness::ensure_repo_root();
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
fn parse_import_qualified_std_path() {
    let src = r#"
    import * as io from "std:stdio"
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
    assert_eq!(import.path, "std:stdio");
    assert_eq!(import.alias.as_deref(), Some("io"));
    assert!(!import.is_external);
}

#[test]
fn parse_import_external_qualified() {
    let src = r#"
    import external "std:bundle"
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
    assert_eq!(import.path, "std:bundle");
    assert!(import.is_external);
}

#[test]
fn parse_import_third_party_package() {
    let src = r#"
    import { foo } from "mypkg:sub/foo"
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
    assert_eq!(import.path, "mypkg:sub/foo");
    assert_eq!(import.items.len(), 1);
    assert_eq!(import.items[0].name, "foo");
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
        | Boom(code) -> return code
        | _ -> return -1
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
    assert!(
        matches!(&catch_arms[0].pattern.node, Pattern::Constructor(name, _) if name.occ() == "Boom")
    );
    assert!(matches!(&catch_arms[1].pattern.node, Pattern::Wildcard));
}

#[test]
fn parse_bare_catch_with_param_binds_exception_value() {
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
    assert_eq!(catch_arms.len(), 1, "bare catch should produce single arm");
    assert!(matches!(&catch_arms[0].pattern.node, Pattern::Variable(name, _) if name == "e"));
}

#[test]
fn parse_pipe_in_expression_position_is_a_parse_error() {
    should_fail_parse(
        r#"
    let main = fn () -> i64 do
      let a = 1
      let b = 2
      let c = a | b
      return c
    end
    "#,
    );
}

#[test]
fn parse_match_with_pipe_arm_separator() {
    let src = r#"
    type Color = Red | Green | Blue

    let main = fn () -> i64 do
      let c = Red
      match c do
        | Red -> return 1
        | Green -> return 2
        | Blue -> return 3
      end
      return 0
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
    let Stmt::Expr(match_expr) = &body[1].node else {
        panic!("expected match")
    };
    let Expr::Match { cases, .. } = &match_expr.node else {
        panic!("expected match")
    };
    assert_eq!(cases.len(), 3, "expected 3 arms separated by |");
}

#[test]
fn parse_catch_with_pipe_arm_separator() {
    let src = r#"
    exception Boom(i64)

    let main = fn () -> i64 do
      try
        raise Boom(42)
      catch
        | Boom(code) -> return code
        | _ -> return -1
      end
      return 0
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
    assert_eq!(catch_arms.len(), 2);
}

#[test]
fn parse_match_or_pattern_two_alts() {
    let src = r#"
    type Color = Red | Green | Blue

    let main = fn () -> i64 do
      let c = Red
      match c do
        | Red | Green -> return 1
        | Blue -> return 2
      end
      return 0
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
    let Stmt::Expr(match_expr) = &body[1].node else {
        panic!("expected match expression at body[1]")
    };
    let Expr::Match { cases, .. } = &match_expr.node else {
        panic!("expected match")
    };
    assert_eq!(cases.len(), 2, "expected 2 match arms");
    let Pattern::Or(alts) = &cases[0].pattern.node else {
        panic!(
            "expected or-pattern in first arm, got {:?}",
            cases[0].pattern.node
        )
    };
    assert_eq!(alts.len(), 2, "first arm should have 2 alternatives");
}

#[test]
fn parse_match_or_pattern_three_alts() {
    let src = r#"
    type Color = Red | Green | Blue

    let main = fn () -> i64 do
      let c = Red
      match c do
        | Red | Green | Blue -> return 1
      end
      return 0
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
    let Stmt::Expr(match_expr) = &body[1].node else {
        panic!("expected match")
    };
    let Expr::Match { cases, .. } = &match_expr.node else {
        panic!("expected match")
    };
    let Pattern::Or(alts) = &cases[0].pattern.node else {
        panic!("expected or-pattern")
    };
    assert_eq!(alts.len(), 3, "should collect 3 alternatives");
}

#[test]
fn parse_catch_or_pattern() {
    let src = r#"
    exception Boom(i64)
    exception Crash

    let main = fn () -> i64 do
      try
        raise Boom(1)
      catch
        | Boom(_) | Crash -> return 1
      end
      return 0
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
    assert_eq!(catch_arms.len(), 1, "single arm with or-pattern");
    let Pattern::Or(alts) = &catch_arms[0].pattern.node else {
        panic!("expected or-pattern, got {:?}", catch_arms[0].pattern.node)
    };
    assert_eq!(alts.len(), 2);
}

// Helpers for record-pun tests below — pull the body of `let main` out as a
// flat statement list so each test can read its own expression directly.
fn parse_main_body(src: &str) -> Vec<Stmt> {
    let program = parser::parser().parse(src).unwrap();
    let gl = program
        .definitions
        .iter()
        .find_map(|def| {
            if let TopLevel::Let(gl) = &def.node {
                if gl.name == "main" {
                    return Some(gl);
                }
            }
            None
        })
        .expect("expected `let main`");
    let Expr::Lambda { body, .. } = &gl.value.node else {
        panic!("expected lambda for main");
    };
    body.iter().map(|s| s.node.clone()).collect()
}

#[test]
fn parse_frontend_parser_files_with_pun_extensions() {
    // Sanity: the self-hosted frontend parser modules (which themselves use
    // record-pun syntax internally) parse cleanly with the bootstrap parser.
    crate::harness::ensure_repo_root();
    for p in &["src/frontend/parser.nx", "src/frontend/parse_pattern.nx"] {
        let src = fs::read_to_string(p).unwrap_or_else(|_| panic!("read {}", p));
        parser::parser()
            .parse(&src)
            .unwrap_or_else(|e| panic!("{}: parse error {:?}", p, e));
    }
}

#[test]
fn parse_record_literal_punning_bare_ident() {
    // `{x}` desugars to `{x: x}` at parse time.
    let body = parse_main_body(
        r#"
    let main = fn () -> unit do
      let x = 7
      let r = {x}
      return ()
    end
    "#,
    );
    let Stmt::Let { value, .. } = &body[1] else {
        panic!("expected `let r = ...`");
    };
    let Expr::Record(fields) = &value.node else {
        panic!("expected Expr::Record, got {:?}", value.node);
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].0, "x", "field label should be 'x'");
    let Expr::Variable(name, sig) = &fields[0].1.node else {
        panic!("expected Variable, got {:?}", fields[0].1.node);
    };
    assert!(matches!(name, RdrName::Unqual(n) if n == "x"));
    assert!(matches!(sig, Sigil::Immutable));
}

#[test]
fn parse_record_pattern_punning_bare_ident() {
    // `let {x} = r` desugars to `let {x: x} = r`.
    let body = parse_main_body(
        r#"
    type R = { x: i64 }
    let main = fn () -> i64 do
      let r = {x: 9}
      let {x} = r
      return x
    end
    "#,
    );
    let Stmt::LetPattern { pattern, .. } = &body[1] else {
        panic!("expected destructuring let, got {:?}", body[1]);
    };
    let Pattern::Record(fields, open) = &pattern.node else {
        panic!("expected Pattern::Record, got {:?}", pattern.node);
    };
    assert!(!*open, "no `_` rest in this pattern");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].0, "x");
    let Pattern::Variable(name, sig) = &fields[0].1.node else {
        panic!("expected Variable pattern");
    };
    assert_eq!(name, "x");
    assert!(matches!(sig, Sigil::Immutable));
}

#[test]
fn parse_record_literal_punning_sigil() {
    // `{%tok}` desugars to `{tok: %tok}`.
    let body = parse_main_body(
        r#"
    let main = fn () -> unit do
      let %tok = 3
      let r = {%tok}
      return ()
    end
    "#,
    );
    let Stmt::Let { value, .. } = &body[1] else {
        panic!("expected `let r = ...`");
    };
    let Expr::Record(fields) = &value.node else {
        panic!("expected Expr::Record");
    };
    assert_eq!(fields[0].0, "tok");
    let Expr::Variable(name, sig) = &fields[0].1.node else {
        panic!("expected Variable, got {:?}", fields[0].1.node);
    };
    assert!(matches!(name, RdrName::Unqual(n) if n == "tok"));
    assert!(matches!(sig, Sigil::Linear));
}

#[test]
fn parse_record_literal_punning_borrow() {
    // `{&env}` desugars to `{env: &env}` (Expr::Borrow on the RHS).
    let body = parse_main_body(
        r#"
    let main = fn () -> unit do
      let env = 3
      let r = {&env}
      return ()
    end
    "#,
    );
    let Stmt::Let { value, .. } = &body[1] else {
        panic!("expected `let r = ...`");
    };
    let Expr::Record(fields) = &value.node else {
        panic!("expected Expr::Record");
    };
    assert_eq!(fields[0].0, "env");
    let Expr::Borrow(name, sig) = &fields[0].1.node else {
        panic!("expected Borrow, got {:?}", fields[0].1.node);
    };
    assert_eq!(name, "env");
    assert!(matches!(sig, Sigil::Immutable));
}

#[test]
fn parse_record_literal_mixed_pun_rename_and_sigil() {
    // `{name, age: a, %tok}` — bare-ident pun + rename + sigil-pun in one literal.
    let body = parse_main_body(
        r#"
    let main = fn () -> unit do
      let name = "x"
      let a = 2
      let %tok = 3
      let r = {name, age: a, %tok}
      return ()
    end
    "#,
    );
    let Stmt::Let { value, .. } = &body[3] else {
        panic!("expected `let r = ...`");
    };
    let Expr::Record(fields) = &value.node else {
        panic!("expected Expr::Record");
    };
    assert_eq!(fields.len(), 3);
    // field 0: `name` (punned)
    assert_eq!(fields[0].0, "name");
    let Expr::Variable(n0, s0) = &fields[0].1.node else {
        panic!("field 0 should be Variable");
    };
    assert!(matches!(n0, RdrName::Unqual(n) if n == "name"));
    assert!(matches!(s0, Sigil::Immutable));
    // field 1: `age: a` (renamed)
    assert_eq!(fields[1].0, "age");
    let Expr::Variable(n1, _) = &fields[1].1.node else {
        panic!("field 1 should be Variable");
    };
    assert!(matches!(n1, RdrName::Unqual(n) if n == "a"));
    // field 2: `%tok` (sigil-punned)
    assert_eq!(fields[2].0, "tok");
    let Expr::Variable(n2, s2) = &fields[2].1.node else {
        panic!("field 2 should be Variable");
    };
    assert!(matches!(n2, RdrName::Unqual(n) if n == "tok"));
    assert!(matches!(s2, Sigil::Linear));
}

#[test]
fn parse_record_literal_mixed_pun_and_rename() {
    // `{x, y: a}` — x puns, y renames.
    let body = parse_main_body(
        r#"
    let main = fn () -> unit do
      let x = 1
      let a = 2
      let r = {x, y: a}
      return ()
    end
    "#,
    );
    let Stmt::Let { value, .. } = &body[2] else {
        panic!("expected `let r = ...`");
    };
    let Expr::Record(fields) = &value.node else {
        panic!("expected Expr::Record");
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].0, "x");
    let Expr::Variable(n0, _) = &fields[0].1.node else {
        panic!("expected punned Variable for field 0");
    };
    assert!(matches!(n0, RdrName::Unqual(n) if n == "x"));
    assert_eq!(fields[1].0, "y");
    let Expr::Variable(n1, _) = &fields[1].1.node else {
        panic!("expected Variable for field 1");
    };
    assert!(
        matches!(n1, RdrName::Unqual(n) if n == "a"),
        "rename should keep RHS as 'a'"
    );
}

#[test]
fn parse_record_pattern_punning_with_open_rest() {
    // `{ name, _ }` puns `name` and ignores other fields.
    let body = parse_main_body(
        r#"
    type R = { name: string, age: i64 }
    let main = fn () -> string do
      let r = {name: "a", age: 1}
      let { name, _ } = r
      return name
    end
    "#,
    );
    let Stmt::LetPattern { pattern, .. } = &body[1] else {
        panic!("expected destructuring let");
    };
    let Pattern::Record(fields, open) = &pattern.node else {
        panic!("expected Pattern::Record");
    };
    assert!(*open, "open `_` rest should be set");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].0, "name");
    let Pattern::Variable(n, _) = &fields[0].1.node else {
        panic!("expected Variable pattern");
    };
    assert_eq!(n, "name");
}

#[test]
fn parse_record_literal_explicit_form_still_works() {
    // Sanity: `{x: x}` (no pun) still parses to the same shape as the punned form.
    let body = parse_main_body(
        r#"
    let main = fn () -> unit do
      let x = 7
      let r = {x: x}
      return ()
    end
    "#,
    );
    let Stmt::Let { value, .. } = &body[1] else {
        panic!("expected `let r = ...`");
    };
    let Expr::Record(fields) = &value.node else {
        panic!("expected Expr::Record");
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].0, "x");
    let Expr::Variable(name, _) = &fields[0].1.node else {
        panic!("expected Variable RHS");
    };
    assert!(matches!(name, RdrName::Unqual(n) if n == "x"));
}

#[test]
fn parse_record_literal_computed_rhs_does_not_pun() {
    // `{x: g(y)}` — RHS is a call, must keep explicit form.
    let body = parse_main_body(
        r#"
    let g = fn (y: i64) -> i64 do return y end
    let main = fn () -> unit do
      let y = 1
      let r = {x: g(y)}
      return ()
    end
    "#,
    );
    let Stmt::Let { value, .. } = &body[1] else {
        panic!("expected `let r = ...`");
    };
    let Expr::Record(fields) = &value.node else {
        panic!("expected Expr::Record");
    };
    assert_eq!(fields[0].0, "x");
    assert!(
        matches!(&fields[0].1.node, Expr::Call { .. }),
        "RHS should remain a Call expr (no punning), got {:?}",
        fields[0].1.node
    );
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
