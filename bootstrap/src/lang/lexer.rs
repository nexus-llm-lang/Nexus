use super::ast::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Int(i64),
    Float(f64),
    StringLit(String),
    CharLit(char),
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
    Task,
    Port,
    Type,
    Import,
    From,
    Export,
    Require,
    Throws,
    Raise,
    Try,
    Catch,
    Handler,
    Inject,
    Exception,
    External,
    While,
    For,

    // Sigils & operators
    Tilde,     // ~
    Percent,   // %
    Ampersand, // &
    At,        // @
    Caret,     // ^
    Shl,       // <<
    Shr,       // >>

    Arrow,    // ->
    FatArrow, // =>

    // Misc
    Star, // *

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
    EqEq,  // ==
    Ne,    // !=
    Lt,    // <
    Le,    // <=
    Gt,    // >
    Ge,    // >=
    EqDot, // ==.
    NeDot, // !=.
    LtDot, // <.
    LeDot, // <=.
    GtDot, // >.
    GeDot, // >=.

    // Boolean
    AndAnd, // &&
    OrOr,   // ||

    // Assignment
    Assign, // <-
    Eq,     // =

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
    ColonColon, // ::
    Comma,
    Dot,
    Pipe,

    // Special
    Ident(String),

    /// Placeholder emitted after a lex error (e.g. invalid literal).
    /// Never reaches the parser — `tokenize()` returns `Err` when any errors exist.
    Error,

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

    /// Resolve an escape sequence after consuming the backslash.
    /// Handles: \a \b \t \n \v \f \r \e \\ \' \" \NNN (octal) \xNN \u{NNNN}
    /// Returns None on error (error already pushed).
    fn resolve_escape(&mut self, start: usize) -> Option<char> {
        match self.advance() {
            Some('a') => Some('\x07'),
            Some('b') => Some('\x08'),
            Some('t') => Some('\t'),
            Some('n') => Some('\n'),
            Some('v') => Some('\x0b'),
            Some('f') => Some('\x0c'),
            Some('r') => Some('\r'),
            Some('e') => Some('\x1b'),
            Some('\\') => Some('\\'),
            Some('\'') => Some('\''),
            Some('"') => Some('"'),
            // Octal escape: \0 through \377 (1–3 octal digits)
            Some(c) if ('0'..='7').contains(&c) => {
                let mut oct = String::with_capacity(3);
                oct.push(c);
                for _ in 0..2 {
                    match self.peek() {
                        Some(d) if ('0'..='7').contains(&d) => {
                            oct.push(d);
                            self.advance();
                        }
                        _ => break,
                    }
                }
                let val = u32::from_str_radix(&oct, 8).unwrap();
                match char::from_u32(val) {
                    Some(ch) => Some(ch),
                    None => {
                        self.errors.push(LexError {
                            message: format!("invalid octal escape \\{}", oct),
                            span: start..self.pos,
                        });
                        None
                    }
                }
            }
            Some('x') => {
                let mut hex = String::with_capacity(2);
                for _ in 0..2 {
                    match self.peek() {
                        Some(c) if c.is_ascii_hexdigit() => {
                            hex.push(c);
                            self.advance();
                        }
                        _ => {
                            self.errors.push(LexError {
                                message: format!(
                                    "\\x requires exactly 2 hex digits, got '{}'",
                                    hex
                                ),
                                span: start..self.pos,
                            });
                            return None;
                        }
                    }
                }
                let val = u8::from_str_radix(&hex, 16).unwrap();
                Some(val as char)
            }
            Some('u') => {
                if self.peek() != Some('{') {
                    self.errors.push(LexError {
                        message: "expected '{' after \\u".to_string(),
                        span: start..self.pos,
                    });
                    return None;
                }
                self.advance();
                let mut hex = String::new();
                loop {
                    match self.peek() {
                        Some('}') => {
                            self.advance();
                            break;
                        }
                        Some(c) if c.is_ascii_hexdigit() => {
                            hex.push(c);
                            self.advance();
                        }
                        _ => {
                            self.errors.push(LexError {
                                message: format!("invalid hex in \\u{{...}} escape: '{}'", hex),
                                span: start..self.pos,
                            });
                            return None;
                        }
                    }
                }
                match u32::from_str_radix(&hex, 16) {
                    Ok(cp) => match char::from_u32(cp) {
                        Some(c) => Some(c),
                        None => {
                            self.errors.push(LexError {
                                message: format!("invalid Unicode codepoint U+{:X}", cp),
                                span: start..self.pos,
                            });
                            None
                        }
                    },
                    Err(_) => {
                        self.errors.push(LexError {
                            message: format!("invalid hex in \\u{{...}} escape: '{}'", hex),
                            span: start..self.pos,
                        });
                        None
                    }
                }
            }
            Some(c) => {
                self.errors.push(LexError {
                    message: format!("unknown escape sequence '\\{}'", c),
                    span: start..self.pos,
                });
                None
            }
            None => {
                self.errors.push(LexError {
                    message: "unterminated escape sequence".to_string(),
                    span: start..self.pos,
                });
                None
            }
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
                    return Some(TokenKind::Error);
                }
                Some('\n') | Some('\r') => {
                    self.errors.push(LexError {
                        message: "unclosed string literal".to_string(),
                        span: start..self.pos,
                    });
                    return Some(TokenKind::Error);
                }
                Some('\\') if !is_raw => {
                    if let Some(c) = self.resolve_escape(start) {
                        content.push(c);
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
                    return TokenKind::Error;
                }
                Some('\n') | Some('\r') => {
                    self.errors.push(LexError {
                        message: "unclosed string literal".to_string(),
                        span: start..self.pos,
                    });
                    return TokenKind::Error;
                }
                Some('"') => {
                    return TokenKind::StringLit(content);
                }
                Some('\\') => {
                    if let Some(c) = self.resolve_escape(start) {
                        content.push(c);
                    }
                }
                Some(c) => content.push(c),
            }
        }
    }

    fn lex_char_literal(&mut self) -> TokenKind {
        let start = self.pos - 1; // include the already-consumed '\''
        let ch = match self.advance() {
            None => {
                self.errors.push(LexError {
                    message: "unterminated char literal".to_string(),
                    span: start..self.pos,
                });
                return TokenKind::Error;
            }
            Some('\\') => match self.resolve_escape(start) {
                Some(c) => c,
                None => return TokenKind::Error,
            },
            Some('\'') => {
                self.errors.push(LexError {
                    message: "empty char literal".to_string(),
                    span: start..self.pos,
                });
                return TokenKind::Error;
            }
            Some(c) => c,
        };
        match self.advance() {
            Some('\'') => TokenKind::CharLit(ch),
            _ => {
                self.errors.push(LexError {
                    message: "expected closing ' for char literal".to_string(),
                    span: start..self.pos,
                });
                TokenKind::Error
            }
        }
    }

    fn lex_number(&mut self, first: char) -> TokenKind {
        let start = self.pos - 1; // first char already consumed
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
            match s.parse::<f64>() {
                Ok(v) => TokenKind::Float(v),
                Err(_) => {
                    self.errors.push(LexError {
                        message: format!("invalid float literal '{}'", s),
                        span: start..self.pos,
                    });
                    TokenKind::Error
                }
            }
        } else {
            match s.parse::<i64>() {
                Ok(v) => TokenKind::Int(v),
                Err(_) => {
                    self.errors.push(LexError {
                        message: format!("integer literal '{}' overflows i64", s),
                        span: start..self.pos,
                    });
                    TokenKind::Error
                }
            }
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
            "task" => TokenKind::Task,
            "port" => TokenKind::Port,
            "type" => TokenKind::Type,
            "import" => TokenKind::Import,
            "from" => TokenKind::From,
            "export" => TokenKind::Export,
            "require" => TokenKind::Require,
            "throws" => TokenKind::Throws,
            "raise" => TokenKind::Raise,
            "try" => TokenKind::Try,
            "catch" => TokenKind::Catch,
            "handler" => TokenKind::Handler,
            "inject" => TokenKind::Inject,
            "exception" => TokenKind::Exception,
            "external" => TokenKind::External,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
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
                    if self.peek() == Some('<') {
                        self.advance();
                        TokenKind::Shl
                    } else if self.peek() == Some('-') {
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
                    if self.peek() == Some('>') {
                        self.advance();
                        TokenKind::Shr
                    } else if self.peek() == Some('=') {
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
                ':' => {
                    if self.peek() == Some(':') {
                        self.advance();
                        TokenKind::ColonColon
                    } else {
                        TokenKind::Colon
                    }
                }
                ',' => TokenKind::Comma,
                '.' => TokenKind::Dot,
                '~' => TokenKind::Tilde,
                '%' => TokenKind::Percent,
                '@' => TokenKind::At,
                '^' => TokenKind::Caret,
                '_' => {
                    // _ or _identifier — always treated as identifier
                    self.lex_ident_or_keyword('_')
                }

                // Char literals
                '\'' => self.lex_char_literal(),

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

            if matches!(kind, TokenKind::Error) {
                continue;
            }
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
        assert!(matches!(tokens[0].kind, TokenKind::Minus));
        assert!(matches!(tokens[1].kind, TokenKind::Int(42)));
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
    fn test_string_all_standard_escapes() {
        let tokens = tokenize(r#""\0\a\b\t\n\v\f\r\e\\\"""#).unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::StringLit(ref s)
            if s == "\0\x07\x08\t\n\x0b\x0c\r\x1b\\\""));
    }

    #[test]
    fn test_string_octal_escape() {
        // \033 = ESC (27), \0 = NUL, \177 = DEL (127), \101 = 'A' (65)
        let tokens = tokenize(r#""\033\0\177\101""#).unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::StringLit(ref s)
            if s == "\x1b\0\x7f\x41"));
    }

    #[test]
    fn test_string_octal_escape_greedy() {
        // \0 followed by non-octal 'A' → NUL + A
        let tokens = tokenize(r#""\0A""#).unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::StringLit(ref s) if s == "\0A"));
    }

    #[test]
    fn test_string_hex_escape() {
        let tokens = tokenize(r#""\x41\x7a""#).unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::StringLit(ref s) if s == "Az"));
    }

    #[test]
    fn test_string_unicode_escape() {
        let tokens = tokenize(r#""\u{1F600}""#).unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::StringLit(ref s) if s == "\u{1F600}"));
    }

    #[test]
    fn test_string_unknown_escape_is_error() {
        let err = tokenize(r#""\q""#).unwrap_err();
        assert!(err[0].message.contains("unknown escape"));
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

    #[test]
    fn test_colon_colon() {
        let tokens = tokenize("x :: xs").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Ident(ref s) if s == "x"));
        assert!(matches!(tokens[1].kind, TokenKind::ColonColon));
        assert!(matches!(tokens[2].kind, TokenKind::Ident(ref s) if s == "xs"));
    }

    #[test]
    fn test_colon_vs_colon_colon() {
        let tokens = tokenize("a: b :: c").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Ident(ref s) if s == "a"));
        assert!(matches!(tokens[1].kind, TokenKind::Colon));
        assert!(matches!(tokens[2].kind, TokenKind::Ident(ref s) if s == "b"));
        assert!(matches!(tokens[3].kind, TokenKind::ColonColon));
        assert!(matches!(tokens[4].kind, TokenKind::Ident(ref s) if s == "c"));
    }

    #[test]
    fn test_char_literal() {
        let tokens = tokenize("'a'").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::CharLit('a')));
    }

    #[test]
    fn test_char_all_standard_escapes() {
        for (input, expected) in [
            (r"'\0'", '\0'),
            (r"'\a'", '\x07'),
            (r"'\b'", '\x08'),
            (r"'\t'", '\t'),
            (r"'\n'", '\n'),
            (r"'\v'", '\x0b'),
            (r"'\f'", '\x0c'),
            (r"'\r'", '\r'),
            (r"'\e'", '\x1b'),
            (r"'\\'", '\\'),
            (r"'\''", '\''),
        ] {
            let tokens = tokenize(input).unwrap();
            assert!(
                matches!(tokens[0].kind, TokenKind::CharLit(c) if c == expected),
                "escape {} should produce {:?}",
                input,
                expected,
            );
        }
    }

    #[test]
    fn test_char_octal_escape() {
        let tokens = tokenize(r"'\033'").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::CharLit('\x1b')));
    }

    #[test]
    fn test_char_hex_escape() {
        let tokens = tokenize(r"'\x41'").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::CharLit('A')));
    }

    #[test]
    fn test_char_unicode_escape() {
        let tokens = tokenize(r"'\u{41}'").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::CharLit('A')));
    }

    #[test]
    fn test_char_literal_empty_is_error() {
        let err = tokenize("''").unwrap_err();
        assert!(err[0].message.contains("empty char literal"));
    }

    #[test]
    fn test_char_unknown_escape_is_error() {
        let err = tokenize(r"'\q'").unwrap_err();
        assert!(err[0].message.contains("unknown escape"));
    }
}
