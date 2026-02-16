use crate::ast::*;
use chumsky::prelude::*;

type P<T> = BoxedParser<'static, char, T, Simple<char>>;



const KEYWORDS: &[&str] = &[

    "let", "fn", "do", "endfn", "return", "if", "else", "endif", "match", "endmatch", "case",

    "task", "endtask", "conc", "endconc", "port", "endport", "perform", "type", "import", "from",

    "pub", "effect", "raise", "try", "catch", "endtry", "handler", "for", "endhandler", "enum",

    "borrow",

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
            text::keyword("float").to(Type::Float),
            text::keyword("bool").to(Type::Bool),
            text::keyword("str").to(Type::Str),
            text::keyword("unit").to(Type::Unit),
            text::keyword("ref")
                .padded()
                .ignore_then(t.clone().delimited_by(just('('), just(')')))
                .map(|inner| Type::Ref(Box::new(inner))),
            just('&')
                .padded()
                .ignore_then(t.clone())
                .map(|inner| Type::Borrow(Box::new(inner))),
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
            t.clone()
                .delimited_by(just('['), just(']'))
                .map(|inner| Type::List(Box::new(inner))),
            t.clone()
                .delimited_by(just("[|"), just("|]"))
                .map(|inner| Type::Array(Box::new(inner))),
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

        let arrow = ident()
            .then_ignore(just(':').padded())
            .then(t.clone())
            .map(|(n, t)| (n, t))
            .or(t.clone().map(|t| ("_".to_string(), t)))
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
    let digits = filter(|c: &char| c.is_ascii_digit())
        .repeated()
        .at_least(1)
        .collect::<String>();

    let number = just('-')
        .or_not()
        .then(digits.clone())
        .then(just('.').ignore_then(digits.clone()).or_not())
        .map(|((sign, int_part), frac_part)| {
            let sign_str = if sign.is_some() { "-" } else { "" };
            if let Some(frac) = frac_part {
                let s = format!("{}{}.{}", sign_str, int_part, frac);
                Literal::Float(s.parse::<f64>().unwrap())
            } else {
                let s = format!("{}{}", sign_str, int_part);
                Literal::Int(s.parse::<i64>().unwrap())
            }
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

    choice((number, bool_lit, unit_lit, str_lit)).padded()
}

fn expr_parser() -> P<Spanned<Expr>> {
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

        let list = expr
            .clone()
            .separated_by(just(',').padded())
            .allow_trailing()
            .delimited_by(just('['), just(']'))
            .map(Expr::List);

        let array = expr
            .clone()
            .separated_by(just(',').padded())
            .allow_trailing()
            .delimited_by(just("[|"), just("|]"))
            .map(Expr::Array);

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

        let borrow_expr = text::keyword("borrow")
            .padded()
            .ignore_then(sigil())
            .then(ident())
            .map(|(s, n)| Expr::Borrow(n, s));

        let atom = choice((
            expr.clone()
                .delimited_by(just('('), just(')'))
                .map(|s| s.node),
            raise,
            borrow_expr,
            perform_call,
            constructor,
            simple_call,
            record,
            array,
            list,
            literal().map(Expr::Literal),
            var,
        ))
        .padded()
        .map_with_span(|node, span| Spanned { node, span });

        enum Postfix {
            Field(String, Span),
            Index(Spanned<Expr>),
        }

        // Postfix ops: .ident and [expr]
        let atom_with_postfix = atom
            .clone()
            .then(
                choice((
                    just('.')
                        .ignore_then(ident())
                        .map_with_span(|n, s| Postfix::Field(n, s)),
                    expr.clone()
                        .delimited_by(just('['), just(']'))
                        .map(Postfix::Index),
                ))
                .repeated(),
            )
            .foldl(|lhs, post| match post {
                Postfix::Field(name, name_span) => {
                    let span = lhs.span.start..name_span.end;
                    let node = Expr::FieldAccess(Box::new(lhs), name);
                    Spanned { node, span }
                }
                Postfix::Index(index) => {
                    let span = lhs.span.start..index.span.end;
                    let node = Expr::Index(Box::new(lhs), Box::new(index));
                    Spanned { node, span }
                }
            });

        let op = choice((
            // Float operators (must come before int operators to handle overlap)
            just("==.").to("==.".to_string()),
            just("!=.").to("!=.".to_string()),
            just("<=.").to("<=.".to_string()),
            just(">=.").to(">=.".to_string()),
            just("<.").to("<.".to_string()),
            just(">.").to(">.".to_string()),
            just("+.").to("+.".to_string()),
            just("-.").to("-.".to_string()),
            just("*.").to("*.".to_string()),
            just("/.").to("/.".to_string()),

            // Int/Generic operators
            just("==").to("==".to_string()),
            just("!=").to("!=".to_string()),
            just("<=").to("<=".to_string()),
            just(">=").to(">=".to_string()),
            just("<").to("<".to_string()),
            just(">").to(">".to_string()),
            just("++").to("++".to_string()),
            just("+").to("+".to_string()),
            just("-").to("-".to_string()),
            just("*").to("*".to_string()),
            just("/").to("/".to_string()),
        ))
        .padded();

        atom_with_postfix
            .clone()
            .then(op.then(atom_with_postfix).repeated())
            .foldl(|lhs, (op, rhs)| {
                let span = lhs.span.start..rhs.span.end;
                let node = Expr::BinaryOp(Box::new(lhs), op, Box::new(rhs));
                Spanned { node, span }
            })
    })
    .boxed()
}

pub fn stmt_parser() -> impl Parser<char, Spanned<Stmt>, Error = Simple<char>> {
    recursive(|stmt| {
        let expr = expr_parser();

        let let_stmt = text::keyword("let")
            .padded()
            .ignore_then(sigil())
            .then(ident())
            .then(just(':').padded().ignore_then(type_parser()).or_not())
            .then(just('=').padded().ignore_then(expr.clone()))
            .map_with_span(|(((s, n), t), v), span| Spanned {
                node: Stmt::Let {
                    name: n,
                    sigil: s,
                    typ: t,
                    value: v,
                },
                span,
            });

        let return_stmt = text::keyword("return")
            .padded()
            .ignore_then(expr.clone())
            .map_with_span(|v, span| Spanned {
                node: Stmt::Return(v),
                span,
            });

        let assign_stmt = expr
            .clone()
            .then_ignore(just("<-").padded())
            .then(expr.clone())
            .map_with_span(|(target, value), span| Spanned {
                node: Stmt::Assign { target, value },
                span,
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
            .map_with_span(|((cond, then_branch), else_branch), span| Spanned {
                node: Stmt::Expr(Spanned {
                    node: Expr::If {
                        cond: Box::new(cond),
                        then_branch,
                        else_branch,
                    },
                    span: span.clone(),
                }),
                span,
            });

        let pattern = recursive(|p: Recursive<'_, char, Spanned<Pattern>, Simple<char>>| {
            let variable = sigil().then(ident()).map_with_span(|(s, n), span| Spanned {
                node: Pattern::Variable(n, s),
                span,
            });
            let lit = literal().map_with_span(|l, span| Spanned {
                node: Pattern::Literal(l),
                span,
            });
            let wildcard = just('_').padded().map_with_span(|_, span| Spanned {
                node: Pattern::Wildcard,
                span,
            });

            let constructor = ident()
                .then(
                    p.clone()
                        .separated_by(just(',').padded())
                        .delimited_by(just('('), just(')')),
                )
                .map_with_span(|(c, args), span| Spanned {
                    node: Pattern::Constructor(c, args),
                    span,
                });

            let record_pat = choice((
                just('_').padded().to(None),
                ident()
                    .then_ignore(just(':').padded())
                    .then(p.clone())
                    .map(Some),
            ))
            .separated_by(just(',').padded())
            .allow_trailing()
            .delimited_by(just('{'), just('}'))
            .try_map(|entries, span| {
                let mut fields = Vec::new();
                let mut open = false;
                for e in entries {
                    match e {
                        Some(f) => {
                            if open {
                                return Err(Simple::custom(span, "_ must be the last element"));
                            }
                            fields.push(f);
                        }
                        None => {
                            if open {
                                return Err(Simple::custom(span, "duplicate _"));
                            }
                            open = true;
                        }
                    }
                }
                Ok(Spanned {
                    node: Pattern::Record(fields, open),
                    span,
                })
            });

            choice((constructor, record_pat, lit, wildcard, variable))
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
            .map_with_span(|(target, cases), span| Spanned {
                node: Stmt::Expr(Spanned {
                    node: Expr::Match {
                        target: Box::new(target),
                        cases,
                    },
                    span: span.clone(),
                }),
                span,
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
                        effects: Type::Row(vec![], None),
                        body,
                        type_params: vec![],
                    })
                    .repeated(),
            )
            .then_ignore(text::keyword("endconc").padded())
            .map_with_span(|(_, tasks), span| Spanned {
                node: Stmt::Conc(tasks),
                span,
            });

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
            .map_with_span(|(body, (catch_param, catch_body)), span| Spanned {
                node: Stmt::Try {
                    body,
                    catch_param,
                    catch_body,
                },
                span,
            });

        let comment = just("//")
            .then(take_until(choice((just('\n'), end().to('\n')))))
            .padded()
            .map_with_span(|_, span| Spanned {
                node: Stmt::Comment,
                span,
            });

        let basic_stmt = choice((
            comment.boxed(),
            let_stmt.boxed(),
            return_stmt.boxed(),
            assign_stmt.boxed(),
        ));

        let complex_stmt = choice((
            if_stmt.boxed(),
            match_stmt.boxed(),
            try_stmt.boxed(),
            conc_block.boxed(),
        ));

        basic_stmt
            .or(complex_stmt)
            .or(expr.map(|v| {
                let span = v.span.clone();
                Spanned {
                    node: Stmt::Expr(v),
                    span,
                }
            }).boxed())
            .padded()            })
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
        
            let enum_def = text::keyword("enum")
                .padded()
                .ignore_then(ident())
                .then(
                    just('<')
                        .ignore_then(ident().separated_by(just(',').padded()))
                        .then_ignore(just('>'))
                        .or_not(),
                )
                .then_ignore(just('{').padded())
                .then(
                    ident()
                        .then(
                            type_parser()
                                .separated_by(just(',').padded())
                                .delimited_by(just('('), just(')'))
                                .or_not(),
                        )
                        .map(|(name, fields)| VariantDef {
                            name,
                            fields: fields.unwrap_or_default(),
                        })
                        .separated_by(just(',').padded())
                        .allow_trailing(),
                )
                .then_ignore(just('}').padded())
                .map(|((name, type_params), variants)| {
                    TopLevel::Enum(EnumDef {
                        name,
                        type_params: type_params.unwrap_or_default(),
                        variants,
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
                        .map(|(((name, params), ret_type), effects)| FunctionSignature {
                            name,
                            params,
                            ret_type,
                            effects: effects.unwrap_or(Type::Row(vec![], None)),
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
                enum_def,
                import_def,
                port_def,
                handler_def,
                comment,
            ))
            .padded()
            .map_with_span(|node, span| Spanned { node, span })
            .repeated()
            .map(|definitions| Program { definitions })
            .then_ignore(end())
        }
