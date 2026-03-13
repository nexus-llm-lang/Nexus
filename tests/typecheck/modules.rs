use crate::harness::{should_fail_parse, should_fail_typecheck, should_typecheck};
use nexus::lang::parser;
use nexus::lang::stdlib::list_stdlib_nx_paths;
use nexus::lang::typecheck::TypeChecker;
use std::fs;

#[test]
fn test_import_external_syntax() {
    let src = r#"
    import external math.wasm
    export external add = "add" : (a: i64, b: i64) -> i64
    let main = fn () -> unit do return () end
    "#;
    should_typecheck(src);
}

#[test]
fn test_pub_import_syntax_is_rejected() {
    let src = r#"
    pub import from examples/math.nx
    let main = fn () -> i64 do
      return 0
    end
    "#;
    should_fail_parse(src);
}

#[test]
fn test_stdlib_module_not_auto_exported() {
    let src = r#"
    let main = fn () -> i64 do
      let xs = [1, 2, 3]
      return list.length(xs: xs)
    end
    "#;
    let err = should_fail_typecheck(src);
    assert!(!err.is_empty());
}

#[test]
fn test_typechecker_does_not_register_drop_function() {
    let checker = TypeChecker::new();
    assert!(checker.env.get("drop").is_none());
}

#[test]
fn test_stdlib_loader_uses_nx_only() {
    let paths = list_stdlib_nx_paths().expect("failed to list stdlib paths");
    assert!(!paths.is_empty(), "stdlib .nx files should exist");
    for p in paths {
        assert_eq!(p.extension().and_then(|s| s.to_str()), Some("nx"));
    }
}

#[test]
fn test_stdio_defines_console_port_and_system_handler() {
    let src = fs::read_to_string("nxlib/stdlib/stdio.nx").unwrap();
    let parser = parser::parser();
    let program = parser.parse(&src).unwrap();

    let has_console_port = program
        .definitions
        .iter()
        .any(|d| matches!(&d.node, nexus::lang::ast::TopLevel::Port(p) if p.name == "Console"));
    assert!(
        has_console_port,
        "Console port should be defined in stdio.nx"
    );

    let let_names: Vec<String> = program
        .definitions
        .iter()
        .filter_map(|d| match &d.node {
            nexus::lang::ast::TopLevel::Let(gl) => Some(gl.name.clone()),
            _ => None,
        })
        .collect();
    assert!(
        let_names.contains(&"system_handler".to_string()),
        "system_handler should be defined in stdio.nx"
    );
}

#[test]
fn all_examples_parse() {
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
fn all_examples_typecheck() {
    for entry in fs::read_dir("examples").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().map_or(false, |e| e == "nx") {
            let src = fs::read_to_string(&path).unwrap();
            let program = parser::parser()
                .parse(&src)
                .unwrap_or_else(|e| panic!("{}: parse error: {:?}", path.display(), e));
            let mut checker = TypeChecker::new();
            checker
                .check_program(&program)
                .unwrap_or_else(|e| panic!("{}: typecheck error: {}", path.display(), e.message));
        }
    }
}
