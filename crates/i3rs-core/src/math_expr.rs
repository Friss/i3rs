//! Recursive descent parser for math expressions.
//!
//! Grammar:
//! ```text
//! expr       = additive
//! additive   = multiplicative (('+' | '-') multiplicative)*
//! multiplicative = unary (('*' | '/' | '%') unary)*
//! unary      = '-' unary | primary
//! primary    = NUMBER | CHANNEL | func_call | '(' expr ')'
//! func_call  = IDENT '(' args ')'
//! args       = expr (',' expr)*
//! CHANNEL    = IDENT | '"' ... '"'
//! IDENT      = [a-zA-Z_][a-zA-Z0-9_]*
//! NUMBER     = [0-9]+ ('.' [0-9]*)? ([eE][+-]?[0-9]+)?
//! ```

use std::fmt;

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    /// Channel reference. The string is exactly as written (quoted or bare ident).
    Channel(String),
    BinaryOp(Box<Expr>, BinOp, Box<Expr>),
    UnaryNeg(Box<Expr>),
    FuncCall(String, Vec<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinOp::Add => write!(f, "+"),
            BinOp::Sub => write!(f, "-"),
            BinOp::Mul => write!(f, "*"),
            BinOp::Div => write!(f, "/"),
            BinOp::Mod => write!(f, "%"),
        }
    }
}

// ---------------------------------------------------------------------------
// Parse error
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parse error at position {}: {}", self.position, self.message)
    }
}

impl std::error::Error for ParseError {}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Ident(String),
    QuotedString(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    LParen,
    RParen,
    Comma,
}

struct Tokenizer<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn tokenize(&mut self) -> Result<Vec<(Token, usize)>, ParseError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            if self.pos >= self.input.len() {
                break;
            }
            let start = self.pos;
            let ch = self.input.as_bytes()[self.pos];
            let tok = match ch {
                b'+' => { self.pos += 1; Token::Plus }
                b'-' => { self.pos += 1; Token::Minus }
                b'*' => { self.pos += 1; Token::Star }
                b'/' => { self.pos += 1; Token::Slash }
                b'%' => { self.pos += 1; Token::Percent }
                b'(' => { self.pos += 1; Token::LParen }
                b')' => { self.pos += 1; Token::RParen }
                b',' => { self.pos += 1; Token::Comma }
                b'"' => self.read_quoted_string()?,
                b'0'..=b'9' | b'.' => self.read_number()?,
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.read_ident(),
                _ => {
                    return Err(ParseError {
                        message: format!("unexpected character '{}'", ch as char),
                        position: self.pos,
                    });
                }
            };
            tokens.push((tok, start));
        }
        Ok(tokens)
    }

    fn read_number(&mut self) -> Result<Token, ParseError> {
        let start = self.pos;
        // Integer part
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        // Decimal part
        if self.pos < self.input.len() && self.input.as_bytes()[self.pos] == b'.' {
            self.pos += 1;
            while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        // Exponent
        if self.pos < self.input.len() && matches!(self.input.as_bytes()[self.pos], b'e' | b'E') {
            self.pos += 1;
            if self.pos < self.input.len() && matches!(self.input.as_bytes()[self.pos], b'+' | b'-') {
                self.pos += 1;
            }
            while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        let s = &self.input[start..self.pos];
        let val: f64 = s.parse().map_err(|_| ParseError {
            message: format!("invalid number '{}'", s),
            position: start,
        })?;
        Ok(Token::Number(val))
    }

    fn read_ident(&mut self) -> Token {
        let start = self.pos;
        while self.pos < self.input.len() {
            let ch = self.input.as_bytes()[self.pos];
            if ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'.' {
                self.pos += 1;
            } else {
                break;
            }
        }
        Token::Ident(self.input[start..self.pos].to_string())
    }

    fn read_quoted_string(&mut self) -> Result<Token, ParseError> {
        let start = self.pos;
        self.pos += 1; // skip opening quote
        let content_start = self.pos;
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos] != b'"' {
            self.pos += 1;
        }
        if self.pos >= self.input.len() {
            return Err(ParseError {
                message: "unterminated string".into(),
                position: start,
            });
        }
        let s = self.input[content_start..self.pos].to_string();
        self.pos += 1; // skip closing quote
        Ok(Token::QuotedString(s))
    }
}

// ---------------------------------------------------------------------------
// Built-in function names (used to disambiguate func calls from channel refs)
// ---------------------------------------------------------------------------

const BUILTIN_FUNCTIONS: &[&str] = &[
    "smooth", "derivative", "integrate",
    "abs", "sqrt", "min", "max",
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
    "log", "ln", "exp", "pow",
    "floor", "ceil", "round",
    "clamp",
];

fn is_builtin_function(name: &str) -> bool {
    BUILTIN_FUNCTIONS.iter().any(|&f| f.eq_ignore_ascii_case(name))
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser {
    tokens: Vec<(Token, usize)>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<(Token, usize)>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn current_pos(&self) -> usize {
        self.tokens.get(self.pos).map(|(_, p)| *p).unwrap_or(usize::MAX)
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|(t, _)| t)
    }

    fn advance(&mut self) -> Option<&Token> {
        let tok = self.tokens.get(self.pos).map(|(t, _)| t);
        self.pos += 1;
        tok
    }

    fn expect_rparen(&mut self) -> Result<(), ParseError> {
        if self.peek() == Some(&Token::RParen) {
            self.advance();
            Ok(())
        } else {
            Err(ParseError {
                message: "expected ')'".into(),
                position: self.current_pos(),
            })
        }
    }

    // expr = additive
    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_additive()
    }

    // additive = multiplicative (('+' | '-') multiplicative)*
    fn parse_additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => BinOp::Add,
                Some(Token::Minus) => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::BinaryOp(Box::new(left), op, Box::new(right));
        }
        Ok(left)
    }

    // multiplicative = unary (('*' | '/' | '%') unary)*
    fn parse_multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => BinOp::Mul,
                Some(Token::Slash) => BinOp::Div,
                Some(Token::Percent) => BinOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::BinaryOp(Box::new(left), op, Box::new(right));
        }
        Ok(left)
    }

    // unary = '-' unary | primary
    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if self.peek() == Some(&Token::Minus) {
            self.advance();
            let expr = self.parse_unary()?;
            Ok(Expr::UnaryNeg(Box::new(expr)))
        } else {
            self.parse_primary()
        }
    }

    // primary = NUMBER | func_call | CHANNEL | '(' expr ')'
    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().cloned() {
            Some(Token::Number(n)) => {
                self.advance();
                Ok(Expr::Number(n))
            }
            Some(Token::QuotedString(s)) => {
                self.advance();
                Ok(Expr::Channel(s))
            }
            Some(Token::Ident(name)) => {
                // Look ahead: if next is '(' and it's a builtin, parse as function call
                if is_builtin_function(&name)
                    && self.tokens.get(self.pos + 1).map(|(t, _)| t) == Some(&Token::LParen)
                {
                    self.advance(); // consume ident
                    self.advance(); // consume '('
                    let args = self.parse_args()?;
                    self.expect_rparen()?;
                    Ok(Expr::FuncCall(name.to_ascii_lowercase(), args))
                } else {
                    self.advance();
                    Ok(Expr::Channel(name))
                }
            }
            Some(Token::LParen) => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect_rparen()?;
                Ok(expr)
            }
            Some(_) => Err(ParseError {
                message: "unexpected token".into(),
                position: self.current_pos(),
            }),
            None => Err(ParseError {
                message: "unexpected end of expression".into(),
                position: self.current_pos(),
            }),
        }
    }

    // args = expr (',' expr)*
    fn parse_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        if self.peek() == Some(&Token::RParen) {
            return Ok(Vec::new());
        }
        let mut args = vec![self.parse_expr()?];
        while self.peek() == Some(&Token::Comma) {
            self.advance();
            args.push(self.parse_expr()?);
        }
        Ok(args)
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a math expression string into an AST.
pub fn parse_expression(input: &str) -> Result<Expr, ParseError> {
    let mut tokenizer = Tokenizer::new(input);
    let tokens = tokenizer.tokenize()?;
    if tokens.is_empty() {
        return Err(ParseError {
            message: "empty expression".into(),
            position: 0,
        });
    }
    let mut parser = Parser::new(tokens);
    let expr = parser.parse_expr()?;
    if parser.pos < parser.tokens.len() {
        return Err(ParseError {
            message: "unexpected tokens after expression".into(),
            position: parser.current_pos(),
        });
    }
    Ok(expr)
}

/// Collect all channel names referenced in an expression.
pub fn referenced_channels(expr: &Expr) -> Vec<String> {
    let mut names = Vec::new();
    collect_channels(expr, &mut names);
    names.sort();
    names.dedup();
    names
}

fn collect_channels(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Channel(name) => out.push(name.clone()),
        Expr::Number(_) => {}
        Expr::BinaryOp(lhs, _, rhs) => {
            collect_channels(lhs, out);
            collect_channels(rhs, out);
        }
        Expr::UnaryNeg(inner) => collect_channels(inner, out),
        Expr::FuncCall(_, args) => {
            for arg in args {
                collect_channels(arg, out);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_number() {
        assert_eq!(parse_expression("42").unwrap(), Expr::Number(42.0));
        assert_eq!(parse_expression("3.14").unwrap(), Expr::Number(3.14));
        assert_eq!(parse_expression("1e3").unwrap(), Expr::Number(1000.0));
    }

    #[test]
    fn parse_channel_ref() {
        assert_eq!(
            parse_expression("Engine_Speed").unwrap(),
            Expr::Channel("Engine_Speed".into())
        );
        assert_eq!(
            parse_expression("\"Engine Speed\"").unwrap(),
            Expr::Channel("Engine Speed".into())
        );
    }

    #[test]
    fn parse_dotted_channel() {
        assert_eq!(
            parse_expression("GPS.Speed").unwrap(),
            Expr::Channel("GPS.Speed".into())
        );
    }

    #[test]
    fn parse_binary_ops() {
        let expr = parse_expression("a + b * c").unwrap();
        // Should parse as a + (b * c) due to precedence
        assert_eq!(
            expr,
            Expr::BinaryOp(
                Box::new(Expr::Channel("a".into())),
                BinOp::Add,
                Box::new(Expr::BinaryOp(
                    Box::new(Expr::Channel("b".into())),
                    BinOp::Mul,
                    Box::new(Expr::Channel("c".into())),
                ))
            )
        );
    }

    #[test]
    fn parse_parens() {
        let expr = parse_expression("(a + b) * c").unwrap();
        assert_eq!(
            expr,
            Expr::BinaryOp(
                Box::new(Expr::BinaryOp(
                    Box::new(Expr::Channel("a".into())),
                    BinOp::Add,
                    Box::new(Expr::Channel("b".into())),
                )),
                BinOp::Mul,
                Box::new(Expr::Channel("c".into())),
            )
        );
    }

    #[test]
    fn parse_unary_neg() {
        let expr = parse_expression("-x").unwrap();
        assert_eq!(expr, Expr::UnaryNeg(Box::new(Expr::Channel("x".into()))));
    }

    #[test]
    fn parse_func_call() {
        let expr = parse_expression("smooth(x, 10)").unwrap();
        assert_eq!(
            expr,
            Expr::FuncCall(
                "smooth".into(),
                vec![Expr::Channel("x".into()), Expr::Number(10.0)]
            )
        );
    }

    #[test]
    fn parse_nested_func() {
        let expr = parse_expression("abs(derivative(x))").unwrap();
        assert_eq!(
            expr,
            Expr::FuncCall(
                "abs".into(),
                vec![Expr::FuncCall("derivative".into(), vec![Expr::Channel("x".into())])]
            )
        );
    }

    #[test]
    fn parse_complex_expression() {
        // Wheel slip formula
        let expr = parse_expression("(WheelSpeed_RL - GPS.Speed) / GPS.Speed * 100").unwrap();
        let channels = referenced_channels(&expr);
        assert_eq!(channels, vec!["GPS.Speed", "WheelSpeed_RL"]);
    }

    #[test]
    fn parse_error_empty() {
        assert!(parse_expression("").is_err());
    }

    #[test]
    fn parse_error_unmatched_paren() {
        assert!(parse_expression("(a + b").is_err());
    }

    #[test]
    fn parse_error_unterminated_string() {
        assert!(parse_expression("\"hello").is_err());
    }

    #[test]
    fn parse_error_trailing_tokens() {
        assert!(parse_expression("a b").is_err());
    }
}
