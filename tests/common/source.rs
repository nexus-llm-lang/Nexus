use nexus::interpreter::{Interpreter, Value};
use nexus::lang::parser;
use nexus::lang::typecheck::TypeChecker;
use std::time::{SystemTime, UNIX_EPOCH};

/// Rename `let main = fn ()` to `pub let __test = fn ()` and append a dummy main.
pub fn prepare_test_source(src: &str) -> String {
    let s = src.replace("let main = fn ()", "pub let __test = fn ()");
    format!("{}\nlet main = fn () -> unit do\n  return ()\nend\n", s)
}

/// Parse + typecheck without any source preparation.
/// Used by typecheck-only tests (effect.rs, exhaustiveness.rs, float.rs, record.rs, etc.)
pub fn check_raw(src: &str) -> Result<(), String> {
    let program = parser::parser()
        .parse(src)
        .map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&program).map_err(|e| e.message)
}

/// Prepare source + parse + typecheck.
pub fn check(src: &str) -> Result<(), String> {
    let src = prepare_test_source(src);
    check_raw(&src)
}

/// Prepare source + parse + typecheck + interpret `__test`.
pub fn run(src: &str) -> Result<Value, String> {
    let src = prepare_test_source(src);
    let p = parser::parser()
        .parse(&src)
        .map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)?;
    let mut interpreter = Interpreter::new(p);
    interpreter.run_function("__test", vec![])
}

/// Parse + typecheck + interpret named function (without source preparation).
pub fn run_raw(src: &str, fn_name: &str) -> Result<Value, String> {
    let program = parser::parser()
        .parse(src)
        .map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker
        .check_program(&program)
        .map_err(|e| format!("type error: {}", e.message))?;
    let mut interpreter = Interpreter::new(program);
    interpreter.run_function(fn_name, vec![])
}

/// Parse + typecheck + extract warnings.
pub fn check_warnings(src: &str) -> Vec<String> {
    let p = parser::parser().parse(src).unwrap();
    let mut checker = TypeChecker::new();
    checker.check_program(&p).unwrap();
    checker
        .take_warnings()
        .into_iter()
        .map(|w| w.message)
        .collect()
}

/// Load file, prepare source, parse + typecheck + interpret `__test`.
pub fn check_and_run(src_path: &str) -> Result<(), String> {
    let raw_src = std::fs::read_to_string(src_path).map_err(|e| e.to_string())?;
    let src = prepare_test_source(&raw_src);
    let parser = parser::parser();
    let program = parser.parse(&src).map_err(|e| format!("{:?}", e))?;

    let mut checker = TypeChecker::new();
    checker.check_program(&program).map_err(|e| e.message)?;

    let mut interpreter = Interpreter::new(program);
    interpreter
        .run_function("__test", vec![])
        .map(|_| ())
}

/// RAII guard for temporary directories — cleans up on drop.
pub struct TempDirGuard {
    path: String,
}

impl TempDirGuard {
    pub fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        Self {
            path: format!("/tmp/nexus_fs_test_{}_{}", label, nanos),
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
