use crate::harness::exec;
use nexus::lang::parser;
use nexus::lang::typecheck::TypeChecker;
use nexus::runtime::ExecutionCapabilities;

fn parse_and_check(src: &str) -> nexus::lang::ast::Program {
    let program = parser::parser().parse(src).expect("parse should succeed");
    let mut checker = TypeChecker::new();
    checker
        .check_program(&program)
        .expect("typecheck should succeed");
    program
}

fn extract_main_requires(program: &nexus::lang::ast::Program) -> Option<&nexus::lang::ast::Type> {
    program.definitions.iter().find_map(|def| {
        if let nexus::lang::ast::TopLevel::Let(gl) = &def.node {
            if gl.name == "main" {
                if let nexus::lang::ast::Expr::Lambda { requires, .. } = &gl.value.node {
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
    assert!(
        err.contains("--allow-net"),
        "error should mention --allow-net, got: {}",
        err
    );
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

    exec(src);
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
    assert!(
        err.contains("--allow-net") && err.contains("--allow-console"),
        "error should mention both --allow-net and --allow-console, got: {}",
        err
    );
}

#[test]
fn no_requires_clause_passes_with_deny_all() {
    exec(
        r#"
    let main = fn () -> unit do
        return ()
    end
    "#,
    );
}
