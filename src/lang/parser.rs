use super::ast::*;
use super::lexer::{self, Token, TokenKind};

const KEYWORDS: &[&str] = &[
    "let",
    "fn",
    "do",
    "end",
    "return",
    "if",
    "else",
    "match",
    "case",
    "task",
    "conc",
    "port",
    "type",
    "import",
    "from",
    "export",
    "require",
    "throws",
    "raise",
    "try",
    "catch",
    "handler",
    "inject",
    "exception",
    "external",
];

fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Parse error at {:?}: {}", self.span, self.message)
    }
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn peek_span(&self) -> Span {
        self.tokens[self.pos].span.clone()
    }

    fn at_end(&self) -> bool {
        matches!(self.peek(), TokenKind::Eof)
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        if !self.at_end() {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, kind: &TokenKind) -> Result<&Token, ParseError> {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(kind) {
            Ok(self.advance())
        } else {
            Err(ParseError {
                message: format!("expected {:?}, got {:?}", kind, self.peek()),
                span: self.peek_span(),
            })
        }
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.peek().clone() {
            TokenKind::Ident(s) => {
                self.advance();
                if is_keyword(&s) {
                    Err(ParseError {
                        message: format!("Keyword '{}' is reserved", s),
                        span: self.tokens[self.pos - 1].span.clone(),
                    })
                } else {
                    Ok(s)
                }
            }
            _ => Err(ParseError {
                message: format!("expected identifier, got {:?}", self.peek()),
                span: self.peek_span(),
            }),
        }
    }

    fn match_token(&mut self, kind: &TokenKind) -> bool {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn match_keyword(&mut self, kw: &TokenKind) -> bool {
        if self.peek() == kw {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Check for contextual keyword (token is Ident with specific text)
    fn is_contextual(&self, kw: &str) -> bool {
        matches!(self.peek(), TokenKind::Ident(ref s) if s == kw)
    }

    /// Match and consume a contextual keyword
    fn match_contextual(&mut self, kw: &str) -> bool {
        if self.is_contextual(kw) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Expect a contextual keyword
    fn expect_contextual(&mut self, kw: &str) -> Result<(), ParseError> {
        if self.match_contextual(kw) {
            Ok(())
        } else {
            Err(ParseError {
                message: format!("expected '{}', got {:?}", kw, self.peek()),
                span: self.peek_span(),
            })
        }
    }

    /// Parse optional type parameters: `<T, U, ...>`. Returns empty vec if no `<` found.
    fn parse_type_params(&mut self) -> Result<Vec<String>, ParseError> {
        if !matches!(self.peek(), TokenKind::Lt) {
            return Ok(vec![]);
        }
        self.advance();
        let mut params = vec![self.expect_ident()?];
        while self.match_token(&TokenKind::Comma) {
            params.push(self.expect_ident()?);
        }
        self.expect(&TokenKind::Gt)?;
        Ok(params)
    }

    /// Parse a comma-separated list of items between open/close delimiters.
    /// `parse_item` is called for each element.
    fn parse_delimited_list<T>(
        &mut self,
        open: &TokenKind,
        close: &TokenKind,
        mut parse_item: impl FnMut(&mut Self) -> Result<T, ParseError>,
    ) -> Result<Vec<T>, ParseError> {
        self.expect(open)?;
        let mut items = Vec::new();
        if std::mem::discriminant(self.peek()) != std::mem::discriminant(close) {
            items.push(parse_item(self)?);
            while self.match_token(&TokenKind::Comma) {
                if std::mem::discriminant(self.peek()) == std::mem::discriminant(close) {
                    break;
                }
                items.push(parse_item(self)?);
            }
        }
        self.expect(close)?;
        Ok(items)
    }

    /// Parse a comma-separated list without delimiters (no open/close tokens).
    /// Stops when `is_end` returns true.
    fn parse_comma_separated<T>(
        &mut self,
        is_end: impl Fn(&TokenKind) -> bool,
        mut parse_item: impl FnMut(&mut Self) -> Result<T, ParseError>,
    ) -> Result<Vec<T>, ParseError> {
        let mut items = Vec::new();
        if !is_end(self.peek()) {
            items.push(parse_item(self)?);
            while self.match_token(&TokenKind::Comma) {
                if is_end(self.peek()) {
                    break;
                }
                items.push(parse_item(self)?);
            }
        }
        Ok(items)
    }

    /// Try parsing `ident : value` (labeled), falling back to just `value` (unlabeled).
    fn parse_optional_labeled<T>(
        &mut self,
        mut parse_value: impl FnMut(&mut Self) -> Result<T, ParseError>,
    ) -> Result<(Option<String>, T), ParseError> {
        let saved = self.pos;
        if let Ok(name) = self.expect_ident() {
            if self.match_token(&TokenKind::Colon) {
                let val = parse_value(self)?;
                return Ok((Some(name), val));
            }
        }
        self.pos = saved;
        let val = parse_value(self)?;
        Ok((None, val))
    }

    fn is_uppercase_ident(s: &str) -> bool {
        s.chars().next().map_or(false, |c| c.is_ascii_uppercase())
    }

    // ---- Type Parsing ----

    fn parse_type(&mut self) -> Result<Type, ParseError> {
        // Try arrow type first: (params) -> ret [require ...] [throws ...]
        if matches!(self.peek(), TokenKind::LParen) {
            let saved = self.pos;
            let arrow_err = match self.try_parse_arrow_type() {
                Ok(t) => return Ok(t),
                Err(e) => e,
            };
            self.pos = saved;
            return self.parse_type_atom().map_err(|mut e| {
                e.message = format!(
                    "{} (also tried as arrow type: {})",
                    e.message, arrow_err.message
                );
                e
            });
        }
        self.parse_type_atom()
    }

    fn try_parse_arrow_type(&mut self) -> Result<Type, ParseError> {
        self.expect(&TokenKind::LParen)?;
        let params = self.parse_arrow_params()?;
        self.expect(&TokenKind::RParen)?;
        self.expect(&TokenKind::Arrow)?;
        let ret = self.parse_type()?;
        let requires = if self.match_keyword(&TokenKind::Require) {
            self.parse_row_or_type()?
        } else {
            Type::Row(vec![], None)
        };
        let throws = if self.match_keyword(&TokenKind::Throws) {
            self.parse_row_or_type()?
        } else {
            Type::Row(vec![], None)
        };
        Ok(Type::Arrow(
            params,
            Box::new(ret),
            Box::new(requires),
            Box::new(throws),
        ))
    }

    fn parse_arrow_params(&mut self) -> Result<Vec<(String, Type)>, ParseError> {
        self.parse_comma_separated(|t| matches!(t, TokenKind::RParen), Self::parse_arrow_param)
    }

    fn parse_arrow_param(&mut self) -> Result<(String, Type), ParseError> {
        // Try named: ident : type
        let saved = self.pos;
        if let Ok(name) = self.expect_ident() {
            if self.match_token(&TokenKind::Colon) {
                let typ = self.parse_type()?;
                return Ok((name, typ));
            }
        }
        // Fallback: just a type, use "_" as name
        self.pos = saved;
        let typ = self.parse_type()?;
        Ok(("_".to_string(), typ))
    }

    fn parse_row_or_type(&mut self) -> Result<Type, ParseError> {
        if matches!(self.peek(), TokenKind::LBrace) {
            self.parse_row_type()
        } else {
            self.parse_type()
        }
    }

    fn parse_row_type(&mut self) -> Result<Type, ParseError> {
        self.expect(&TokenKind::LBrace)?;
        let mut items = Vec::new();
        if !matches!(self.peek(), TokenKind::RBrace) {
            items.push(self.parse_type()?);
            while self.match_token(&TokenKind::Comma) {
                if matches!(self.peek(), TokenKind::RBrace) {
                    break;
                }
                items.push(self.parse_type()?);
            }
        }
        let tail = if self.match_token(&TokenKind::Pipe) {
            Some(Box::new(self.parse_type()?))
        } else {
            None
        };
        self.expect(&TokenKind::RBrace)?;
        Ok(Type::Row(items, tail))
    }

    fn parse_type_atom(&mut self) -> Result<Type, ParseError> {
        match self.peek().clone() {
            TokenKind::Ident(ref s) => {
                let s = s.clone();
                match s.as_str() {
                    "i32" => {
                        self.advance();
                        Ok(Type::I32)
                    }
                    "i64" => {
                        self.advance();
                        Ok(Type::I64)
                    }
                    "f32" => {
                        self.advance();
                        Ok(Type::F32)
                    }
                    "f64" => {
                        self.advance();
                        Ok(Type::F64)
                    }
                    "float" => {
                        self.advance();
                        Ok(Type::F64)
                    }
                    "bool" => {
                        self.advance();
                        Ok(Type::Bool)
                    }
                    "char" => {
                        self.advance();
                        Ok(Type::Char)
                    }
                    "string" => {
                        self.advance();
                        Ok(Type::String)
                    }
                    "unit" => {
                        self.advance();
                        Ok(Type::Unit)
                    }
                    "ref" => {
                        self.advance();
                        self.expect(&TokenKind::LParen)?;
                        let inner = self.parse_type()?;
                        self.expect(&TokenKind::RParen)?;
                        Ok(Type::Ref(Box::new(inner)))
                    }
                    _ => {
                        // UserDefined name, possibly with generic args
                        let name = self.expect_ident()?;
                        if matches!(self.peek(), TokenKind::Lt) {
                            let args = self.parse_generic_args()?;
                            Ok(Type::UserDefined(name, args))
                        } else {
                            Ok(Type::UserDefined(name, vec![]))
                        }
                    }
                }
            }
            TokenKind::Handler => {
                self.advance();
                let name = self.expect_ident()?;
                Ok(Type::Handler(name, Box::new(Type::Row(vec![], None))))
            }
            TokenKind::Ampersand => {
                self.advance();
                let inner = self.parse_type()?;
                Ok(Type::Borrow(Box::new(inner)))
            }
            TokenKind::Percent => {
                self.advance();
                let inner = self.parse_type()?;
                Ok(Type::Linear(Box::new(inner)))
            }
            TokenKind::LBrace => {
                // Record type or row type
                // Try record: { name: type, ... }
                let saved = self.pos;
                let record_err = match self.try_parse_record_type() {
                    Ok(record) => return Ok(record),
                    Err(e) => e,
                };
                self.pos = saved;
                // row type, with record error context if this also fails
                self.parse_row_type().map_err(|mut e| {
                    e.message = format!(
                        "{} (also tried as record type: {})",
                        e.message, record_err.message
                    );
                    e
                })
            }
            TokenKind::LBracket => {
                // List type: [T]
                self.advance();
                let inner = self.parse_type()?;
                self.expect(&TokenKind::RBracket)?;
                Ok(Type::List(Box::new(inner)))
            }
            TokenKind::LBracketPipe => {
                // Array type: [| T |]
                self.advance();
                let inner = self.parse_type()?;
                self.expect(&TokenKind::RBracketPipe)?;
                Ok(Type::Array(Box::new(inner)))
            }
            _ => Err(ParseError {
                message: format!("expected type, got {:?}", self.peek()),
                span: self.peek_span(),
            }),
        }
    }

    fn try_parse_record_type(&mut self) -> Result<Type, ParseError> {
        let fields = self.parse_delimited_list(
            &TokenKind::LBrace,
            &TokenKind::RBrace,
            Self::parse_named_field,
        )?;
        Ok(Type::Record(fields))
    }

    fn parse_generic_args(&mut self) -> Result<Vec<Type>, ParseError> {
        self.parse_delimited_list(&TokenKind::Lt, &TokenKind::Gt, Self::parse_type)
    }

    // ---- Sigil parsing ----

    fn parse_sigil(&mut self) -> Sigil {
        match self.peek() {
            TokenKind::Tilde => {
                self.advance();
                Sigil::Mutable
            }
            TokenKind::Percent => {
                self.advance();
                Sigil::Linear
            }
            TokenKind::Ampersand => {
                self.advance();
                Sigil::Borrow
            }
            _ => Sigil::Immutable,
        }
    }

    // ---- Literal parsing ----

    fn parse_literal(&mut self) -> Result<Literal, ParseError> {
        match self.peek().clone() {
            TokenKind::Int(n) => {
                self.advance();
                Ok(Literal::Int(n))
            }
            TokenKind::Float(n) => {
                self.advance();
                Ok(Literal::Float(n))
            }
            TokenKind::Minus => {
                // Negative number literal: - followed by Int or Float
                match self.peek_at_offset(1) {
                    Some(TokenKind::Int(_) | TokenKind::Float(_)) => {
                        self.advance(); // consume minus
                        match self.peek().clone() {
                            TokenKind::Int(n) => {
                                self.advance();
                                Ok(Literal::Int(-n))
                            }
                            TokenKind::Float(n) => {
                                self.advance();
                                Ok(Literal::Float(-n))
                            }
                            _ => unreachable!(),
                        }
                    }
                    _ => Err(ParseError {
                        message: format!("expected literal, got {:?}", self.peek()),
                        span: self.peek_span(),
                    }),
                }
            }
            TokenKind::True => {
                self.advance();
                Ok(Literal::Bool(true))
            }
            TokenKind::False => {
                self.advance();
                Ok(Literal::Bool(false))
            }
            TokenKind::CharLit(c) => {
                self.advance();
                Ok(Literal::Char(c))
            }
            TokenKind::StringLit(ref s) => {
                let s = s.clone();
                self.advance();
                Ok(Literal::String(s))
            }
            _ => Err(ParseError {
                message: format!("expected literal, got {:?}", self.peek()),
                span: self.peek_span(),
            }),
        }
    }

    // ---- Pattern parsing ----

    fn parse_pattern(&mut self) -> Result<Spanned<Pattern>, ParseError> {
        let lhs = self.parse_atom_pattern()?;

        // :: (cons) pattern — right-associative, desugars to Cons(v: lhs, rest: rhs)
        if matches!(self.peek(), TokenKind::ColonColon) {
            self.advance();
            let rhs = self.parse_pattern()?; // recursive → right-assoc
            let span = lhs.span.start..rhs.span.end;
            Ok(Spanned {
                node: Pattern::Constructor(
                    "Cons".to_string(),
                    vec![
                        (Some("v".to_string()), lhs),
                        (Some("rest".to_string()), rhs),
                    ],
                ),
                span,
            })
        } else {
            Ok(lhs)
        }
    }

    fn parse_atom_pattern(&mut self) -> Result<Spanned<Pattern>, ParseError> {
        let start = self.peek_span().start;

        match self.peek().clone() {
            TokenKind::LBrace => {
                // Record pattern
                self.advance();
                let mut fields = Vec::new();
                let mut open = false;
                if !matches!(self.peek(), TokenKind::RBrace) {
                    loop {
                        if matches!(self.peek(), TokenKind::Ident(ref s) if s == "_") {
                            self.advance();
                            if open {
                                return Err(ParseError {
                                    message: "duplicate _".to_string(),
                                    span: start..self.peek_span().end,
                                });
                            }
                            open = true;
                        } else {
                            if open {
                                return Err(ParseError {
                                    message: "_ must be the last element".to_string(),
                                    span: start..self.peek_span().end,
                                });
                            }
                            let name = self.expect_ident()?;
                            self.expect(&TokenKind::Colon)?;
                            let pat = self.parse_pattern()?;
                            fields.push((name, pat));
                        }
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                        if matches!(self.peek(), TokenKind::RBrace) {
                            break;
                        }
                    }
                }
                let end = self.peek_span().end;
                self.expect(&TokenKind::RBrace)?;
                Ok(Spanned {
                    node: Pattern::Record(fields, open),
                    span: start..end,
                })
            }
            TokenKind::Ident(ref s) if s == "_" => {
                self.advance();
                let end = self.tokens[self.pos - 1].span.end;
                Ok(Spanned {
                    node: Pattern::Wildcard,
                    span: start..end,
                })
            }
            TokenKind::Ident(ref s) if Self::is_uppercase_ident(s) => {
                // Constructor pattern
                let name = s.clone();
                self.advance();
                if matches!(self.peek(), TokenKind::LParen) {
                    self.advance();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), TokenKind::RParen) {
                        args.push(self.parse_ctor_pat_arg()?);
                        while self.match_token(&TokenKind::Comma) {
                            args.push(self.parse_ctor_pat_arg()?);
                        }
                    }
                    let end = self.peek_span().end;
                    self.expect(&TokenKind::RParen)?;
                    Ok(Spanned {
                        node: Pattern::Constructor(name, args),
                        span: start..end,
                    })
                } else {
                    let end = self.tokens[self.pos - 1].span.end;
                    Ok(Spanned {
                        node: Pattern::Constructor(name, vec![]),
                        span: start..end,
                    })
                }
            }
            // [p1, p2, ...]: list pattern — desugars to nested Cons/Nil
            TokenKind::LBracket => {
                self.advance(); // [
                if matches!(self.peek(), TokenKind::RBracket) {
                    let end = self.peek_span().end;
                    self.advance(); // ]
                    Ok(Spanned {
                        node: Pattern::Constructor("Nil".to_string(), vec![]),
                        span: start..end,
                    })
                } else {
                    let mut pats = vec![self.parse_pattern()?];
                    while self.match_token(&TokenKind::Comma) {
                        if matches!(self.peek(), TokenKind::RBracket) {
                            break;
                        }
                        pats.push(self.parse_pattern()?);
                    }
                    let end = self.peek_span().end;
                    self.expect(&TokenKind::RBracket)?;
                    // Desugar [p1, p2, ...] → Cons(v: p1, rest: Cons(v: p2, rest: ... Nil))
                    let mut result = Spanned {
                        node: Pattern::Constructor("Nil".to_string(), vec![]),
                        span: start..end,
                    };
                    for pat in pats.into_iter().rev() {
                        result = Spanned {
                            node: Pattern::Constructor(
                                "Cons".to_string(),
                                vec![
                                    (Some("v".to_string()), pat),
                                    (Some("rest".to_string()), result),
                                ],
                            ),
                            span: start..end,
                        };
                    }
                    Ok(result)
                }
            }
            _ => {
                // Try literal
                let saved = self.pos;
                if let Ok(lit) = self.parse_literal() {
                    let end = self.tokens[self.pos - 1].span.end;
                    return Ok(Spanned {
                        node: Pattern::Literal(lit),
                        span: start..end,
                    });
                }
                self.pos = saved;

                // Variable pattern (with sigil)
                let sigil = self.parse_sigil();
                let name = self.expect_ident()?;
                let end = self.tokens[self.pos - 1].span.end;
                Ok(Spanned {
                    node: Pattern::Variable(name, sigil),
                    span: start..end,
                })
            }
        }
    }

    fn parse_ctor_pat_arg(&mut self) -> Result<(Option<String>, Spanned<Pattern>), ParseError> {
        self.parse_optional_labeled(Self::parse_pattern)
    }

    // ---- Param parsing ----

    fn parse_param(&mut self) -> Result<Param, ParseError> {
        let sigil = self.parse_sigil();
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Colon)?;
        let typ = self.parse_type()?;
        Ok(Param { name, sigil, typ })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, ParseError> {
        self.parse_delimited_list(&TokenKind::LParen, &TokenKind::RParen, Self::parse_param)
    }

    // ---- Require/Effect parsing (shared helper) ----

    fn parse_require_clause(&mut self) -> Result<Type, ParseError> {
        if self.match_keyword(&TokenKind::Require) {
            self.parse_row_or_type()
        } else {
            Ok(Type::Row(vec![], None))
        }
    }

    fn parse_throws_clause(&mut self) -> Result<Type, ParseError> {
        if self.match_keyword(&TokenKind::Throws) {
            self.parse_row_or_type()
        } else {
            Ok(Type::Row(vec![], None))
        }
    }

    // ---- Expression parsing ----

    fn parse_expr(&mut self) -> Result<Spanned<Expr>, ParseError> {
        self.parse_binary_expr()
    }

    fn parse_binary_expr(&mut self) -> Result<Spanned<Expr>, ParseError> {
        self.parse_prec_expr(0)
    }

    fn token_to_binop(tok: &TokenKind) -> Option<BinaryOp> {
        match tok {
            TokenKind::OrOr => Some(BinaryOp::Or),
            TokenKind::AndAnd => Some(BinaryOp::And),
            TokenKind::EqEq => Some(BinaryOp::Eq),
            TokenKind::Ne => Some(BinaryOp::Ne),
            TokenKind::Le => Some(BinaryOp::Le),
            TokenKind::Ge => Some(BinaryOp::Ge),
            TokenKind::Lt => Some(BinaryOp::Lt),
            TokenKind::Gt => Some(BinaryOp::Gt),
            TokenKind::EqDot => Some(BinaryOp::FEq),
            TokenKind::NeDot => Some(BinaryOp::FNe),
            TokenKind::LeDot => Some(BinaryOp::FLe),
            TokenKind::GeDot => Some(BinaryOp::FGe),
            TokenKind::LtDot => Some(BinaryOp::FLt),
            TokenKind::GtDot => Some(BinaryOp::FGt),
            TokenKind::Plus => Some(BinaryOp::Add),
            TokenKind::Minus => Some(BinaryOp::Sub),
            TokenKind::PlusPlus => Some(BinaryOp::Concat),
            TokenKind::PlusDot => Some(BinaryOp::FAdd),
            TokenKind::MinusDot => Some(BinaryOp::FSub),
            TokenKind::Star => Some(BinaryOp::Mul),
            TokenKind::Slash => Some(BinaryOp::Div),
            TokenKind::Percent => Some(BinaryOp::Mod),
            TokenKind::StarDot => Some(BinaryOp::FMul),
            TokenKind::SlashDot => Some(BinaryOp::FDiv),
            TokenKind::Ampersand => Some(BinaryOp::BitAnd),
            TokenKind::Pipe => Some(BinaryOp::BitOr),
            TokenKind::Caret => Some(BinaryOp::BitXor),
            TokenKind::Shl => Some(BinaryOp::Shl),
            TokenKind::Shr => Some(BinaryOp::Shr),
            _ => None,
        }
    }

    fn binop_precedence(op: &BinaryOp) -> u8 {
        match op {
            BinaryOp::Or => 1,
            BinaryOp::And => 2,
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
            | BinaryOp::FEq
            | BinaryOp::FNe
            | BinaryOp::FLt
            | BinaryOp::FLe
            | BinaryOp::FGt
            | BinaryOp::FGe => 3,
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Concat
            | BinaryOp::FAdd
            | BinaryOp::FSub
            | BinaryOp::BitOr
            | BinaryOp::BitXor => 4,
            BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::FMul
            | BinaryOp::FDiv
            | BinaryOp::BitAnd => 5,
            BinaryOp::Shl | BinaryOp::Shr => 6,
        }
    }

    /// Precedence for `::` (cons) — same level as additive operators.
    const CONS_PREC: u8 = 4;

    fn parse_prec_expr(&mut self, min_prec: u8) -> Result<Spanned<Expr>, ParseError> {
        let mut lhs = self.parse_postfix_expr()?;

        loop {
            // :: (cons) — right-associative, desugars to Cons(v: lhs, rest: rhs)
            if matches!(self.peek(), TokenKind::ColonColon) && Self::CONS_PREC >= min_prec {
                self.advance();
                let rhs = self.parse_prec_expr(Self::CONS_PREC)?; // same prec → right-assoc
                let span = lhs.span.start..rhs.span.end;
                lhs = Spanned {
                    node: Expr::Constructor(
                        "Cons".to_string(),
                        vec![
                            (Some("v".to_string()), lhs),
                            (Some("rest".to_string()), rhs),
                        ],
                    ),
                    span,
                };
                continue;
            }

            let op = match Self::token_to_binop(&self.peek()) {
                Some(op) if Self::binop_precedence(&op) >= min_prec => op,
                _ => break,
            };
            let prec = Self::binop_precedence(&op);
            self.advance();
            let rhs = self.parse_prec_expr(prec + 1)?;
            let span = lhs.span.start..rhs.span.end;
            lhs = Spanned {
                node: Expr::BinaryOp(Box::new(lhs), op, Box::new(rhs)),
                span,
            };
        }

        Ok(lhs)
    }

    fn parse_postfix_expr(&mut self) -> Result<Spanned<Expr>, ParseError> {
        let mut expr = self.parse_atom()?;

        loop {
            match self.peek() {
                TokenKind::Dot => {
                    self.advance();
                    let name = self.expect_ident()?;
                    let end = self.tokens[self.pos - 1].span.end;
                    let span = expr.span.start..end;
                    expr = Spanned {
                        node: Expr::FieldAccess(Box::new(expr), name),
                        span,
                    };
                }
                TokenKind::LBracket => {
                    self.advance();
                    let index = self.parse_expr()?;
                    let end = self.peek_span().end;
                    self.expect(&TokenKind::RBracket)?;
                    let span = expr.span.start..end;
                    expr = Spanned {
                        node: Expr::Index(Box::new(expr), Box::new(index)),
                        span,
                    };
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    fn parse_atom(&mut self) -> Result<Spanned<Expr>, ParseError> {
        let start = self.peek_span().start;

        match self.peek().clone() {
            // Parenthesized expression or unit literal ()
            TokenKind::LParen => {
                self.advance();
                // Check for unit literal ()
                if matches!(self.peek(), TokenKind::RParen) {
                    self.advance();
                    let end = self.tokens[self.pos - 1].span.end;
                    return Ok(Spanned {
                        node: Expr::Literal(Literal::Unit),
                        span: start..end,
                    });
                }
                let inner = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                let end = self.tokens[self.pos - 1].span.end;
                Ok(Spanned {
                    node: inner.node,
                    span: start..end,
                })
            }

            // raise expr
            TokenKind::Raise => {
                self.advance();
                let expr = self.parse_expr()?;
                let end = expr.span.end;
                Ok(Spanned {
                    node: Expr::Raise(Box::new(expr)),
                    span: start..end,
                })
            }

            // &sigil ident (borrow expression)
            TokenKind::Ampersand => {
                self.advance();
                let sigil = self.parse_sigil();
                let name = self.expect_ident()?;
                let end = self.tokens[self.pos - 1].span.end;
                Ok(Spanned {
                    node: Expr::Borrow(name, sigil),
                    span: start..end,
                })
            }

            // fn ... (lambda)
            TokenKind::Fn => self.parse_lambda(start),

            // handler Port do ... end
            TokenKind::Handler => self.parse_handler_expr(start),

            // Array literal [| ... |]
            TokenKind::LBracketPipe => {
                self.advance();
                let mut items = Vec::new();
                if !matches!(self.peek(), TokenKind::RBracketPipe) {
                    items.push(self.parse_expr()?);
                    while self.match_token(&TokenKind::Comma) {
                        if matches!(self.peek(), TokenKind::RBracketPipe) {
                            break;
                        }
                        items.push(self.parse_expr()?);
                    }
                }
                let end = self.peek_span().end;
                self.expect(&TokenKind::RBracketPipe)?;
                Ok(Spanned {
                    node: Expr::Array(items),
                    span: start..end,
                })
            }

            // List literal [...]
            TokenKind::LBracket => {
                self.advance();
                let mut items = Vec::new();
                if !matches!(self.peek(), TokenKind::RBracket) {
                    items.push(self.parse_expr()?);
                    while self.match_token(&TokenKind::Comma) {
                        if matches!(self.peek(), TokenKind::RBracket) {
                            break;
                        }
                        items.push(self.parse_expr()?);
                    }
                }
                let end_span = self.peek_span();
                self.expect(&TokenKind::RBracket)?;
                let list_span = start..end_span.end;
                Ok(Spanned {
                    node: Expr::List(items),
                    span: list_span,
                })
            }

            // Record literal { name: expr, ... }
            TokenKind::LBrace => {
                self.advance();
                let mut fields = Vec::new();
                if !matches!(self.peek(), TokenKind::RBrace) {
                    let name = self.expect_ident()?;
                    self.expect(&TokenKind::Colon)?;
                    let val = self.parse_expr()?;
                    fields.push((name, val));
                    while self.match_token(&TokenKind::Comma) {
                        if matches!(self.peek(), TokenKind::RBrace) {
                            break;
                        }
                        let name = self.expect_ident()?;
                        self.expect(&TokenKind::Colon)?;
                        let val = self.parse_expr()?;
                        fields.push((name, val));
                    }
                }
                let end = self.peek_span().end;
                self.expect(&TokenKind::RBrace)?;
                Ok(Spanned {
                    node: Expr::Record(fields),
                    span: start..end,
                })
            }

            // Literals
            TokenKind::Int(_)
            | TokenKind::Float(_)
            | TokenKind::True
            | TokenKind::False
            | TokenKind::CharLit(_)
            | TokenKind::StringLit(_) => {
                let lit = self.parse_literal()?;
                let end = self.tokens[self.pos - 1].span.end;
                Ok(Spanned {
                    node: Expr::Literal(lit),
                    span: start..end,
                })
            }

            // If expression: desugar if-else to match on bool
            // `if c then A else B end` → `match c do case true -> A case false -> B end`
            // `if c then A end` (no else) → Expr::If (statement, returns unit)
            TokenKind::If => {
                self.advance();
                let cond = self.parse_expr()?;
                self.expect_contextual("then")?;
                let then_branch = self.parse_stmt_list()?;
                let else_branch = if self.match_keyword(&TokenKind::Else) {
                    Some(self.parse_stmt_list()?)
                } else {
                    None
                };
                self.expect(&TokenKind::End)?;
                let end = self.tokens[self.pos - 1].span.end;
                if let Some(eb) = else_branch {
                    // Desugar to match bool — gets proper expression type inference
                    Ok(Spanned {
                        node: Expr::Match {
                            target: Box::new(cond),
                            cases: vec![
                                MatchCase {
                                    pattern: Spanned {
                                        node: Pattern::Literal(Literal::Bool(true)),
                                        span: start..end,
                                    },
                                    body: then_branch,
                                },
                                MatchCase {
                                    pattern: Spanned {
                                        node: Pattern::Literal(Literal::Bool(false)),
                                        span: start..end,
                                    },
                                    body: eb,
                                },
                            ],
                        },
                        span: start..end,
                    })
                } else {
                    Ok(Spanned {
                        node: Expr::If {
                            cond: Box::new(cond),
                            then_branch,
                            else_branch: None,
                        },
                        span: start..end,
                    })
                }
            }

            // Match expression: match expr do case pat -> body ... end
            TokenKind::Match => {
                self.advance();
                let target = self.parse_expr()?;
                // `do` is optional after match target (allow `match x case ...`)
                if matches!(self.peek(), TokenKind::Do) {
                    self.advance();
                }
                let mut cases = Vec::new();
                while matches!(self.peek(), TokenKind::Case) {
                    self.advance();
                    let pattern = self.parse_pattern()?;
                    self.expect(&TokenKind::Arrow)?;
                    let body = self.parse_stmt_list()?;
                    cases.push(MatchCase { pattern, body });
                }
                self.expect(&TokenKind::End)?;
                let end = self.tokens[self.pos - 1].span.end;
                Ok(Spanned {
                    node: Expr::Match {
                        target: Box::new(target),
                        cases,
                    },
                    span: start..end,
                })
            }

            // Sigil + ident (variable with sigil), or %[...] (linear list literal)
            TokenKind::Tilde | TokenKind::Percent => {
                // %[...] is a linear list literal — parse as Expr::List
                if matches!(self.peek(), TokenKind::Percent)
                    && matches!(self.peek_at_offset(1), Some(TokenKind::LBracket))
                {
                    self.advance(); // skip %
                    self.advance(); // skip [
                    let mut items = Vec::new();
                    if !matches!(self.peek(), TokenKind::RBracket) {
                        items.push(self.parse_expr()?);
                        while self.match_token(&TokenKind::Comma) {
                            if matches!(self.peek(), TokenKind::RBracket) {
                                break;
                            }
                            items.push(self.parse_expr()?);
                        }
                    }
                    let end_span = self.peek_span();
                    self.expect(&TokenKind::RBracket)?;
                    let list_span = start..end_span.end;
                    return Ok(Spanned {
                        node: Expr::List(items),
                        span: list_span,
                    });
                }

                let sigil = self.parse_sigil();
                let name = self.expect_ident()?;
                let end = self.tokens[self.pos - 1].span.end;
                Ok(Spanned {
                    node: Expr::Variable(name, sigil),
                    span: start..end,
                })
            }

            // Identifier — could be variable, function call, constructor, or path call
            TokenKind::Ident(ref s) => {
                let s = s.clone();
                let is_upper = Self::is_uppercase_ident(&s);

                // First, try to read a dotted path: a.b.c
                // This handles both Console.println(...) calls and module.fn(...) calls
                let first = self.expect_ident()?;
                let mut path = first.clone();
                let mut segments = vec![first];

                while matches!(self.peek(), TokenKind::Dot) {
                    let saved = self.pos;
                    self.advance(); // consume dot
                    match self.peek() {
                        TokenKind::Ident(_) => {
                            let next = self.expect_ident()?;
                            path = format!("{}.{}", path, next);
                            segments.push(next);
                        }
                        _ => {
                            // Not a dotted path, put back the dot
                            self.pos = saved;
                            break;
                        }
                    }
                }

                if matches!(self.peek(), TokenKind::LParen) {
                    if segments.len() > 1 || !is_upper {
                        // Multi-segment path call or lowercase function call
                        self.advance();
                        let args = self.parse_call_args()?;
                        let end = self.tokens[self.pos - 1].span.end;
                        return Ok(Spanned {
                            node: Expr::Call { func: path, args },
                            span: start..end,
                        });
                    } else {
                        // Single uppercase name with () — Constructor call
                        self.advance();
                        let args = self.parse_ctor_args()?;
                        let end = self.tokens[self.pos - 1].span.end;
                        return Ok(Spanned {
                            node: Expr::Constructor(path, args),
                            span: start..end,
                        });
                    }
                }

                // No parens following
                if is_upper && segments.len() == 1 {
                    // Single uppercase: Constructor with no args
                    let end = self.tokens[self.pos - 1].span.end;
                    Ok(Spanned {
                        node: Expr::Constructor(path, vec![]),
                        span: start..end,
                    })
                } else if segments.len() == 1 {
                    // Single lowercase: Variable
                    let end = self.tokens[self.pos - 1].span.end;
                    Ok(Spanned {
                        node: Expr::Variable(path, Sigil::Immutable),
                        span: start..end,
                    })
                } else {
                    // Multi-segment without parens: parse as first segment variable,
                    // then let postfix handle the rest via FieldAccess
                    // We need to rewind back to after the first segment
                    let first_name = segments[0].clone();
                    // Find the position right after the first ident
                    // We need to go back. The first ident was consumed at start.
                    // After that, each .ident consumed 2 tokens (dot + ident)
                    let extra_tokens = (segments.len() - 1) * 2;
                    self.pos -= extra_tokens;
                    let end = self.tokens[self.pos - 1].span.end;
                    Ok(Spanned {
                        node: Expr::Variable(first_name, Sigil::Immutable),
                        span: start..end,
                    })
                }
            }

            // Negative number as atom (when preceded by minus as prefix, not binary)
            TokenKind::Minus => {
                let lit = self.parse_literal()?;
                let end = self.tokens[self.pos - 1].span.end;
                Ok(Spanned {
                    node: Expr::Literal(lit),
                    span: start..end,
                })
            }

            _ => Err(ParseError {
                message: format!("expected expression, got {:?}", self.peek()),
                span: self.peek_span(),
            }),
        }
    }

    fn peek_at_offset(&self, offset: usize) -> Option<&TokenKind> {
        self.tokens.get(self.pos + offset).map(|t| &t.kind)
    }

    fn parse_call_args(&mut self) -> Result<Vec<(String, Spanned<Expr>)>, ParseError> {
        self.parse_comma_separated(|t| matches!(t, TokenKind::RParen), Self::parse_call_arg)
            .and_then(|args| {
                self.expect(&TokenKind::RParen)?;
                Ok(args)
            })
    }

    fn parse_call_arg(&mut self) -> Result<(String, Spanned<Expr>), ParseError> {
        // label : expr
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Colon)?;
        let val = self.parse_expr()?;
        Ok((name, val))
    }

    fn parse_ctor_args(&mut self) -> Result<Vec<(Option<String>, Spanned<Expr>)>, ParseError> {
        self.parse_comma_separated(|t| matches!(t, TokenKind::RParen), Self::parse_ctor_arg)
            .and_then(|args| {
                self.expect(&TokenKind::RParen)?;
                Ok(args)
            })
    }

    fn parse_ctor_arg(&mut self) -> Result<(Option<String>, Spanned<Expr>), ParseError> {
        self.parse_optional_labeled(Self::parse_expr)
    }

    fn parse_lambda(&mut self, start: usize) -> Result<Spanned<Expr>, ParseError> {
        self.advance(); // consume 'fn'

        let type_params = self.parse_type_params()?;

        let params = self.parse_params()?;
        self.expect(&TokenKind::Arrow)?;
        let ret_type = self.parse_type()?;
        let requires = self.parse_require_clause()?;
        let throws = self.parse_throws_clause()?;
        self.expect(&TokenKind::Do)?;
        let body = self.parse_stmt_list()?;
        if body.is_empty() {
            return Err(ParseError {
                message: "Function body cannot be empty".into(),
                span: self.peek_span(),
            });
        }
        self.expect(&TokenKind::End)?;
        let end = self.tokens[self.pos - 1].span.end;

        Ok(Spanned {
            node: Expr::Lambda {
                type_params,
                params,
                ret_type,
                requires,
                throws,
                body,
            },
            span: start..end,
        })
    }

    fn parse_handler_expr(&mut self, start: usize) -> Result<Spanned<Expr>, ParseError> {
        self.advance(); // consume 'handler'
        let coeffect_name = self.expect_ident()?;

        let requires = if self.match_keyword(&TokenKind::Require) {
            self.parse_row_or_type()?
        } else {
            Type::Row(vec![], None)
        };

        self.expect(&TokenKind::Do)?;

        let mut functions = Vec::new();
        while matches!(self.peek(), TokenKind::Fn) {
            functions.push(self.parse_handler_function()?);
        }

        self.expect(&TokenKind::End)?;
        let end = self.tokens[self.pos - 1].span.end;

        Ok(Spanned {
            node: Expr::Handler {
                coeffect_name,
                requires,
                functions,
            },
            span: start..end,
        })
    }

    fn parse_handler_function(&mut self) -> Result<Function, ParseError> {
        self.expect(&TokenKind::Fn)?;
        let name = self.expect_ident()?;

        let type_params = self.parse_type_params()?;

        let params = self.parse_params()?;
        self.expect(&TokenKind::Arrow)?;
        let ret_type = self.parse_type()?;
        let requires = self.parse_require_clause()?;
        let throws = self.parse_throws_clause()?;
        self.expect(&TokenKind::Do)?;
        let body = self.parse_stmt_list()?;
        if body.is_empty() {
            return Err(ParseError {
                message: "Function body cannot be empty".into(),
                span: self.peek_span(),
            });
        }
        self.expect(&TokenKind::End)?;

        Ok(Function {
            name,
            is_public: false,
            type_params,
            params,
            ret_type,
            requires,
            throws,
            body,
        })
    }

    // ---- Statement parsing ----

    fn parse_stmt_list(&mut self) -> Result<Vec<Spanned<Stmt>>, ParseError> {
        let mut stmts = Vec::new();
        loop {
            // Check for terminators
            match self.peek() {
                TokenKind::End
                | TokenKind::Else
                | TokenKind::Catch
                | TokenKind::Case
                | TokenKind::Eof => break,
                _ => {}
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Spanned<Stmt>, ParseError> {
        let start = self.peek_span().start;

        match self.peek().clone() {
            TokenKind::Let => {
                self.advance();
                // Check for destructuring pattern: `let { ... } = expr` or `let Ctor(...) = expr`
                let is_record_pattern = matches!(self.peek(), TokenKind::LBrace);
                let is_ctor_pattern = matches!(self.peek(), TokenKind::Ident(ref s) if Self::is_uppercase_ident(s))
                    && matches!(self.peek_at_offset(1), Some(TokenKind::LParen));
                if is_record_pattern || is_ctor_pattern {
                    let pattern = self.parse_pattern()?;
                    self.expect(&TokenKind::Eq)?;
                    let value = self.parse_expr()?;
                    let end = value.span.end;
                    return Ok(Spanned {
                        node: Stmt::LetPattern { pattern, value },
                        span: start..end,
                    });
                }
                let sigil = self.parse_sigil();
                let name = self.expect_ident()?;
                let typ = if self.match_token(&TokenKind::Colon) {
                    Some(self.parse_type()?)
                } else {
                    None
                };
                self.expect(&TokenKind::Eq)?;
                let value = self.parse_expr()?;
                let end = value.span.end;
                Ok(Spanned {
                    node: Stmt::Let {
                        name,
                        sigil,
                        typ,
                        value,
                    },
                    span: start..end,
                })
            }

            TokenKind::Return => {
                self.advance();
                let value = self.parse_expr()?;
                let end = value.span.end;
                Ok(Spanned {
                    node: Stmt::Return(value),
                    span: start..end,
                })
            }

            TokenKind::If => self.parse_if_stmt(start),

            TokenKind::Match => self.parse_match_stmt(start),

            TokenKind::While => self.parse_while_stmt(start),

            TokenKind::For => self.parse_for_stmt(start),

            TokenKind::Try => self.parse_try_stmt(start),

            TokenKind::Conc => self.parse_conc_block(start),

            TokenKind::Inject => self.parse_inject_stmt(start),

            _ => {
                // Could be: assign or expression statement
                let expr = self.parse_expr()?;

                // Check for assignment: expr <- value
                if matches!(self.peek(), TokenKind::Assign) {
                    self.advance();
                    let value = self.parse_expr()?;
                    let end = value.span.end;
                    Ok(Spanned {
                        node: Stmt::Assign {
                            target: expr,
                            value,
                        },
                        span: start..end,
                    })
                } else {
                    let end = expr.span.end;
                    Ok(Spanned {
                        node: Stmt::Expr(expr),
                        span: start..end,
                    })
                }
            }
        }
    }

    fn parse_if_stmt(&mut self, start: usize) -> Result<Spanned<Stmt>, ParseError> {
        self.advance(); // consume 'if'
        let cond = self.parse_expr()?;
        self.expect_contextual("then")?;
        let then_branch = self.parse_stmt_list()?;
        let else_branch = if self.match_keyword(&TokenKind::Else) {
            Some(self.parse_stmt_list()?)
        } else {
            None
        };
        self.expect(&TokenKind::End)?;
        let end = self.tokens[self.pos - 1].span.end;

        // Desugar if-else to match on bool (same as expression form)
        let expr = if let Some(eb) = else_branch {
            Spanned {
                node: Expr::Match {
                    target: Box::new(cond),
                    cases: vec![
                        MatchCase {
                            pattern: Spanned {
                                node: Pattern::Literal(Literal::Bool(true)),
                                span: start..end,
                            },
                            body: then_branch,
                        },
                        MatchCase {
                            pattern: Spanned {
                                node: Pattern::Literal(Literal::Bool(false)),
                                span: start..end,
                            },
                            body: eb,
                        },
                    ],
                },
                span: start..end,
            }
        } else {
            Spanned {
                node: Expr::If {
                    cond: Box::new(cond),
                    then_branch,
                    else_branch: None,
                },
                span: start..end,
            }
        };
        Ok(Spanned {
            node: Stmt::Expr(expr),
            span: start..end,
        })
    }

    fn parse_match_stmt(&mut self, start: usize) -> Result<Spanned<Stmt>, ParseError> {
        self.advance(); // consume 'match'
        let target = self.parse_expr()?;
        self.expect(&TokenKind::Do)?;

        let mut cases = Vec::new();
        while matches!(self.peek(), TokenKind::Case) {
            self.advance();
            let pattern = self.parse_pattern()?;
            self.expect(&TokenKind::Arrow)?;
            let body = self.parse_stmt_list()?;
            cases.push(MatchCase { pattern, body });
        }

        self.expect(&TokenKind::End)?;
        let end = self.tokens[self.pos - 1].span.end;

        Ok(Spanned {
            node: Stmt::Expr(Spanned {
                node: Expr::Match {
                    target: Box::new(target),
                    cases,
                },
                span: start..end,
            }),
            span: start..end,
        })
    }

    fn parse_while_stmt(&mut self, start: usize) -> Result<Spanned<Stmt>, ParseError> {
        self.advance(); // consume 'while'
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::Do)?;
        let body = self.parse_stmt_list()?;
        self.expect(&TokenKind::End)?;
        let end = self.tokens[self.pos - 1].span.end;

        Ok(Spanned {
            node: Stmt::Expr(Spanned {
                node: Expr::While {
                    cond: Box::new(cond),
                    body,
                },
                span: start..end,
            }),
            span: start..end,
        })
    }

    fn parse_for_stmt(&mut self, start: usize) -> Result<Spanned<Stmt>, ParseError> {
        self.advance(); // consume 'for'
        let var = self.expect_ident()?;
        self.expect(&TokenKind::Eq)?;
        let start_expr = self.parse_expr()?;
        self.expect_contextual("to")?;
        let end_expr = self.parse_expr()?;
        self.expect(&TokenKind::Do)?;
        let body = self.parse_stmt_list()?;
        self.expect(&TokenKind::End)?;
        let end = self.tokens[self.pos - 1].span.end;

        Ok(Spanned {
            node: Stmt::Expr(Spanned {
                node: Expr::For {
                    var,
                    start: Box::new(start_expr),
                    end_expr: Box::new(end_expr),
                    body,
                },
                span: start..end,
            }),
            span: start..end,
        })
    }

    fn parse_try_stmt(&mut self, start: usize) -> Result<Spanned<Stmt>, ParseError> {
        self.advance(); // consume 'try'
        let body = self.parse_stmt_list()?;
        self.expect(&TokenKind::Catch)?;
        let catch_param = self.expect_ident()?;
        self.expect(&TokenKind::Arrow)?;
        let catch_body = self.parse_stmt_list()?;
        self.expect(&TokenKind::End)?;
        let end = self.tokens[self.pos - 1].span.end;

        Ok(Spanned {
            node: Stmt::Try {
                body,
                catch_param,
                catch_body,
            },
            span: start..end,
        })
    }

    fn parse_conc_block(&mut self, start: usize) -> Result<Spanned<Stmt>, ParseError> {
        self.advance(); // consume 'conc'
        self.expect(&TokenKind::Do)?;

        let mut tasks = Vec::new();
        while matches!(self.peek(), TokenKind::Task) {
            self.advance();
            let name = self.expect_ident()?;

            let throws = if self.match_keyword(&TokenKind::Throws) {
                let effs = self.parse_throws_ident_list()?;
                Type::Row(
                    effs.into_iter()
                        .map(|e| Type::UserDefined(e, vec![]))
                        .collect(),
                    None,
                )
            } else {
                Type::Row(vec![], None)
            };

            self.expect(&TokenKind::Do)?;
            let body = self.parse_stmt_list()?;
            self.expect(&TokenKind::End)?;

            tasks.push(Function {
                name,
                is_public: false,
                params: vec![],
                ret_type: Type::Unit,
                requires: Type::Row(vec![], None),
                throws,
                body,
                type_params: vec![],
            });
        }

        self.expect(&TokenKind::End)?;
        let end = self.tokens[self.pos - 1].span.end;

        Ok(Spanned {
            node: Stmt::Conc(tasks),
            span: start..end,
        })
    }

    fn parse_throws_ident_list(&mut self) -> Result<Vec<String>, ParseError> {
        self.parse_delimited_list(&TokenKind::LBrace, &TokenKind::RBrace, Self::expect_ident)
    }

    fn parse_inject_stmt(&mut self, start: usize) -> Result<Spanned<Stmt>, ParseError> {
        self.advance(); // consume 'inject'

        let mut handlers = Vec::new();
        handlers.push(self.parse_dotted_ident()?);
        while self.match_token(&TokenKind::Comma) {
            handlers.push(self.parse_dotted_ident()?);
        }

        self.expect(&TokenKind::Do)?;
        let body = self.parse_stmt_list()?;
        self.expect(&TokenKind::End)?;
        let end = self.tokens[self.pos - 1].span.end;

        Ok(Spanned {
            node: Stmt::Inject { handlers, body },
            span: start..end,
        })
    }

    fn parse_dotted_ident(&mut self) -> Result<String, ParseError> {
        let mut path = self.expect_ident()?;
        while matches!(self.peek(), TokenKind::Dot) {
            self.advance();
            let next = self.expect_ident()?;
            path = format!("{}.{}", path, next);
        }
        Ok(path)
    }

    // ---- Top-level parsing ----

    fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut definitions = Vec::new();
        while !self.at_end() {
            let start = self.peek_span().start;
            let def = self.parse_top_level()?;
            let end = self.tokens[self.pos - 1].span.end;
            definitions.push(Spanned {
                node: def,
                span: start..end,
            });
        }
        Ok(Program {
            definitions,
            source_file: None,
            source_text: None,
        })
    }

    fn parse_top_level(&mut self) -> Result<TopLevel, ParseError> {
        match self.peek().clone() {
            TokenKind::Export => {
                self.advance();
                self.parse_top_level_pub(true)
            }
            TokenKind::Ident(ref s) if s == "opaque" => {
                // opaque type ...
                self.advance();
                self.parse_type_def(false, true)
            }
            TokenKind::Type => self.parse_type_def(false, false),
            TokenKind::Exception => self.parse_exception_def(false),
            TokenKind::Import => self.parse_import_def(),
            TokenKind::Port => self.parse_port_def(false),
            TokenKind::External => self.parse_external_def(false),
            TokenKind::Let => self.parse_global_let(false),
            _ => Err(ParseError {
                message: format!("expected top-level definition, got {:?}", self.peek()),
                span: self.peek_span(),
            }),
        }
    }

    fn parse_top_level_pub(&mut self, is_public: bool) -> Result<TopLevel, ParseError> {
        match self.peek().clone() {
            TokenKind::Ident(ref s) if s == "opaque" => {
                self.advance();
                self.parse_type_def(is_public, true)
            }
            TokenKind::Type => self.parse_type_def(is_public, false),
            TokenKind::Exception => self.parse_exception_def(is_public),
            TokenKind::Port => self.parse_port_def(is_public),
            TokenKind::External => self.parse_external_def(is_public),
            TokenKind::Let => self.parse_global_let(is_public),
            _ => Err(ParseError {
                message: format!("expected definition after 'export', got {:?}", self.peek()),
                span: self.peek_span(),
            }),
        }
    }

    fn parse_type_def(&mut self, is_public: bool, is_opaque: bool) -> Result<TopLevel, ParseError> {
        self.expect(&TokenKind::Type)?;
        let name = self.expect_ident()?;

        let type_params = self.parse_type_params()?;

        self.expect(&TokenKind::Eq)?;

        // Try record body { ... }
        let record_err = if matches!(self.peek(), TokenKind::LBrace) {
            let saved = self.pos;
            match self.try_parse_record_body() {
                Ok(fields) => {
                    return Ok(TopLevel::TypeDef(TypeDef {
                        name,
                        is_public,
                        type_params,
                        fields,
                    }));
                }
                Err(e) => {
                    self.pos = saved;
                    Some(e)
                }
            }
        } else {
            None
        };

        // Sum type: Variant1(args) | Variant2(args) | ...
        let mut variants = Vec::new();
        variants.push(self.parse_variant_def().map_err(|mut e| {
            if let Some(record_err) = &record_err {
                e.message = format!(
                    "{} (also tried as record type: {})",
                    e.message, record_err.message
                );
            }
            e
        })?);
        while self.match_token(&TokenKind::Pipe) {
            variants.push(self.parse_variant_def()?);
        }

        Ok(TopLevel::Enum(EnumDef {
            name,
            is_public,
            is_opaque,
            type_params,
            variants,
        }))
    }

    fn parse_named_field(&mut self) -> Result<(String, Type), ParseError> {
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Colon)?;
        let typ = self.parse_type()?;
        Ok((name, typ))
    }

    fn try_parse_record_body(&mut self) -> Result<Vec<(String, Type)>, ParseError> {
        self.parse_delimited_list(
            &TokenKind::LBrace,
            &TokenKind::RBrace,
            Self::parse_named_field,
        )
    }

    fn parse_variant_def(&mut self) -> Result<VariantDef, ParseError> {
        let name = self.expect_ident()?;
        let fields = if matches!(self.peek(), TokenKind::LParen) {
            self.parse_delimited_list(
                &TokenKind::LParen,
                &TokenKind::RParen,
                Self::parse_variant_field,
            )?
        } else {
            vec![]
        };
        Ok(VariantDef { name, fields })
    }

    fn parse_variant_field(&mut self) -> Result<(Option<String>, Type), ParseError> {
        self.parse_optional_labeled(Self::parse_type)
    }

    fn parse_exception_def(&mut self, is_public: bool) -> Result<TopLevel, ParseError> {
        self.expect(&TokenKind::Exception)?;
        let name = self.expect_ident()?;
        if !Self::is_uppercase_ident(&name) {
            return Err(ParseError {
                message: "exception constructor must start with uppercase letter".to_string(),
                span: self.tokens[self.pos - 1].span.clone(),
            });
        }
        let fields = if matches!(self.peek(), TokenKind::LParen) {
            self.parse_delimited_list(
                &TokenKind::LParen,
                &TokenKind::RParen,
                Self::parse_variant_field,
            )?
        } else {
            vec![]
        };
        Ok(TopLevel::Exception(ExceptionDef {
            name,
            is_public,
            fields,
        }))
    }

    fn parse_import_def(&mut self) -> Result<TopLevel, ParseError> {
        self.expect(&TokenKind::Import)?;

        match self.peek().clone() {
            // import external path
            TokenKind::External => {
                self.advance();
                let path = self.parse_import_path()?;
                Ok(TopLevel::Import(Import {
                    path,
                    alias: None,
                    items: vec![],
                    is_external: true,
                }))
            }
            // import { items } from path
            // import { items }, * as alias from path
            TokenKind::LBrace => {
                let items =
                    self.parse_delimited_list(&TokenKind::LBrace, &TokenKind::RBrace, |this| {
                        let name = this.expect_ident()?;
                        let alias = if this.match_contextual("as") {
                            Some(this.expect_ident()?)
                        } else {
                            None
                        };
                        Ok(ImportItem { name, alias })
                    })?;

                // Optional: , * as alias
                let alias = if self.match_token(&TokenKind::Comma) {
                    self.expect(&TokenKind::Star)?;
                    self.expect_contextual("as")?;
                    Some(self.expect_ident()?)
                } else {
                    None
                };

                self.expect(&TokenKind::From)?;
                let path = self.parse_import_path()?;
                Ok(TopLevel::Import(Import {
                    path,
                    alias,
                    items,
                    is_external: false,
                }))
            }
            // import * as alias from path
            TokenKind::Star => {
                self.advance();
                self.expect_contextual("as")?;
                let alias = self.expect_ident()?;
                self.expect(&TokenKind::From)?;
                let path = self.parse_import_path()?;
                Ok(TopLevel::Import(Import {
                    path,
                    alias: Some(alias),
                    items: vec![],
                    is_external: false,
                }))
            }
            _ => Err(ParseError {
                message: format!("expected import form, got {:?}", self.peek()),
                span: self.peek_span(),
            }),
        }
    }

    fn parse_import_path(&mut self) -> Result<String, ParseError> {
        // Import path must be a quoted string, e.g. "stdlib/stdio.nx"
        match self.peek().clone() {
            TokenKind::StringLit(s) => {
                self.advance();
                Ok(s)
            }
            _ => Err(ParseError {
                message: "expected quoted import path (e.g. \"stdlib/foo.nx\")".to_string(),
                span: self.peek_span(),
            }),
        }
    }

    fn parse_port_def(&mut self, is_public: bool) -> Result<TopLevel, ParseError> {
        self.expect(&TokenKind::Port)?;
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Do)?;

        let mut functions = Vec::new();
        while matches!(self.peek(), TokenKind::Fn) {
            self.advance();
            let fn_name = self.expect_ident()?;
            let params = self.parse_params()?;
            self.expect(&TokenKind::Arrow)?;
            let ret_type = self.parse_type()?;
            let requires = self.parse_require_clause()?;
            let throws = self.parse_throws_clause()?;

            functions.push(FunctionSignature {
                name: fn_name,
                params,
                ret_type,
                requires,
                throws,
            });
        }

        self.expect(&TokenKind::End)?;

        Ok(TopLevel::Port(Port {
            name,
            is_public,
            functions,
        }))
    }

    fn parse_external_def(&mut self, is_public: bool) -> Result<TopLevel, ParseError> {
        let start = self.peek_span().start;
        self.expect(&TokenKind::External)?;
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Eq)?;

        // Bracket string for wasm symbol
        let wasm_name = match self.peek().clone() {
            TokenKind::StringLit(s) => {
                self.advance();
                s
            }
            _ => {
                return Err(ParseError {
                    message: "expected string literal for external symbol".to_string(),
                    span: self.peek_span(),
                })
            }
        };

        self.expect(&TokenKind::Colon)?;

        let type_params = self.parse_type_params()?;

        let typ = self.parse_type()?;
        let end = self.tokens[self.pos - 1].span.end;
        let ext_span = start..end;

        Ok(TopLevel::Let(GlobalLet {
            name,
            is_public,
            typ: Some(typ.clone()),
            value: Spanned {
                node: Expr::External(wasm_name, type_params, typ),
                span: ext_span,
            },
        }))
    }

    fn parse_global_let(&mut self, is_public: bool) -> Result<TopLevel, ParseError> {
        self.expect(&TokenKind::Let)?;
        let name = self.expect_ident()?;
        let typ = if self.match_token(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(&TokenKind::Eq)?;
        let value = self.parse_expr()?;

        Ok(TopLevel::Let(GlobalLet {
            name,
            is_public,
            typ,
            value,
        }))
    }
}

// ---- Public API ----

/// Parses a Nexus source string into a Program AST.
#[tracing::instrument(skip_all, name = "parse")]
pub fn parse(source: &str) -> Result<Program, Vec<ParseError>> {
    let tokens = lexer::tokenize(source).map_err(|errs| {
        errs.into_iter()
            .map(|e| ParseError {
                message: e.message,
                span: e.span,
            })
            .collect::<Vec<_>>()
    })?;

    let mut parser = Parser::new(tokens);
    match parser.parse_program() {
        Ok(prog) => Ok(prog),
        Err(e) => Err(vec![e]),
    }
}

/// Returns the full Nexus program parser (chumsky-compatible API for backward compat).
/// This function returns a closure that parses the source string.
pub fn parser() -> ParserWrapper {
    ParserWrapper
}

pub struct ParserWrapper;

impl ParserWrapper {
    pub fn parse(&self, source: &str) -> Result<Program, Vec<ParseError>> {
        parse(source)
    }
}

/// Returns a statement parser for REPL usage.
pub fn stmt_parser() -> StmtParserWrapper {
    StmtParserWrapper
}

pub struct StmtParserWrapper;

impl StmtParserWrapper {
    pub fn parse(&self, source: &str) -> Result<Spanned<Stmt>, Vec<ParseError>> {
        let tokens = lexer::tokenize(source).map_err(|errs| {
            errs.into_iter()
                .map(|e| ParseError {
                    message: e.message,
                    span: e.span,
                })
                .collect::<Vec<_>>()
        })?;
        let mut parser = Parser::new(tokens);
        parser.parse_stmt().map_err(|e| vec![e])
    }
}
