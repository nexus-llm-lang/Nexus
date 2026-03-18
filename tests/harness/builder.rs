/// Fluent builder for common test patterns.
///
/// # Examples
///
/// ```ignore
/// // Compile + execute (no stdlib)
/// TestRunner::new(src).run();
///
/// // Compile + execute with stdlib
/// TestRunner::new(src).with_stdlib().run();
///
/// // From fixture, with stdlib
/// TestRunner::fixture("test_lexer.nx").with_stdlib().run();
///
/// // Expect a trap
/// let msg = TestRunner::new(src).expect_trap().run().error.unwrap();
///
/// // Compile only — inspect WASM bytes
/// let result = TestRunner::new(src).compile_only().run();
/// let wasm = result.wasm.unwrap();
///
/// // Typecheck only
/// TestRunner::new(src).typecheck_only().run();
///
/// // Expect typecheck failure
/// let err = TestRunner::new(src).expect_typecheck_error().run().error.unwrap();
/// ```
pub struct TestRunner {
    src: String,
    mode: Mode,
    stdlib: bool,
    expectation: Expectation,
}

enum Mode {
    Execute,
    CompileOnly,
    TypecheckOnly,
}

enum Expectation {
    Success,
    Trap,
    TypecheckError,
    CompileError,
}

pub struct TestResult {
    pub wasm: Option<Vec<u8>>,
    pub error: Option<String>,
}

impl TestRunner {
    pub fn new(src: &str) -> Self {
        Self {
            src: src.to_string(),
            mode: Mode::Execute,
            stdlib: false,
            expectation: Expectation::Success,
        }
    }

    pub fn fixture(name: &str) -> Self {
        Self::new(&super::fixture::read_fixture(name))
    }

    pub fn nxc_fixture(name: &str) -> Self {
        Self::new(&super::fixture::read_nxc_fixture(name))
    }

    pub fn with_stdlib(mut self) -> Self {
        self.stdlib = true;
        self
    }

    pub fn compile_only(mut self) -> Self {
        self.mode = Mode::CompileOnly;
        self
    }

    pub fn typecheck_only(mut self) -> Self {
        self.mode = Mode::TypecheckOnly;
        self
    }

    pub fn expect_trap(mut self) -> Self {
        self.expectation = Expectation::Trap;
        self
    }

    pub fn expect_typecheck_error(mut self) -> Self {
        self.mode = Mode::TypecheckOnly;
        self.expectation = Expectation::TypecheckError;
        self
    }

    pub fn expect_compile_error(mut self) -> Self {
        self.mode = Mode::CompileOnly;
        self.expectation = Expectation::CompileError;
        self
    }

    pub fn run(self) -> TestResult {
        match self.mode {
            Mode::TypecheckOnly => self.run_typecheck(),
            Mode::CompileOnly => self.run_compile(),
            Mode::Execute => self.run_execute(),
        }
    }

    fn run_typecheck(self) -> TestResult {
        match self.expectation {
            Expectation::TypecheckError => {
                let err = super::typecheck::should_fail_typecheck(&self.src);
                TestResult {
                    wasm: None,
                    error: Some(err),
                }
            }
            _ => {
                super::typecheck::should_typecheck(&self.src);
                TestResult {
                    wasm: None,
                    error: None,
                }
            }
        }
    }

    fn run_compile(self) -> TestResult {
        match self.expectation {
            Expectation::CompileError => match super::compile::try_compile(&self.src) {
                Ok(_) => panic!("expected compile error, but compilation succeeded"),
                Err(e) => TestResult {
                    wasm: None,
                    error: Some(e),
                },
            },
            _ => {
                let wasm = super::compile::compile(&self.src);
                TestResult {
                    wasm: Some(wasm),
                    error: None,
                }
            }
        }
    }

    fn run_execute(self) -> TestResult {
        let wasm = super::compile::compile(&self.src);
        match self.expectation {
            Expectation::Trap => match super::execute::run_main(&wasm) {
                Ok(()) => panic!("expected trap but main returned successfully"),
                Err(msg) => TestResult {
                    wasm: Some(wasm),
                    error: Some(msg),
                },
            },
            _ => {
                if self.stdlib {
                    super::execute::run_main_with_deps(&wasm)
                        .unwrap_or_else(|e| panic!("execution failed: {}", e));
                } else {
                    super::execute::run_main(&wasm)
                        .unwrap_or_else(|e| panic!("execution failed: {}", e));
                }
                TestResult {
                    wasm: Some(wasm),
                    error: None,
                }
            }
        }
    }
}
