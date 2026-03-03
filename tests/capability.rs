mod common;

use nexus::interpreter::Interpreter;
use nexus::lang::ast;
use nexus::lang::parser;
use nexus::lang::typecheck::TypeChecker;
use nexus::runtime::ExecutionCapabilities;

fn parse_and_check(src: &str) -> ast::Program {
    let program = parser::parser()
        .parse(src)
        .expect("parse should succeed");
    let mut checker = TypeChecker::new();
    checker
        .check_program(&program)
        .expect("typecheck should succeed");
    program
}

fn extract_main_requires(program: &ast::Program) -> Option<&ast::Type> {
    program.definitions.iter().find_map(|def| {
        if let ast::TopLevel::Let(gl) = &def.node {
            if gl.name == "main" {
                if let ast::Expr::Lambda { requires, .. } = &gl.value.node {
                    return Some(requires);
                }
            }
        }
        None
    })
}

#[test]
fn static_capability_check_rejects_missing_net() {
    let src = r#"
    let main = fn () -> unit require { PermNet } do
        return ()
    end
    "#;
    let program = parse_and_check(src);
    let caps = ExecutionCapabilities::deny_all();

    let requires = extract_main_requires(&program).expect("main should have requires");
    let result = caps.validate_program_requires(requires);
    assert!(result.is_err(), "should reject missing --allow-net");
    let err = result.unwrap_err();
    assert!(err.contains("--allow-net"), "got: {}", err);
    assert!(err.contains("PermNet"), "got: {}", err);
}

#[test]
fn static_capability_check_passes_when_net_allowed() {
    let src = r#"
    let main = fn () -> unit require { PermNet } do
        return ()
    end
    "#;
    let program = parse_and_check(src);
    let caps = ExecutionCapabilities {
        allow_net: true,
        ..ExecutionCapabilities::deny_all()
    };

    let requires = extract_main_requires(&program).expect("main should have requires");
    assert!(caps.validate_program_requires(requires).is_ok());

    // Execution also succeeds
    let mut interp = Interpreter::new_with_capabilities(program, caps);
    let result = interp.run_function("main", vec![]);
    assert!(
        result.is_ok(),
        "execution should succeed: {:?}",
        result.err()
    );
}

#[test]
fn static_capability_check_rejects_multiple_missing() {
    let src = r#"
    let main = fn () -> unit require { PermNet, PermConsole } do
        return ()
    end
    "#;
    let program = parse_and_check(src);
    let caps = ExecutionCapabilities::deny_all();

    let requires = extract_main_requires(&program).expect("main should have requires");
    let result = caps.validate_program_requires(requires);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("--allow-net"), "got: {}", err);
    assert!(err.contains("--allow-console"), "got: {}", err);
}

#[test]
fn no_requires_clause_passes_with_deny_all() {
    let src = r#"
    let main = fn () -> unit do
        return ()
    end
    "#;
    let program = parse_and_check(src);
    let caps = ExecutionCapabilities::deny_all();
    let mut interp = Interpreter::new_with_capabilities(program, caps);
    let result = interp.run_function("main", vec![]);
    assert!(result.is_ok(), "no-require main should run with deny_all");
}
