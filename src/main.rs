use ariadne::{Color, Label, Report, ReportKind, Source};
use chumsky::prelude::*;
use std::fs;

mod ast;
mod lang;
mod parser;

use std::env;

mod interpreter;
mod repl;
mod typecheck;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 || args[1] == "--repl" {
        repl::start();
        return;
    }
    let filename = &args[1];
    let src = fs::read_to_string(filename).expect("Failed to read file");

    let parser = parser::parser();
    let result = parser.parse(src.clone());

    match result {
        Ok(program) => {
            let mut checker = typecheck::TypeChecker::new();
            match checker.check_program(&program) {
                Ok(_) => {}
                Err(e) => {
                    Report::build(ReportKind::Error, filename, e.span.start)
                        .with_message(e.message.clone())
                        .with_label(
                            Label::new((filename, e.span))
                                .with_message(e.message)
                                .with_color(Color::Red),
                        )
                        .finish()
                        .print((filename, Source::from(&src)))
                        .unwrap();
                    return;
                }
            }

            let mut interpreter = interpreter::Interpreter::new(program);
            match interpreter.run_function("main", vec![]) {
                Ok(interpreter::Value::Unit) => {} // Do not print Unit result
                Ok(res) => println!("Result: {:?}", res),
                Err(e) => {
                    if !e.contains("not found") {
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
