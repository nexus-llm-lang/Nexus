use ariadne::{Color, Label, Report, ReportKind, Source};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use chumsky::prelude::*;

use crate::ast::Program;
use crate::interpreter::{Interpreter, Env, ExprResult};
use crate::typecheck::TypeChecker;
use crate::parser::stmt_parser;

pub fn start() {
    let mut rl = DefaultEditor::new().unwrap();
    
    // Initialize empty program for context
    let program = Program { definitions: vec![] };
    let mut interpreter = Interpreter::new(program);
    let mut checker = TypeChecker::new();
    let mut env = Env::new();

    println!("Nexus REPL v0.1.0");
    println!("Type 'exit' or Ctrl-D to quit.");

    loop {
        let readline = rl.readline(">> ");
        match readline {
            Ok(line) => {
                let line_str = line.trim();
                if line_str == "exit" {
                    break;
                }
                if line_str.is_empty() {
                    continue;
                }

                rl.add_history_entry(line_str).unwrap();

                // Parse
                let parser = stmt_parser().then_ignore(end());
                let result = parser.parse(line_str);

                match result {
                    Ok(stmt) => {
                        // Typecheck
                        match checker.check_repl_stmt(&stmt) {
                            Ok(typ) => {
                                // Interpret
                                match interpreter.eval_repl_stmt(&stmt, &mut env) {
                                    Ok(res) => {
                                        match res {
                                            ExprResult::Normal(val) => {
                                                println!("{:?} : {:?}", val, typ);
                                            },
                                            ExprResult::EarlyReturn(val) => {
                                                println!("returned {:?} : {:?}", val, typ);
                                            }
                                        }
                                    },
                                    Err(e) => println!("Runtime Error: {}", e),
                                }
                            },
                            Err(e) => {
                                Report::build(ReportKind::Error, "<repl>", e.span.start)
                                    .with_message(e.message.clone())
                                    .with_label(
                                        Label::new(("<repl>", e.span))
                                            .with_message(e.message)
                                            .with_color(Color::Red),
                                    )
                                    .finish()
                                    .print(("<repl>", Source::from(&line_str)))
                                    .unwrap();
                            },
                        }
                    },
                    Err(errors) => {
                        for err in errors {
                            Report::build(ReportKind::Error, "<repl>", err.span().start)
                                .with_message(format!("{:?}", err))
                                .with_label(
                                    Label::new(("<repl>", err.span()))
                                        .with_message(format!("{}", err))
                                        .with_color(Color::Red),
                                )
                                .finish()
                                .print(("<repl>", Source::from(&line_str)))
                                .unwrap();
                        }
                    }
                }
            },
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                break;
            },
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                break;
            },
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }
}
