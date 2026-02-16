use ariadne::{Color, Label, Report, ReportKind, Source};
use chumsky::prelude::*;
use std::fs;

mod ast;
mod parser;

use std::env;

mod interpreter;
mod typecheck;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: nexus <file.nx>");
        return;
    }
    let filename = &args[1];
    let src = fs::read_to_string(filename).expect("Failed to read file");

    let parser = parser::parser();
    let result = parser.parse(src.clone());

    match result {
        Ok(program) => {
            println!("Parsing successful!");

            let mut checker = typecheck::TypeChecker::new();
            match checker.check_program(&program) {
                Ok(_) => println!("Type checking successful!"),
                Err(e) => {
                    eprintln!("Type Error: {}", e);
                    return;
                }
            }

            let mut interpreter = interpreter::Interpreter::new(program);
            match interpreter.run_function("main", vec![]) {
                Ok(interpreter::Value::Unit) => {}, // Do not print Unit result
                Ok(res) => println!("Result: {:?}", res),
                Err(e) => {
                    // Check if error is just missing main
                    if e.contains("not found") {
                        println!("No 'main' function found. (Analysis complete)");
                    } else {
                        eprintln!("Runtime Error: {}", e);
                    }
                }
            }
        }
        Err(errors) => {
            for err in errors {
                Report::build(ReportKind::Error, filename, err.span().start)
                    .with_message(format!("{:?}", err))
                    .with_label(
                        Label::new((filename, err.span()))
                            .with_message(format!("{}", err))
                            .with_color(Color::Red),
                    )
                    .finish()
                    .print((filename, Source::from(&src)))
                    .unwrap();
            }
        }
    }
}
