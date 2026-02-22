use ariadne::{Color, Label, Report, ReportKind, Source};
use chumsky::prelude::*;
use rustyline::error::ReadlineError;
use rustyline::Config;

use super::{Env, ExprResult, Interpreter};
use crate::lang::ast::{Program, Spanned, Stmt, TopLevel};
use crate::lang::parser::{parser, stmt_parser};
use crate::lang::typecheck::TypeChecker;

enum ReplInput {
    Stmt(Spanned<Stmt>),
    TopLevels(Vec<Spanned<TopLevel>>),
}

enum ParseState {
    Complete(ReplInput),
    Incomplete,
    Error(Vec<Simple<char>>),
}

fn parse_input_for_repl(input: &str) -> ParseState {
    let stmt = stmt_parser().then_ignore(end()).parse(input);
    if let Some(stmt) = stmt.ok() {
        return ParseState::Complete(ReplInput::Stmt(stmt));
    }

    let top_level_parser = parser();
    if let Ok(program) = top_level_parser.parse(input) {
        if !program.definitions.is_empty() {
            return ParseState::Complete(ReplInput::TopLevels(program.definitions));
        }
    }

    let top_err = parser().parse(input).err();
    let stmt_err = stmt_parser().then_ignore(end()).parse(input).err();

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

fn is_incomplete_input(input: &str, errors: &[Simple<char>]) -> bool {
    if input.trim().is_empty() {
        return false;
    }
    let len = input.chars().count();
    errors.iter().any(|err| {
        let at_end = err.span().end >= len.saturating_sub(1);
        err.found().is_none() && at_end
    })
}

/// Starts the interactive Nexus REPL session.
pub fn start() {
    let config = Config::builder().history_ignore_space(true).build();

    let mut rl =
        rustyline::Editor::<(), rustyline::history::DefaultHistory>::with_config(config).unwrap();

    let history_file = ".nexus_history";
    if rl.load_history(history_file).is_err() {
        // No history
    }

    // Initialize environment
    let mut env = Env::new();
    let program = Program {
        definitions: vec![],
    };
    let mut interpreter = Interpreter::new(program);
    let mut checker = TypeChecker::new();
    let mut top_level_defs: Vec<Spanned<TopLevel>> = Vec::new();

    let mut buffer = String::new();

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
                            ReplInput::Stmt(stmt) => match checker.check_repl_stmt(&stmt) {
                                Ok(typ) => match interpreter.eval_repl_stmt(&stmt, &mut env) {
                                    Ok(res) => match res {
                                        ExprResult::Normal(val) => {
                                            println!("{} : {}", val, typ);
                                        }
                                        ExprResult::EarlyReturn(val) => {
                                            println!("returned {} : {}", val, typ);
                                        }
                                    },
                                    Err(e) => println!("Runtime Error: {}", e),
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
                                        .print(("<repl>", Source::from(&buffer)))
                                        .unwrap();
                                }
                            },
                            ReplInput::TopLevels(defs) => {
                                let program = Program {
                                    definitions: defs.clone(),
                                };
                                match checker.check_program(&program) {
                                    Ok(()) => {
                                        top_level_defs.extend(defs);
                                        interpreter = Interpreter::new(Program {
                                            definitions: top_level_defs.clone(),
                                        });
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
                                            .print(("<repl>", Source::from(&buffer)))
                                            .unwrap();
                                    }
                                }
                            }
                        }
                        buffer.clear();
                    }
                    ParseState::Incomplete => {}
                    ParseState::Error(errors) => {
                        for err in errors {
                            Report::build(ReportKind::Error, "<repl>", err.span().start)
                                .with_message(format!("{:?}", err))
                                .with_label(
                                    Label::new(("<repl>", err.span()))
                                        .with_message(format!("{}", err))
                                        .with_color(Color::Red),
                                )
                                .finish()
                                .print(("<repl>", Source::from(&buffer)))
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

#[cfg(test)]
mod tests {
    use super::{parse_input_for_repl, ParseState, ReplInput};

    #[test]
    fn parse_complete_single_line_stmt() {
        assert!(matches!(
            parse_input_for_repl("let x = 1"),
            ParseState::Complete(ReplInput::Stmt(_))
        ));
    }

    #[test]
    fn parse_incomplete_if_stmt() {
        assert!(matches!(
            parse_input_for_repl("if true then"),
            ParseState::Incomplete
        ));
    }

    #[test]
    fn parse_complete_multi_line_if_stmt() {
        let src = "if true then\n  let x = 1\nelse\n  let x = 2\nendif";
        assert!(matches!(
            parse_input_for_repl(src),
            ParseState::Complete(ReplInput::Stmt(_))
        ));
    }

    #[test]
    fn parse_complete_top_level_fn() {
        let src = "let id = fn (x: i64) -> i64 do\n  return x\nendfn";
        assert!(matches!(
            parse_input_for_repl(src),
            ParseState::Complete(ReplInput::Stmt(_))
        ));
    }

    #[test]
    fn parse_complete_top_level_block_comment() {
        assert!(matches!(
            parse_input_for_repl("/* top-level comment */"),
            ParseState::Complete(ReplInput::Stmt(_))
                | ParseState::Complete(ReplInput::TopLevels(_))
        ));
    }

    #[test]
    fn parse_complete_multi_line_if_with_block_comment_stmt() {
        let src =
            "if true then\n  /* block\n     comment */\n  let x = 1\nelse\n  let x = 2\nendif";
        assert!(matches!(
            parse_input_for_repl(src),
            ParseState::Complete(ReplInput::Stmt(_))
        ));
    }

    #[test]
    fn parse_syntax_error_when_not_incomplete() {
        assert!(matches!(
            parse_input_for_repl("let = 1"),
            ParseState::Error(_)
        ));
    }
}
