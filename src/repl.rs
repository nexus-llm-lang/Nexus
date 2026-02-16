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
                let line = line.trim();
                if line == "exit" {
                    break;
                }
                if line.is_empty() {
                    continue;
                }

                rl.add_history_entry(line).unwrap();

                // Parse
                let parser = stmt_parser().then_ignore(end());
                let result = parser.parse(line);

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
                            Err(e) => println!("Type Error: {}", e),
                        }
                    },
                    Err(errs) => {
                        for err in errs {
                            println!("Parse Error: {:?}", err);
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
