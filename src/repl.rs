use ariadne::{Color, Label, Report, ReportKind, Source};
use chumsky::prelude::*;
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Config, Helper};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::ast::{Program, Stmt};
use crate::interpreter::{Env, ExprResult, Interpreter};
use crate::parser::stmt_parser;
use crate::typecheck::TypeChecker;

struct NexusHelper {
    vars: Rc<RefCell<HashSet<String>>>,
}

impl Completer for NexusHelper {
    type Candidate = Pair;
    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let (start, word) =
            rustyline::completion::extract_word(line, pos, None, |c| " \t\n\r(){}[],.".contains(c));
        let mut candidates = Vec::new();
        let vars = self.vars.borrow();
        for var in vars.iter() {
            if var.starts_with(word) {
                candidates.push(Pair {
                    display: var.clone(),
                    replacement: var.clone(),
                });
            }
        }
        Ok((start, candidates))
    }
}

impl Hinter for NexusHelper {
    type Hint = String;
}
impl Highlighter for NexusHelper {}
impl Validator for NexusHelper {}
impl Helper for NexusHelper {}

pub fn start() {
    let config = Config::builder().history_ignore_space(true).build();

    let vars = Rc::new(RefCell::new(HashSet::new()));
    let helper = NexusHelper { vars: vars.clone() };

    let mut rl =
        rustyline::Editor::<NexusHelper, rustyline::history::DefaultHistory>::with_config(config)
            .unwrap();
    rl.set_helper(Some(helper));

    let history_file = ".nexus_history";
    if rl.load_history(history_file).is_err() {
        // No history
    }

    // Initialize environment
    let mut env = Env::new();
    let stdlib_names = vec![
        "print",
        "i64_to_string",
        "float_to_string",
        "bool_to_string",
        "drop_i64",
        "drop_array",
    ];
    for name in &stdlib_names {
        vars.borrow_mut().insert(name.to_string());
    }

    let program = Program {
        definitions: vec![],
    };
    let mut interpreter = Interpreter::new(program);
    let mut checker = TypeChecker::new();

    println!("Nexus REPL v0.1.0");
    println!("Type ':exit' or Ctrl-D to quit. Type ':help' for commands.");

    loop {
        let readline = rl.readline(">> ");
        match readline {
            Ok(line) => {
                let line_str = line.trim();

                if line_str.starts_with(':') {
                    match line_str {
                        ":exit" | ":quit" => break,
                        ":help" => {
                            println!("Available commands:");
                            println!("  :exit, :quit  Exit the REPL");
                            println!("  :help         Show this help message");
                            println!("  :vars         Show loaded variables");
                            continue;
                        }
                        ":vars" => {
                            let v = vars.borrow();
                            let mut list: Vec<_> = v.iter().collect();
                            list.sort();
                            println!("Variables: {:?}", list);
                            continue;
                        }
                        _ => {
                            println!("Unknown command: {}", line_str);
                            continue;
                        }
                    }
                }

                if line_str.is_empty() {
                    continue;
                }

                let _ = rl.add_history_entry(line.as_str());

                // Parse
                let parser = stmt_parser().then_ignore(end());
                let result = parser.parse(line_str);

                match result {
                    Ok(stmt) => {
                        // Track new variables
                        match &stmt.node {
                            Stmt::Let { name, sigil, .. } => {
                                vars.borrow_mut().insert(sigil.get_key(name));
                            }
                            _ => {}
                        }

                        // Typecheck
                        match checker.check_repl_stmt(&stmt) {
                            Ok(typ) => {
                                // Interpret
                                match interpreter.eval_repl_stmt(&stmt, &mut env) {
                                    Ok(res) => match res {
                                        ExprResult::Normal(val) => {
                                            println!("{} : {}", val, typ);
                                        }
                                        ExprResult::EarlyReturn(val) => {
                                            println!("returned {} : {}", val, typ);
                                        }
                                    },
                                    Err(e) => println!("Runtime Error: {}", e),
                                }
                            }
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
                            }
                        }
                    }
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
            }
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }

    if let Err(e) = rl.save_history(history_file) {
        println!("Error saving history: {}", e);
    }
}
