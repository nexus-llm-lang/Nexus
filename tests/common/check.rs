//! Typecheck-only helpers. No execution.

use nexus::lang::parser;
use nexus::lang::typecheck::TypeChecker;

/// Parse + typecheck. Asserts success.
pub fn should_typecheck(src: &str) {
    let program = parser::parser()
        .parse(src)
        .unwrap_or_else(|e| panic!("parse failed: {:?}", e));
    let mut checker = TypeChecker::new();
    if let Err(e) = checker.check_program(&program) {
        panic!("expected typecheck to pass, got error: {}", e.message);
    }
}

/// Parse + typecheck. Asserts failure. Returns error message.
pub fn should_fail_typecheck(src: &str) -> String {
    let program = match parser::parser().parse(src) {
        Ok(p) => p,
        Err(e) => return format!("parse error: {:?}", e),
    };
    let mut checker = TypeChecker::new();
    match checker.check_program(&program) {
        Err(e) => e.message,
        Ok(_) => panic!("expected typecheck to fail, but it passed"),
    }
}

/// Parse + typecheck + return warnings.
pub fn typecheck_warnings(src: &str) -> Vec<String> {
    let program = parser::parser()
        .parse(src)
        .unwrap_or_else(|e| panic!("parse failed: {:?}", e));
    let mut checker = TypeChecker::new();
    checker
        .check_program(&program)
        .unwrap_or_else(|e| panic!("typecheck failed: {}", e.message));
    checker
        .take_warnings()
        .into_iter()
        .map(|w| w.message)
        .collect()
}

/// Parse only. Asserts failure.
pub fn should_fail_parse(src: &str) {
    if parser::parser().parse(src).is_ok() {
        panic!("expected parse to fail, but it passed");
    }
}
