//! JIT REPL — accumulate-and-recompile
//!
//! Each REPL input:
//! 1. Parse input (expression, statement, or top-level definition)
//! 2. Append to accumulated state
//! 3. Wrap accumulated stmts in synthetic `main` function
//! 4. Run full pipeline: parse → typecheck → lower → codegen → wasmtime
//! 5. Execute (stdout inherited — output appears directly in terminal)
//! 6. On error: roll back appended input, show error

use ariadne::{Color, Label, Report, ReportKind, Source};
use rustyline::error::ReadlineError;
use rustyline::Config;

use crate::compiler::{bundler, codegen};
use crate::constants::{Permission, ENTRYPOINT};
use crate::lang::ast::{Expr, GlobalLet, Program, Stmt, TopLevel};
use crate::lang::parser::{parser, stmt_parser, ParseError};
use crate::lang::typecheck::TypeChecker;
use crate::runtime::backtrace;
use crate::runtime::conc::add_nexus_host_stubs;
use crate::runtime::ExecutionCapabilities;
use crate::types::{Literal, Spanned, Type};

use wasmtime::{Engine, Linker, Module, Store};
use wasmtime_wasi::WasiCtxBuilder;

/// Accumulated REPL state
struct ReplState {
    /// All accumulated top-level definitions (fn, type, port, etc.)
    top_levels: Vec<Spanned<TopLevel>>,
    /// Wasmtime engine (reused across compilations)
    engine: Engine,
    /// Capability policy for WASI setup
    capabilities: ExecutionCapabilities,
}

impl ReplState {
    fn new(capabilities: ExecutionCapabilities) -> Self {
        ReplState {
            top_levels: Vec::new(),
            engine: Engine::default(),
            capabilities,
        }
    }

    /// Create a fresh type checker and check the given program
    fn typecheck(&self, program: &Program) -> Result<(), crate::lang::typecheck::TypeError> {
        let mut checker = TypeChecker::new();
        checker.check_program(program)
    }

    /// Build a complete program from accumulated state + a body for main
    fn build_program(&self, main_body: Vec<Spanned<Stmt>>) -> Program {
        let mut defs = self.top_levels.clone();

        // REPL requires all permissions so users don't hit requires errors.
        // The capabilities/WASI layer already gates actual access.
        let requires = Type::Row(
            Permission::ALL
                .iter()
                .map(|p| Type::UserDefined(p.perm_name().to_string(), vec![]))
                .collect(),
            None,
        );

        // Add synthetic main function wrapping the body
        let main_fn = Spanned {
            node: TopLevel::Let(GlobalLet {
                name: "main".to_string(),
                is_public: false,
                typ: None,
                value: Spanned {
                    node: Expr::Lambda {
                        type_params: vec![],
                        params: vec![],
                        ret_type: Type::Unit,
                        requires,
                        throws: Type::Row(vec![], None),
                        body: main_body,
                    },
                    span: 0..0,
                },
            }),
            span: 0..0,
        };
        defs.push(main_fn);

        Program {
            definitions: defs,
            source_file: Some("<repl>".to_string()),
            source_text: None,
        }
    }

    /// Compile and execute the program via wasmtime.
    /// Stdout is inherited — output appears directly in the terminal.
    fn compile_and_run(&self, program: &Program) -> Result<(), String> {
        let wasm =
            codegen::compile_program_to_wasm(program).map_err(|e| format!("Compile error: {e}"))?;

        let config = bundler::BundleConfig::default();
        let wasm = bundler::bundle_core_wasm(&wasm, &config)?;

        let module = Module::from_binary(&self.engine, &wasm)
            .map_err(|e| format!("WASM load error: {e}"))?;

        let mut linker = Linker::<wasmtime_wasi::p1::WasiP1Ctx>::new(&self.engine);
        wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx| ctx)
            .map_err(|e| format!("WASI link error: {e}"))?;
        self.capabilities
            .enforce_denied_wasi_functions(&mut linker)
            .map_err(|e| format!("WASI capability enforcement error: {e}"))?;

        // stdlib.wasm is monolithic and imports nexus:cli/nexus-host for net FFI.
        // Add no-op stubs so instantiation succeeds even when net isn't used.
        add_nexus_host_stubs(&mut linker);

        // Backtrace host functions — always add for REPL (stdlib may need them).
        backtrace::reset();
        backtrace::add_bt_to_linker(&mut linker)
            .map_err(|e| format!("Backtrace link error: {e}"))?;

        let mut builder = WasiCtxBuilder::new();
        builder.inherit_stdio();

        // Apply capability policy (fs, net, etc.)
        if let Err(msg) = self.capabilities.apply_to_wasi_builder(&mut builder) {
            return Err(format!("Capability error: {msg}"));
        }

        let mut store = Store::new(&self.engine, builder.build_p1());

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| format!("Instantiation error: {e}"))?;

        let main = instance
            .get_typed_func::<(), ()>(&mut store, ENTRYPOINT)
            .map_err(|e| format!("Missing main: {e}"))?;

        main.call(&mut store, ())
            .map_err(|e| format!("Runtime error: {e}"))
    }
}

enum ReplInput {
    Stmt(Spanned<Stmt>),
    TopLevels(Vec<Spanned<TopLevel>>),
}

enum ParseState {
    Complete(ReplInput),
    Incomplete,
    Error(Vec<ParseError>),
}

fn parse_input_for_repl(input: &str) -> ParseState {
    // If input is only whitespace/comments, treat as empty program
    if let Ok(program) = parser().parse(input) {
        if program.definitions.is_empty() {
            return ParseState::Complete(ReplInput::TopLevels(vec![]));
        }
    }

    let stmt = stmt_parser().parse(input);
    if let Ok(stmt) = stmt {
        return ParseState::Complete(ReplInput::Stmt(stmt));
    }

    let top_level_parser = parser();
    if let Ok(program) = top_level_parser.parse(input) {
        if !program.definitions.is_empty() {
            return ParseState::Complete(ReplInput::TopLevels(program.definitions));
        }
    }

    let top_err = parser().parse(input).err();
    let stmt_err = stmt_parser().parse(input).err();

    let mut any_incomplete = false;
    if let Some(errors) = &top_err {
        any_incomplete |= is_incomplete_input(input, errors);
    }
    if let Some(errors) = &stmt_err {
        any_incomplete |= is_incomplete_input(input, errors);
    }
    if any_incomplete {
        return ParseState::Incomplete;
    }

    if let Some(errors) = stmt_err {
        return ParseState::Error(errors);
    }
    if let Some(errors) = top_err {
        return ParseState::Error(errors);
    }
    ParseState::Error(vec![])
}

fn is_incomplete_input(input: &str, errors: &[ParseError]) -> bool {
    if input.trim().is_empty() {
        return false;
    }
    let len = input.len();
    errors.iter().any(|err| {
        let at_end = err.span.end >= len.saturating_sub(1);
        at_end && (err.message.contains("expected") || err.message.contains("got Eof"))
    })
}

/// Starts the JIT REPL session.
/// Uses accumulate-and-recompile: each input is compiled and executed via wasmtime.
pub fn start(capabilities: ExecutionCapabilities) {
    // REPL always enables console so print/println work
    let mut repl_caps = capabilities;
    repl_caps.allow_console = true;

    let config = Config::builder().history_ignore_space(true).build();

    let mut rl =
        rustyline::Editor::<(), rustyline::history::DefaultHistory>::with_config(config).unwrap();

    let history_file = ".nexus_history";
    if rl.load_history(history_file).is_err() {
        // No history
    }

    let mut state = ReplState::new(repl_caps);
    let mut buffer = String::new();
    let mut stmt_history: Vec<Spanned<Stmt>> = Vec::new();

    println!("Nexus REPL (JIT compiled)");
    println!("Type :help for available commands, :exit to quit.\n");

    loop {
        let prompt = if buffer.is_empty() { ">> " } else { ".. " };
        let readline = rl.readline(prompt);
        match readline {
            Ok(line) => {
                let line_str = line.trim_end();

                if buffer.is_empty() && line_str.starts_with(':') {
                    match line_str {
                        ":exit" | ":quit" => break,
                        ":help" => {
                            println!("Available commands:");
                            println!("  :exit, :quit  Exit the REPL");
                            println!("  :help         Show this help message");
                            println!("  :reset        Reset accumulated state");
                            println!("  :defs         Show accumulated definitions");
                            continue;
                        }
                        ":reset" => {
                            let caps = state.capabilities.clone();
                            state = ReplState::new(caps);
                            stmt_history.clear();
                            println!("State reset.");
                            continue;
                        }
                        ":defs" => {
                            if state.top_levels.is_empty() {
                                println!("(no definitions)");
                            } else {
                                for def in &state.top_levels {
                                    match &def.node {
                                        TopLevel::Let(gl) => {
                                            println!("  let {} = ...", gl.name);
                                        }
                                        TopLevel::TypeDef(td) => {
                                            println!("  type {}", td.name);
                                        }
                                        TopLevel::Enum(ed) => {
                                            println!("  type {} = ...", ed.name);
                                        }
                                        TopLevel::Port(p) => {
                                            println!("  port {}", p.name);
                                        }
                                        TopLevel::Exception(ex) => {
                                            println!("  exception {}", ex.name);
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            continue;
                        }
                        _ => {
                            println!("Unknown command: {}", line_str);
                            continue;
                        }
                    }
                }

                if buffer.is_empty() && line_str.trim().is_empty() {
                    continue;
                }

                buffer.push_str(&line);
                buffer.push('\n');

                match parse_input_for_repl(&buffer) {
                    ParseState::Complete(input) => {
                        let _ = rl.add_history_entry(buffer.trim_end());
                        match input {
                            ReplInput::Stmt(stmt) => {
                                handle_stmt(&mut state, &mut stmt_history, &stmt, &buffer);
                            }
                            ReplInput::TopLevels(defs) => {
                                if !defs.is_empty() {
                                    handle_top_levels(&mut state, defs, &buffer);
                                }
                            }
                        }
                        buffer.clear();
                    }
                    ParseState::Incomplete => {}
                    ParseState::Error(errors) => {
                        for err in errors {
                            Report::build(ReportKind::Error, "<repl>", err.span.start)
                                .with_message(&err.message)
                                .with_label(
                                    Label::new(("<repl>", err.span.clone()))
                                        .with_message(&err.message)
                                        .with_color(Color::Red),
                                )
                                .finish()
                                .print(("<repl>", Source::from(&*buffer)))
                                .unwrap();
                        }
                        buffer.clear();
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                if buffer.is_empty() {
                    break;
                }
                buffer.clear();
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(err) => {
                eprintln!("Readline Error: {:?}", err);
                break;
            }
        }
    }

    if let Err(e) = rl.save_history(history_file) {
        eprintln!("Error saving history: {}", e);
    }
}

fn handle_stmt(
    state: &mut ReplState,
    stmt_history: &mut Vec<Spanned<Stmt>>,
    stmt: &Spanned<Stmt>,
    source: &str,
) {
    // Build a program with all accumulated stmts + new stmt as main body
    let mut body = stmt_history.clone();
    body.push(stmt.clone());

    // Add a return () at the end if the last statement isn't a return
    let needs_return = !matches!(stmt.node, Stmt::Return(_));
    if needs_return {
        body.push(Spanned {
            node: Stmt::Return(Spanned {
                node: Expr::Literal(Literal::Unit),
                span: 0..0,
            }),
            span: 0..0,
        });
    }

    let program = state.build_program(body);

    // Typecheck
    match state.typecheck(&program) {
        Ok(()) => {}
        Err(e) => {
            Report::build(ReportKind::Error, "<repl>", e.span.start)
                .with_message(e.message.clone())
                .with_label(
                    Label::new(("<repl>", e.span))
                        .with_message(e.message)
                        .with_color(Color::Red),
                )
                .finish()
                .print(("<repl>", Source::from(source)))
                .unwrap();
            return;
        }
    }

    // Compile and execute
    match state.compile_and_run(&program) {
        Ok(()) => {
            // Execution succeeded — commit the statement
            stmt_history.push(stmt.clone());

            // For let bindings, show the binding was created
            if let Stmt::Let { name, .. } = &stmt.node {
                println!("  {} defined", name);
            }
        }
        Err(msg) => {
            eprintln!("{}", msg);
        }
    }
}

fn handle_top_levels(state: &mut ReplState, defs: Vec<Spanned<TopLevel>>, source: &str) {
    // Save state for rollback
    let saved_top_levels = state.top_levels.clone();

    // Tentatively add definitions
    state.top_levels.extend(defs.clone());

    // Build program with a trivial main to validate
    let program = state.build_program(vec![Spanned {
        node: Stmt::Return(Spanned {
            node: Expr::Literal(Literal::Unit),
            span: 0..0,
        }),
        span: 0..0,
    }]);

    // Typecheck
    match state.typecheck(&program) {
        Ok(()) => {
            // Compile and execute (trivial main, just validates)
            match state.compile_and_run(&program) {
                Ok(()) => {
                    for def in &defs {
                        match &def.node {
                            TopLevel::Let(gl) => println!("  {} defined", gl.name),
                            TopLevel::TypeDef(td) => println!("  type {} defined", td.name),
                            TopLevel::Enum(ed) => println!("  type {} defined", ed.name),
                            TopLevel::Port(p) => println!("  port {} defined", p.name),
                            TopLevel::Exception(ex) => {
                                println!("  exception {} defined", ex.name)
                            }
                            TopLevel::Import(imp) => {
                                let alias = imp.alias.as_deref().unwrap_or(imp.path.as_str());
                                println!("  imported {}", alias);
                            }
                        }
                    }
                }
                Err(msg) => {
                    // Rollback
                    state.top_levels = saved_top_levels;
                    eprintln!("{}", msg);
                }
            }
        }
        Err(e) => {
            // Rollback
            state.top_levels = saved_top_levels;
            Report::build(ReportKind::Error, "<repl>", e.span.start)
                .with_message(e.message.clone())
                .with_label(
                    Label::new(("<repl>", e.span))
                        .with_message(e.message)
                        .with_color(Color::Red),
                )
                .finish()
                .print(("<repl>", Source::from(source)))
                .unwrap();
        }
    }
}
