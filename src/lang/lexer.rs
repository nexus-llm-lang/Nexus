use super::ast::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Int(i64),
    Float(f64),
    StringLit(String),
    True,
    False,

    // Keywords (from the KEYWORDS list — these can't be used as identifiers)
    Let,
    Fn,
    Do,
    End,
    Return,
    If,
    Else,
    Match,
    Case,
    Task,
    Conc,
    Port,
    Type,
    Import,
    From,
    Pub,
    Require,
    Effect,
    Raise,
    Try,
    Catch,
    Handler,
    Inject,
    Exception,
    External,

    // Sigils & operators
    Tilde,    // ~
    Percent,  // %
    Ampersand, // &

    Arrow,    // ->
    FatArrow, // =>

    // Misc
    Star,     // *

    // Arithmetic
    Plus,     // +
    Minus,    // -
    Slash,    // /
    PlusDot,  // +.
    MinusDot, // -.
    StarDot,  // *.
    SlashDot, // /.
    PlusPlus, // ++

    // Comparison
    EqEq,     // ==
    Ne,       // !=
    Lt,       // <
    Le,       // <=
    Gt,       // >
    Ge,       // >=
    EqDot,    // ==.
    NeDot,    // !=.
    LtDot,    // <.
    LeDot,    // <=.
    GtDot,    // >.
    GeDot,    // >=.

    // Boolean
    AndAnd,   // &&
    OrOr,     // ||

    // Assignment
    Assign,   // <-
    Eq,       // =

    // Delimiters
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracketPipe, // [|
    RBracketPipe, // |]
    LBracket,
    RBracket,

    // Punctuation
    Colon,
    Comma,
    Dot,
    Pipe,

    // Special
    Ident(String),

    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct LexError {
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lex error at {:?}: {}", self.span, self.message)
    }
}

struct Lexer {
    source: Vec<char>,
    pos: usize,
    tokens: Vec<Token>,
    errors: Vec<LexError>,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Lexer {
            source: source.chars().collect(),
            pos: 0,
            tokens: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.source.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.source.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.source.get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn starts_with(&self, s: &str) -> bool {
        let chars: Vec<char> = s.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            if self.source.get(self.pos + i).copied() != Some(c) {
                return false;
            }
        }
        true
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // Skip whitespace (including newlines since they're not significant)
            while let Some(c) = self.peek() {
                if c.is_whitespace() {
                    self.advance();
                } else {
                    break;
                }
            }

            // Skip line comments
            if self.starts_with("//") {
                while let Some(c) = self.peek() {
                    self.advance();
                    if c == '\n' {
                        break;
                    }
                }
                continue;
            }

            // Skip block comments
            if self.starts_with("/*") {
                self.advance(); // '/'
                self.advance(); // '*'
                let mut depth = 1;
                while depth > 0 {
                    match self.advance() {
                        Some('/') if self.peek() == Some('*') => {
                            self.advance();
                            depth += 1;
                        }
                        Some('*') if self.peek() == Some('/') => {
                            self.advance();
                            depth -= 1;
                        }
                        None => {
                            self.errors.push(LexError {
                                message: "unterminated block comment".to_string(),
                                span: self.pos..self.pos,
                            });
                            break;
                        }
                        _ => {}
                    }
                }
                continue;
            }

            break;
        }
    }

    fn lex_bracket_string(&mut self) -> Option<TokenKind> {
        // We've already consumed the first '[', check for '=' or '['
        let start = self.pos - 1; // include the already-consumed '['
        let mut eq_count = 0;
        while self.peek() == Some('=') {
            self.advance();
            eq_count += 1;
        }
        if self.peek() != Some('[') {
            // Not a bracket string, need to back up
            // Reset position to after the initial '['
            self.pos = start + 1;
            return None;
        }
        self.advance(); // consume second '['

        let is_raw = eq_count >= 2;

        // Build closing sequence: ]===]
        let closing: Vec<char> = {
            let mut v = vec![']'];
            for _ in 0..eq_count {
                v.push('=');
            }
            v.push(']');
            v
        };

        let mut content = String::new();
        loop {
            // Check for closing sequence
            let mut found = true;
            for (i, &c) in closing.iter().enumerate() {
                if self.source.get(self.pos + i).copied() != Some(c) {
                    found = false;
                    break;
                }
            }
            if found {
                for _ in 0..closing.len() {
                    self.advance();
                }
                return Some(TokenKind::StringLit(content));
            }

            match self.advance() {
                None => {
                    self.errors.push(LexError {
                        message: "unterminated bracket string".to_string(),
                        span: start..self.pos,
                    });
                    return Some(TokenKind::StringLit(content));
                }
                Some('\n') | Some('\r') => {
                    self.errors.push(LexError {
                        message: "unclosed string literal".to_string(),
                        span: start..self.pos,
                    });
                    return Some(TokenKind::StringLit(content));
                }
                Some('\\') if !is_raw => {
                    match self.advance() {
                        Some('n') => content.push('\n'),
                        Some('r') => content.push('\r'),
                        Some('t') => content.push('\t'),
                        Some('\\') => content.push('\\'),
                        Some(c) => content.push(c),
                        None => {
                            self.errors.push(LexError {
                                message: "unterminated escape in string".to_string(),
                                span: start..self.pos,
                            });
                        }
                    }
                }
                Some(c) => content.push(c),
            }
        }
    }

    fn lex_double_quoted_string(&mut self) -> TokenKind {
        let start = self.pos - 1; // include the already-consumed '"'
        let mut content = String::new();
        loop {
            match self.advance() {
                None => {
                    self.errors.push(LexError {
                        message: "unterminated string literal".to_string(),
                        span: start..self.pos,
                    });
                    return TokenKind::StringLit(content);
                }
                Some('\n') | Some('\r') => {
                    self.errors.push(LexError {
                        message: "unclosed string literal".to_string(),
                        span: start..self.pos,
                    });
                    return TokenKind::StringLit(content);
                }
                Some('"') => {
                    return TokenKind::StringLit(content);
                }
                Some('\\') => {
                    match self.advance() {
                        Some('"') => content.push('"'),
                        Some('\\') => content.push('\\'),
                        Some('n') => content.push('\n'),
                        Some('r') => content.push('\r'),
                        Some('t') => content.push('\t'),
                        Some(c) => content.push(c),
                        None => {
                            self.errors.push(LexError {
                                message: "unterminated escape in string".to_string(),
                                span: start..self.pos,
                            });
                        }
                    }
                }
                Some(c) => content.push(c),
            }
        }
    }

    fn lex_number(&mut self, first: char) -> TokenKind {
        let mut s = String::new();
        s.push(first);
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        // Check for float
        if self.peek() == Some('.') && self.peek_at(1).map_or(false, |c| c.is_ascii_digit()) {
            s.push('.');
            self.advance(); // consume '.'
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    s.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
            TokenKind::Float(s.parse::<f64>().unwrap())
        } else {
            TokenKind::Int(s.parse::<i64>().unwrap())
        }
    }

    fn lex_ident_or_keyword(&mut self, first: char) -> TokenKind {
        let mut s = String::new();
        s.push(first);
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        match s.as_str() {
            "let" => TokenKind::Let,
            "fn" => TokenKind::Fn,
            "do" => TokenKind::Do,
            "end" => TokenKind::End,
            "return" => TokenKind::Return,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "match" => TokenKind::Match,
            "case" => TokenKind::Case,
            "task" => TokenKind::Task,
            "conc" => TokenKind::Conc,
            "port" => TokenKind::Port,
            "type" => TokenKind::Type,
            "import" => TokenKind::Import,
            "from" => TokenKind::From,
            "pub" => TokenKind::Pub,
            "require" => TokenKind::Require,
            "effect" => TokenKind::Effect,
            "raise" => TokenKind::Raise,
            "try" => TokenKind::Try,
            "catch" => TokenKind::Catch,
            "handler" => TokenKind::Handler,
            "inject" => TokenKind::Inject,
            "exception" => TokenKind::Exception,
            "external" => TokenKind::External,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            // "then", "as", "opaque", "ref", "borrow" are contextual —
            // they can be used as identifiers, so they stay as Ident
            _ => TokenKind::Ident(s),
        }
    }

    fn lex_all(&mut self) {
        loop {
            self.skip_whitespace_and_comments();
            if self.pos >= self.source.len() {
                self.tokens.push(Token {
                    kind: TokenKind::Eof,
                    span: self.pos..self.pos,
                });
                break;
            }

            let start = self.pos;
            let c = self.advance().unwrap();

            let kind = match c {
                '(' => TokenKind::LParen,
                ')' => TokenKind::RParen,
                '{' => TokenKind::LBrace,
                '}' => TokenKind::RBrace,
                '[' => {
                    if self.peek() == Some('|') {
                        self.advance();
                        TokenKind::LBracketPipe
                    } else if self.peek() == Some('[') || self.peek() == Some('=') {
                        // Bracket string
                        match self.lex_bracket_string() {
                            Some(tok) => tok,
                            None => {
                                // Was just '[', not a bracket string
                                TokenKind::LBracket
                            }
                        }
                    } else {
                        TokenKind::LBracket
                    }
                }
                ']' => TokenKind::RBracket,
                '|' => {
                    if self.peek() == Some(']') {
                        self.advance();
                        TokenKind::RBracketPipe
                    } else if self.peek() == Some('|') {
                        self.advance();
                        TokenKind::OrOr
                    } else {
                        TokenKind::Pipe
                    }
                }

                // Operators
                '+' => {
                    if self.peek() == Some('.') {
                        self.advance();
                        TokenKind::PlusDot
                    } else if self.peek() == Some('+') {
                        self.advance();
                        TokenKind::PlusPlus
                    } else {
                        TokenKind::Plus
                    }
                }
                '-' => {
                    if self.peek() == Some('>') {
                        self.advance();
                        TokenKind::Arrow
                    } else if self.peek() == Some('.') {
                        self.advance();
                        TokenKind::MinusDot
                    } else if self.peek().map_or(false, |c| c.is_ascii_digit()) {
                        // Negative number
                        let first_digit = self.advance().unwrap();
                        let tok = self.lex_number(first_digit);
                        // Negate the value
                        match tok {
                            TokenKind::Int(n) => TokenKind::Int(-n),
                            TokenKind::Float(n) => TokenKind::Float(-n),
                            _ => unreachable!(),
                        }
                    } else {
                        TokenKind::Minus
                    }
                }
                '*' => {
                    if self.peek() == Some('.') {
                        self.advance();
                        TokenKind::StarDot
                    } else {
                        TokenKind::Star
                    }
                }
                '/' => {
                    if self.peek() == Some('.') {
                        self.advance();
                        TokenKind::SlashDot
                    } else {
                        TokenKind::Slash
                    }
                }
                '=' => {
                    if self.peek() == Some('=') {
                        self.advance();
                        if self.peek() == Some('.') {
                            self.advance();
                            TokenKind::EqDot
                        } else {
                            TokenKind::EqEq
                        }
                    } else if self.peek() == Some('>') {
                        self.advance();
                        TokenKind::FatArrow
                    } else {
                        TokenKind::Eq
                    }
                }
                '!' => {
                    if self.peek() == Some('=') {
                        self.advance();
                        if self.peek() == Some('.') {
                            self.advance();
                            TokenKind::NeDot
                        } else {
                            TokenKind::Ne
                        }
                    } else {
                        self.errors.push(LexError {
                            message: format!("unexpected character '!'"),
                            span: start..self.pos,
                        });
                        continue;
                    }
                }
                '<' => {
                    if self.peek() == Some('-') {
                        self.advance();
                        TokenKind::Assign
                    } else if self.peek() == Some('=') {
                        self.advance();
                        if self.peek() == Some('.') {
                            self.advance();
                            TokenKind::LeDot
                        } else {
                            TokenKind::Le
                        }
                    } else if self.peek() == Some('.') {
                        self.advance();
                        TokenKind::LtDot
                    } else {
                        TokenKind::Lt
                    }
                }
                '>' => {
                    if self.peek() == Some('=') {
                        self.advance();
                        if self.peek() == Some('.') {
                            self.advance();
                            TokenKind::GeDot
                        } else {
                            TokenKind::Ge
                        }
                    } else if self.peek() == Some('.') {
                        self.advance();
                        TokenKind::GtDot
                    } else {
                        TokenKind::Gt
                    }
                }
                '&' => {
                    if self.peek() == Some('&') {
                        self.advance();
                        TokenKind::AndAnd
                    } else {
                        TokenKind::Ampersand
                    }
                }

                // Punctuation
                ':' => TokenKind::Colon,
                ',' => TokenKind::Comma,
                '.' => TokenKind::Dot,
                '~' => TokenKind::Tilde,
                '%' => TokenKind::Percent,
                '_' => {
                    // _ or _identifier — always treated as identifier
                    self.lex_ident_or_keyword('_')
                }

                // Double-quoted strings
                '"' => self.lex_double_quoted_string(),

                // Numbers
                c if c.is_ascii_digit() => self.lex_number(c),

                // Identifiers and keywords
                c if c.is_alphabetic() || c == '_' => self.lex_ident_or_keyword(c),

                other => {
                    self.errors.push(LexError {
                        message: format!("unexpected character '{}'", other),
                        span: start..self.pos,
                    });
                    continue;
                }
            };

            self.tokens.push(Token {
                kind,
                span: start..self.pos,
            });
        }
    }
}

pub fn tokenize(source: &str) -> Result<Vec<Token>, Vec<LexError>> {
    let mut lexer = Lexer::new(source);
    lexer.lex_all();
    if lexer.errors.is_empty() {
        Ok(lexer.tokens)
    } else {
        Err(lexer.errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_tokens() {
        let tokens = tokenize("let x = 42").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Let));
        assert!(matches!(tokens[1].kind, TokenKind::Ident(ref s) if s == "x"));
        assert!(matches!(tokens[2].kind, TokenKind::Eq));
        assert!(matches!(tokens[3].kind, TokenKind::Int(42)));
        assert!(matches!(tokens[4].kind, TokenKind::Eof));
    }

    #[test]
    fn test_bracket_string() {
        let tokens = tokenize("[=[hello]=]").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::StringLit(ref s) if s == "hello"));
    }

    #[test]
    fn test_operators() {
        let tokens = tokenize("+ - * / +. -. *. /. ++ == != < <= > >= -> <- && ||").unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert!(matches!(kinds[0], TokenKind::Plus));
        assert!(matches!(kinds[1], TokenKind::Minus));
        assert!(matches!(kinds[2], TokenKind::Star));
        assert!(matches!(kinds[3], TokenKind::Slash));
        assert!(matches!(kinds[4], TokenKind::PlusDot));
        assert!(matches!(kinds[5], TokenKind::MinusDot));
        assert!(matches!(kinds[6], TokenKind::StarDot));
        assert!(matches!(kinds[7], TokenKind::SlashDot));
        assert!(matches!(kinds[8], TokenKind::PlusPlus));
        assert!(matches!(kinds[9], TokenKind::EqEq));
        assert!(matches!(kinds[10], TokenKind::Ne));
        assert!(matches!(kinds[11], TokenKind::Lt));
        assert!(matches!(kinds[12], TokenKind::Le));
        assert!(matches!(kinds[13], TokenKind::Gt));
        assert!(matches!(kinds[14], TokenKind::Ge));
        assert!(matches!(kinds[15], TokenKind::Arrow));
        assert!(matches!(kinds[16], TokenKind::Assign));
        assert!(matches!(kinds[17], TokenKind::AndAnd));
        assert!(matches!(kinds[18], TokenKind::OrOr));
    }

    #[test]
    fn test_comments() {
        let tokens = tokenize("let x = 1 // comment\nlet y = 2").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Let));
        assert!(matches!(tokens[3].kind, TokenKind::Int(1)));
        assert!(matches!(tokens[4].kind, TokenKind::Let));
    }

    #[test]
    fn test_unit_literal() {
        let tokens = tokenize("()").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::LParen));
        assert!(matches!(tokens[1].kind, TokenKind::RParen));
    }

    #[test]
    fn test_float_operators() {
        let tokens = tokenize("==. !=. <. <=. >. >=.").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::EqDot));
        assert!(matches!(tokens[1].kind, TokenKind::NeDot));
        assert!(matches!(tokens[2].kind, TokenKind::LtDot));
        assert!(matches!(tokens[3].kind, TokenKind::LeDot));
        assert!(matches!(tokens[4].kind, TokenKind::GtDot));
        assert!(matches!(tokens[5].kind, TokenKind::GeDot));
    }

    #[test]
    fn test_negative_number() {
        let tokens = tokenize("-42").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Int(-42)));
    }

    #[test]
    fn test_double_quoted_string() {
        let tokens = tokenize(r#""hello""#).unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::StringLit(ref s) if s == "hello"));
    }

    #[test]
    fn test_double_quoted_string_escapes() {
        let tokens = tokenize(r#""a\"b\\c\nd\re\tf""#).unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::StringLit(ref s) if s == "a\"b\\c\nd\re\tf"));
    }

    #[test]
    fn test_double_quoted_string_unterminated() {
        let err = tokenize(r#""hello"#).unwrap_err();
        assert!(err[0].message.contains("unterminated string literal"));
    }

    #[test]
    fn test_double_quoted_string_newline_rejected() {
        let err = tokenize("\"hello\nworld\"").unwrap_err();
        assert!(err[0].message.contains("unclosed string literal"));
    }
}
