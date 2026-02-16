use crate::ast::*;
use chumsky::prelude::*;

type P<T> = BoxedParser<'static, char, T, Simple<char>>;



const KEYWORDS: &[&str] = &[

    "let", "fn", "do", "endfn", "return", "if", "else", "endif", "match", "endmatch", "case",

    "task", "endtask", "conc", "endconc", "port", "endport", "perform", "type", "import", "from",

    "pub", "effect", "raise", "try", "catch", "endtry", "handler", "for", "endhandler",

];



fn ident() -> impl Parser<char, String, Error = Simple<char>> + Clone {

    text::ident().padded().try_map(|s: String, span| {

        if KEYWORDS.contains(&s.as_str()) {

            Err(Simple::custom(span, format!("Keyword '{}' is reserved", s)))

        } else {

            Ok(s)

        }

    })

}





fn sigil() -> impl Parser<char, Sigil, Error = Simple<char>> + Clone {
    choice((just('~').to(Sigil::Mutable), just('%').to(Sigil::Linear)))
        .or(empty().to(Sigil::Immutable))
}

fn type_parser() -> P<Type> {
    recursive(|t: Recursive<'_, char, Type, Simple<char>>| {
        let base = choice((
            text::keyword("i64").to(Type::I64),
            text::keyword("bool").to(Type::Bool),
            text::keyword("str").to(Type::Str),
            text::keyword("unit").to(Type::Unit),
            text::keyword("ref")
                .padded()
                .ignore_then(t.clone().delimited_by(just('('), just(')')))
                .map(|inner| Type::Ref(Box::new(inner))),
            just('%')
                .padded()
                .ignore_then(t.clone())
                .map(|inner| Type::Linear(Box::new(inner))),
            ident()
                .then_ignore(just(':').padded())
                .then(t.clone())
                .separated_by(just(',').padded())
                .delimited_by(just('{'), just('}'))
                .map(Type::Record),
            ident().map(|n| Type::UserDefined(n, vec![])),
        ));

        let generic = ident()
            .then(
                t.clone()
                    .separated_by(just(',').padded())
                    .delimited_by(just('<'), just('>')),
            )
            .map(|(base, args)| {
                if base == "Result" && args.len() == 2 {
                    Type::Result(Box::new(args[0].clone()), Box::new(args[1].clone()))
                } else {
                    Type::UserDefined(base, args)
                }
            });

        let arrow = t
            .clone()
            .separated_by(just(',').padded())
            .delimited_by(just('('), just(')'))
            .then_ignore(just("->").padded())
            .then(t.clone())
            .then(
                text::keyword("effect")
                    .padded()
                    .ignore_then(choice((
                        t.clone()
                            .separated_by(just(',').padded())
                            .then(
                                just('|')
                                    .padded()
                                    .ignore_then(t.clone())
                                    .or_not(),
                            )
                            .delimited_by(just('{'), just('}'))
                            .map(|(effs, tail)| Type::Row(effs, tail.map(Box::new))),
                        t.clone(),
                    )))
                    .or_not(),
            )
            .map(|((params, ret), effects)| {
                Type::Arrow(
                    params,
                    Box::new(ret),
                    Box::new(effects.unwrap_or(Type::Row(vec![], None))),
                )
            });

        arrow.or(generic).or(base).padded()
    })
    .boxed()
}

fn literal() -> impl Parser<char, Literal, Error = Simple<char>> + Clone {
    let int = just('-').or_not().then(text::int(10)).map(|(sign, s)| {
        let val = s.parse::<i64>().unwrap();
        Literal::Int(if sign.is_some() { -val } else { val })
    });

    let bool_lit = choice((
        text::keyword("true").to(true),
        text::keyword("false").to(false),
    ))
    .map(Literal::Bool);

    let unit_lit = just("()").to(Literal::Unit);
    let str_lit = just('"')
        .ignore_then(filter(|c| *c != '"').repeated())
        .then_ignore(just('"'))
        .collect::<String>()
        .map(Literal::String);

    choice((int, bool_lit, unit_lit, str_lit)).padded()
}

fn expr_parser() -> P<Expr> {
    recursive(|expr| {
        let path = ident()
            .separated_by(just('.'))
            .at_least(1)
            .map(|v| v.join("."));

        let call_arg = ident().then_ignore(just(':').padded()).then(expr.clone());
        let call_args = call_arg
            .separated_by(just(','))
            .delimited_by(just('('), just(')'));

        let perform_call = text::keyword("perform")
            .padded()
            .ignore_then(path.clone())
            .then(call_args.clone())
            .map(|(func, args)| Expr::Call {
                func,
                args,
                perform: true,
            });

        let simple_call = path.clone().then(call_args).map(|(func, args)| Expr::Call {
            func,
            args,
            perform: false,
        });



        let record_field = ident().then_ignore(just(':').padded()).then(expr.clone());
        let record = record_field
            .separated_by(just(','))
            .delimited_by(just('{'), just('}'))
            .map(Expr::Record);

        let var = sigil().then(ident()).map(|(s, n)| Expr::Variable(n, s));

        let constructor = ident()
            .then(
                expr.clone()
                    .separated_by(just(','))
                    .delimited_by(just('('), just(')')),
            )
            .map(|(name, args)| Expr::Constructor(name, args));

        let raise = text::keyword("raise")
            .padded()
            .ignore_then(expr.clone())
            .map(|e| Expr::Raise(Box::new(e)));

        let atom = choice((
            raise,
            perform_call,
            constructor,
            simple_call,
            record,
            literal().map(Expr::Literal),
            var,
        ))
        .padded();

        // Field Access: atom.ident
        let atom_with_access = atom
            .clone()
            .then(just('.').ignore_then(ident()).repeated())
            .foldl(|lhs, name| Expr::FieldAccess(Box::new(lhs), name));

        let op = choice((
            just("==").to("==".to_string()),
            just("!=").to("!=".to_string()),
            just("<=").to("<=".to_string()),
            just(">=").to(">=".to_string()),
            just("<").to("<".to_string()),
            just(">").to(">".to_string()),
            just("+").to("+".to_string()),
            just("-").to("-".to_string()),
            just("*").to("*".to_string()),
            just("/").to("/".to_string()),
        ))
        .padded();

        atom_with_access
            .clone()
            .then(op.then(atom_with_access).repeated())
            .foldl(|lhs, (op, rhs)| Expr::BinaryOp(Box::new(lhs), op, Box::new(rhs)))
    })
    .boxed()
}

pub fn stmt_parser() -> impl Parser<char, Stmt, Error = Simple<char>> {
    recursive(|stmt| {
        let expr = expr_parser();

        let let_stmt = text::keyword("let")
            .padded()
            .ignore_then(sigil())
            .then(ident())
            .then(just('=').padded().ignore_then(expr.clone()))
            .map(|((s, n), v)| Stmt::Let {
                name: n,
                sigil: s,
                typ: None,
                value: v,
            });

        let return_stmt = text::keyword("return")
            .padded()
            .ignore_then(expr.clone())
            .map(Stmt::Return);

        let assign_stmt = sigil()
            .then(ident())
            .then_ignore(just("<-").padded())
            .then(expr.clone())
            .map(|((s, n), v)| Stmt::Assign {
                name: n,
                sigil: s,
                value: v,
            });

        let if_stmt = text::keyword("if")
            .padded()
            .ignore_then(expr.clone())
            .then_ignore(text::keyword("then").padded())
            .then(stmt.clone().repeated())
            .then(
                text::keyword("else")
                    .padded()
                    .ignore_then(stmt.clone().repeated())
                    .or_not(),
            )
            .then_ignore(text::keyword("endif").padded())
            .map(|((cond, then_branch), else_branch)| {
                Stmt::Expr(Expr::If {
                    cond: Box::new(cond),
                    then_branch,
                    else_branch,
                })
            });

        let pattern = recursive(|p: Recursive<'_, char, Pattern, Simple<char>>| {
            let variable = sigil().then(ident()).map(|(s, n)| Pattern::Variable(n, s));
            let lit = literal().map(Pattern::Literal);
            let wildcard = just('_').padded().to(Pattern::Wildcard);

            let constructor = ident()
                .then(
                    p.separated_by(just(',').padded())
                        .delimited_by(just('('), just(')')),
                )
                .map(|(c, args)| Pattern::Constructor(c, args));

            choice((constructor, lit, wildcard, variable))
        });

        let match_case = text::keyword("case")
            .padded()
            .ignore_then(pattern)
            .then_ignore(just("->").padded())
            .then(stmt.clone().repeated())
            .map(|(pattern, body)| MatchCase { pattern, body });

        let match_stmt = text::keyword("match")
            .padded()
            .ignore_then(expr.clone())
            .then_ignore(text::keyword("do").padded())
            .then(match_case.repeated())
            .then_ignore(text::keyword("endmatch").padded())
            .map(|(target, cases)| {
                Stmt::Expr(Expr::Match {
                    target: Box::new(target),
                    cases,
                })
            });

        let conc_block = text::keyword("conc")
            .padded()
            .ignore_then(text::keyword("do").padded())
            .then(
                text::keyword("task")
                    .padded()
                    .ignore_then(
                        just('"')
                            .ignore_then(take_until(just('"')))
                            .map(|(s, _)| s.into_iter().collect::<String>()),
                    )
                    .then_ignore(text::keyword("do").padded())
                    .then(stmt.clone().repeated())
                    .then_ignore(text::keyword("endtask").padded())
                    .map(|(name, body)| Function {
                        name,
                        is_public: false,
                        params: vec![],
                        ret_type: Type::Unit,
                        effects: Type::Unit,
                        body,
                        type_params: vec![],
                    })
                    .repeated(),
            )
            .then_ignore(text::keyword("endconc").padded())
            .map(|(_, tasks)| Stmt::Conc(tasks));

        let try_stmt = text::keyword("try")
            .padded()
            .ignore_then(stmt.clone().repeated())
            .then(
                text::keyword("catch")
                    .padded()
                    .ignore_then(ident())
                    .then_ignore(just("->").padded())
                    .then(stmt.clone().repeated()),
            )
            .then_ignore(text::keyword("endtry").padded())
            .map(|(body, (catch_param, catch_body))| Stmt::Try {
                body,
                catch_param,
                catch_body,
            });

        let comment = just("//")
            .then(take_until(choice((just('\n'), end().to('\n')))))
            .padded()
            .map(|_| Stmt::Comment);

        choice((
            comment,
            let_stmt,
            return_stmt,
            assign_stmt,
            if_stmt,
            match_stmt,
            try_stmt,
            conc_block,
            expr.map(Stmt::Expr),
        ))
        .padded()
    })
    .boxed()
}

pub fn parser() -> impl Parser<char, Program, Error = Simple<char>> {
    let param = sigil()
        .then(ident())
        .then_ignore(just(':').padded())
        .then(type_parser())
        .map(|((sigil, name), typ)| Param { name, sigil, typ });

    let function_inner = text::keyword("pub")
        .or_not()
        .padded()
        .then_ignore(text::keyword("fn").padded())
        .then(ident())
        .then(
            just('<')
                .ignore_then(ident().separated_by(just(',').padded()))
                .then_ignore(just('>'))
                .or_not(),
        )
        .then(
            param
                .clone()
                .separated_by(just(','))
                .delimited_by(just('('), just(')')),
        )
        .then_ignore(just("->").padded())
        .then(type_parser())
        .then(
            text::keyword("effect")
                .padded()
                .ignore_then(choice((
                    type_parser()
                        .separated_by(just(',').padded())
                        .then(
                            just('|')
                                .padded()
                                .ignore_then(type_parser())
                                .or_not(),
                        )
                        .delimited_by(just('{'), just('}'))
                        .map(|(effs, tail)| Type::Row(effs, tail.map(Box::new))),
                    type_parser(),
                )))
                .or_not(),
        )
        .then_ignore(text::keyword("do").padded())
        .then(stmt_parser().repeated())
        .then_ignore(text::keyword("endfn").padded())
        .map(
            |((((((vis, name), type_params), params), ret_type), effects), body)| Function {
                name,
                is_public: vis.is_some(),
                type_params: type_params.unwrap_or_default(),
                params,
                ret_type,
                effects: effects.unwrap_or(Type::Row(vec![], None)),
                body,
            },
        )
        .boxed();

    let func_def = function_inner.clone().map(TopLevel::Function);

    let type_def = text::keyword("type")
        .padded()
        .ignore_then(ident())
        .then(
            just('<')
                .ignore_then(ident().separated_by(just(',').padded()))
                .then_ignore(just('>'))
                .or_not(),
        )
        .then_ignore(just('=').padded())
        .then(
            ident()
                .then_ignore(just(':').padded())
                .then(type_parser())
                .separated_by(just(','))
                .delimited_by(just('{'), just('}')),
        )
        .map(|((name, type_params), fields)| {
            TopLevel::TypeDef(TypeDef {
                name,
                type_params: type_params.unwrap_or_default(),
                fields,
            })
        });

    let import_def = text::keyword("import")
        .padded()
        .ignore_then(
            ident()
                .separated_by(just(','))
                .delimited_by(just('{'), just('}')),
        )
        .then_ignore(text::keyword("from").padded())
        .then(
            just('"')
                .ignore_then(take_until(just('"')))
                .map(|(s, _)| s.into_iter().collect::<String>()),
        )
        .map(|(items, module)| TopLevel::Import(Import { module, items }));

    let port_def = text::keyword("port")
        .padded()
        .ignore_then(ident())
        .then_ignore(text::keyword("do").padded())
        .then(
            text::keyword("fn")
                .padded()
                .ignore_then(ident())
                .then(
                    param
                        .clone()
                        .separated_by(just(','))
                        .delimited_by(just('('), just(')')),
                )
                .then_ignore(just("->").padded())
                .then(type_parser())
                .map(|((name, params), ret_type)| FunctionSignature {
                    name,
                    params,
                    ret_type,
                    effects: Type::Row(vec![], None),
                })
                .repeated(),
        )
        .then_ignore(text::keyword("endport").padded())
        .map(|(name, functions)| TopLevel::Port(Port { name, functions }));

    let handler_def = text::keyword("handler")
        .padded()
        .ignore_then(ident())
        .then_ignore(text::keyword("for").padded())
        .then(ident())
        .then_ignore(text::keyword("do").padded())
        .then(function_inner.repeated())
        .then_ignore(text::keyword("endhandler").padded())
        .map(|((name, port_name), functions)| {
            TopLevel::Handler(Handler {
                name,
                port_name,
                functions,
            })
        });

    let comment = just("//")
        .then(take_until(just('\n')))
        .padded()
        .map(|_| TopLevel::Comment);

    choice((
        func_def,
        type_def,
        import_def,
        port_def,
        handler_def,
        comment,
    ))
        .padded()
        .repeated()
        .map(|definitions| Program { definitions })
        .then_ignore(end())
}
