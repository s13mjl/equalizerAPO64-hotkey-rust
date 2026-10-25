//! Tokenizer for the expression language.

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),
    // operators / punctuation
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Xor,
    Not,
    BitAnd,
    BitOr,
    Shl,
    Shr,
    Question,
    Colon,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semicolon,
    Eof,
}

impl Token {
    pub fn describe(&self) -> String {
        match self {
            Token::Int(i) => format!("integer constant {}", i),
            Token::Float(f) => format!("float constant {}", f),
            Token::Str(_) => "string constant".into(),
            Token::Ident(s) => format!("identifier '{}'", s),
            other => format!("'{}'", other.symbol()),
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            Token::Plus => "+",
            Token::Minus => "-",
            Token::Star => "*",
            Token::Slash => "/",
            Token::Percent => "%",
            Token::Caret => "^",
            Token::Assign => "=",
            Token::PlusAssign => "+=",
            Token::MinusAssign => "-=",
            Token::StarAssign => "*=",
            Token::SlashAssign => "/=",
            Token::Eq => "==",
            Token::Ne => "!=",
            Token::Lt => "<",
            Token::Le => "<=",
            Token::Gt => ">",
            Token::Ge => ">=",
            Token::And => "and",
            Token::Or => "or",
            Token::Xor => "xor",
            Token::Not => "not",
            Token::BitAnd => "&",
            Token::BitOr => "|",
            Token::Shl => "<<",
            Token::Shr => ">>",
            Token::Question => "?",
            Token::Colon => ":",
            Token::LParen => "(",
            Token::RParen => ")",
            Token::LBrace => "{",
            Token::RBrace => "}",
            Token::LBracket => "[",
            Token::RBracket => "]",
            Token::Comma => ",",
            Token::Semicolon => ";",
            Token::Eof => "end of expression",
            _ => "?",
        }
    }
}

pub struct Lexer<'a> {
    chars: Vec<char>,
    pos: usize,
    _src: &'a str,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self { chars: src.chars().collect(), pos: 0, _src: src }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    pub fn next_token(&mut self) -> Result<Token, String> {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
        let c = match self.bump() {
            Some(c) => c,
            None => return Ok(Token::Eof),
        };
        let tok = match c {
            '+' => {
                if self.peek() == Some('=') {
                    self.pos += 1;
                    Token::PlusAssign
                } else {
                    Token::Plus
                }
            }
            '-' => {
                if self.peek() == Some('=') {
                    self.pos += 1;
                    Token::MinusAssign
                } else {
                    Token::Minus
                }
            }
            '*' => {
                if self.peek() == Some('=') {
                    self.pos += 1;
                    Token::StarAssign
                } else {
                    Token::Star
                }
            }
            '/' => {
                if self.peek() == Some('=') {
                    self.pos += 1;
                    Token::SlashAssign
                } else {
                    Token::Slash
                }
            }
            '%' => Token::Percent,
            '^' => Token::Caret,
            '=' => {
                if self.peek() == Some('=') {
                    self.pos += 1;
                    Token::Eq
                } else {
                    Token::Assign
                }
            }
            '!' => {
                if self.peek() == Some('=') {
                    self.pos += 1;
                    Token::Ne
                } else {
                    Token::Not
                }
            }
            '<' => {
                if self.peek() == Some('=') {
                    self.pos += 1;
                    Token::Le
                } else if self.peek() == Some('<') {
                    self.pos += 1;
                    Token::Shl
                } else {
                    Token::Lt
                }
            }
            '>' => {
                if self.peek() == Some('=') {
                    self.pos += 1;
                    Token::Ge
                } else if self.peek() == Some('>') {
                    self.pos += 1;
                    Token::Shr
                } else {
                    Token::Gt
                }
            }
            '&' => {
                if self.peek() == Some('&') {
                    self.pos += 1;
                    Token::And
                } else {
                    Token::BitAnd
                }
            }
            '|' => {
                if self.peek() == Some('|') {
                    self.pos += 1;
                    Token::Or
                } else {
                    Token::BitOr
                }
            }
            '?' => Token::Question,
            ':' => Token::Colon,
            '(' => Token::LParen,
            ')' => Token::RParen,
            '{' => Token::LBrace,
            '}' => Token::RBrace,
            '[' => Token::LBracket,
            ']' => Token::RBracket,
            ',' => Token::Comma,
            ';' => Token::Semicolon,
            '"' | '\'' => self.lex_string(c)?,
            _ => {
                if c.is_ascii_digit() || (c == '.' && self.peek().map(|x| x.is_ascii_digit()).unwrap_or(false)) {
                    self.lex_number(c)?
                } else if c.is_alphabetic() || c == '_' {
                    self.lex_ident(c)
                } else {
                    return Err(format!("unexpected character '{}'", c));
                }
            }
        };
        Ok(tok)
    }

    fn lex_string(&mut self, quote: char) -> Result<Token, String> {
        let mut s = String::new();
        loop {
            match self.bump() {
                None => return Err("unterminated string constant".into()),
                Some(c) if c == quote => break,
                Some('\\') => {
                    let e = self.bump().ok_or("unterminated escape sequence")?;
                    match e {
                        'n' => s.push('\n'),
                        'r' => s.push('\r'),
                        't' => s.push('\t'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        '\'' => s.push('\''),
                        '0' => s.push('\0'),
                        other => {
                            s.push('\\');
                            s.push(other);
                        }
                    }
                }
                Some(c) => s.push(c),
            }
        }
        Ok(Token::Str(s))
    }

    fn lex_number(&mut self, first: char) -> Result<Token, String> {
        let mut text = String::new();
        text.push(first);
        if first == '0' && matches!(self.peek(), Some('x') | Some('X')) {
            self.pos += 1;
            let mut hex = String::new();
            while let Some(c) = self.peek() {
                if c.is_ascii_hexdigit() {
                    hex.push(c);
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if hex.is_empty() {
                return Err("invalid hexadecimal constant".into());
            }
            let value = i64::from_str_radix(&hex, 16).map_err(|_| "invalid hexadecimal constant")?;
            return Ok(Token::Int(value));
        }
        let mut is_float = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                text.push(c);
                self.pos += 1;
            } else if c == '.' && !is_float {
                is_float = true;
                text.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            is_float = true;
            text.push('e');
            self.pos += 1;
            if matches!(self.peek(), Some('+') | Some('-')) {
                text.push(self.bump().unwrap());
            }
            let mut any = false;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    text.push(c);
                    self.pos += 1;
                    any = true;
                } else {
                    break;
                }
            }
            if !any {
                return Err("invalid exponent".into());
            }
        }
        if is_float {
            text.parse::<f64>()
                .map(Token::Float)
                .map_err(|_| "invalid float constant".into())
        } else {
            text.parse::<i64>()
                .map(Token::Int)
                .map_err(|_| "invalid integer constant".into())
        }
    }

    fn lex_ident(&mut self, first: char) -> Token {
        let mut text = String::new();
        text.push(first);
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                text.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        match text.as_str() {
            "and" => Token::And,
            "or" => Token::Or,
            "xor" => Token::Xor,
            "not" => Token::Not,
            _ => Token::Ident(text),
        }
    }
}
