use crate::{
    symbol_pool::SymbolPool,
    token::Token,
    unicode_ident::{is_xid_continue, is_xid_start},
};
use std::{fmt, iter::Peekable, str::CharIndices};

pub struct Lexer<'a> {
    source: &'a str,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    start: usize,
    end: usize,
}
impl Span {
    fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}
#[derive(Debug)]
pub struct LexError {
    pub message: String,
    pub byte_offset: usize,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (line {}, column {})",
            self.message, self.line, self.column
        )
    }
}
#[inline]
fn skip_hex(iter: &mut Peekable<CharIndices>) {
    while let Some(_) = iter.next_if(|&(_, ch)| ch.is_ascii_hexdigit() || ch == '_') {}
}
#[inline]
fn skip_number(iter: &mut Peekable<CharIndices>) -> bool {
    if let Some(_) = iter.next_if(|&(_, ch)| ('0' <= ch && ch <= '9') || ch == '_') {
        while let Some(_) = iter.next_if(|&(_, ch)| ('0' <= ch && ch <= '9') || ch == '_') {}
        true
    } else {
        false
    }
}
#[inline]
fn is_interger_type(t: &str) -> bool {
    if let "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128"
    | "isize" = t
    {
        true
    } else {
        false
    }
}
#[inline]
fn is_float_type(t: &str) -> bool {
    if let "f16" | "f32" | "f64" = t {
        true
    } else {
        false
    }
}
impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self { source: input }
    }
    pub fn get_str(&self, span: Span) -> &str {
        &self.source[span.start..span.end]
    }
    #[inline]
    fn parse_suffix(&self, iter: &mut Peekable<CharIndices>) -> (usize, usize) {
        if let Some((start, _)) = iter.next_if(|&(_, ch)| is_xid_start(ch)) {
            while let Some(_) = iter.next_if(|&(_, ch)| is_xid_continue(ch)) {}
            if let Some(&(i, _)) = iter.peek() {
                (start, i)
            } else {
                (start, self.source.len())
            }
        } else {
            if let Some(&(i, _)) = iter.peek() {
                (i, i)
            } else {
                (self.source.len(), self.source.len())
            }
        }
    }
    #[inline]
    fn position(&self, iter: &mut Peekable<CharIndices>) -> usize {
        if let Some(&(pos, _)) = iter.peek() {
            pos
        } else {
            self.source.len()
        }
    }
    fn parse_number(
        &self,
        start: usize,
        mut iter: &mut Peekable<CharIndices>,
    ) -> Result<Token, LexError> {
        skip_number(&mut iter);
        let (suffix_start, end) = self.parse_suffix(iter);
        if suffix_start < end {
            let suffix = &self.source[suffix_start..end];
            return if is_interger_type(suffix) {
                Ok(Token::Integer(Span::new(start, end)))
            } else if is_float_type(suffix) {
                Ok(Token::Float(Span::new(start, end)))
            } else {
                Err(self.build_error(format!("unknown suffix {suffix}"), suffix_start))
            };
        }
        let (dot, part) = if let Some(_) = iter.next_if(|&(_, ch)| ch == '.') {
            (true, skip_number(&mut iter))
        } else {
            (false, false)
        };
        let mut is_float = dot;
        if dot == part {
            if let Some(_) = iter.next_if(|&(_, ch)| ch == 'e' || ch == 'E') {
                iter.next_if(|&(_, ch)| ch == '-' || ch == '+');
                if !skip_number(&mut iter) {
                    return Err(self.build_error(
                        format!("expected at least one digit in exponent"),
                        iter.peek().map_or(self.source.len(), |&(i, _)| i),
                    ));
                }
                is_float = true;
            }
        }
        let (suffix_start, end) = self.parse_suffix(iter);
        if suffix_start < end {
            let suffix = &self.source[suffix_start..end];
            if is_interger_type(suffix) {
                if is_float {
                    return Err(self.build_error(
                        format!("invalid suffix {suffix} for float literal"),
                        suffix_start,
                    ));
                }
            } else if !is_float_type(suffix) {
                return Err(self.build_error(format!("unknown suffix {suffix}"), suffix_start));
            }
        }
        let end = self.position(iter);
        if is_float {
            Ok(Token::Integer(Span::new(start, end)))
        } else {
            Ok(Token::Float(Span::new(start, end)))
        }
    }
    fn parse_interger_suffix(&self, iter: &mut Peekable<CharIndices>) -> Result<usize, LexError> {
        let (start, end) = self.parse_suffix(iter);
        if start < end {
            let suffix = &self.source[start..end];
            if is_interger_type(suffix) {
                Ok(end)
            } else {
                Err(self.build_error(
                    format!("invalid suffix {suffix} for interger literal"),
                    start,
                ))
            }
        } else {
            Ok(start)
        }
    }
    pub fn tokenize(&mut self, symbol_pool: &mut SymbolPool) -> Result<Vec<Token>, LexError> {
        let trace = std::env::var("XIYI_LEXER_TRACE").is_ok();
        let mut tokens = Vec::new();
        let mut iter = self.source.char_indices().peekable();
        while let Some((start, ch)) = iter.next() {
            let token = if is_xid_start(ch) {
                while let Some(_) = iter.next_if(|&(_, ch)| is_xid_continue(ch)) {}
                let end = if let Some(&(i, _)) = iter.peek() {
                    i
                } else {
                    self.source.len()
                };
                let symbol = symbol_pool.intern(&self.source[start..end]);
                symbol.into_token()
            } else {
                match ch {
                    '0' => {
                        if let Some((_, ch)) = iter.peek() {
                            match ch {
                                'x' => {
                                    iter.next();
                                    while let Some(_) = iter.next_if(|&(_, ch)| ch == '_') {}
                                    if let Some(&(i, ch)) = iter.peek() {
                                        if ch.is_ascii_hexdigit() {
                                            skip_hex(&mut iter);
                                        } else {
                                            return Err(self.build_error_bad_char(i));
                                        }
                                    } else {
                                        return Err(self.build_error_bad_char(self.source.len()));
                                    }
                                    Token::Integer(Span::new(
                                        start,
                                        self.parse_interger_suffix(&mut iter)?,
                                    ))
                                }
                                'b' => {
                                    iter.next();
                                    while let Some(_) = iter.next_if(|&(_, ch)| ch == '_') {}
                                    if let Some(&(i, ch)) = iter.peek() {
                                        if ch == '0' || ch == '1' {
                                            while let Some(_) = iter.next_if(|&(_, ch)| {
                                                ch == '0' || ch == '1' || ch == '_'
                                            }) {}
                                        } else {
                                            return Err(self.build_error_bad_char(i));
                                        }
                                    } else {
                                        return Err(self.build_error_bad_char(self.source.len()));
                                    }
                                    Token::Integer(Span::new(
                                        start,
                                        self.parse_interger_suffix(&mut iter)?,
                                    ))
                                }
                                'o' => {
                                    iter.next();
                                    while let Some(_) = iter.next_if(|&(_, ch)| ch == '_') {}
                                    if let Some(&(i, ch)) = iter.peek() {
                                        if '0' <= ch && ch <= '7' {
                                            while let Some(_) = iter.next_if(|&(_, ch)| {
                                                ('0' <= ch && ch <= '7') || ch == '_'
                                            }) {}
                                        } else {
                                            return Err(self.build_error_bad_char(i));
                                        }
                                    } else {
                                        return Err(self.build_error_bad_char(self.source.len()));
                                    }
                                    Token::Integer(Span::new(
                                        start,
                                        self.parse_interger_suffix(&mut iter)?,
                                    ))
                                }
                                _ => self.parse_number(start, &mut iter)?,
                            }
                        } else {
                            Token::Integer(Span::new(start, start + 1))
                        }
                    }
                    '1'..='9' => self.parse_number(start, &mut iter)?,
                    '+' => {
                        if let Some((_, '=')) = iter.peek() {
                            iter.next();
                            Token::PlusEq
                        } else {
                            Token::Plus
                        }
                    }
                    '-' => {
                        if let Some((_, ch)) = iter.peek() {
                            match ch {
                                '=' => {
                                    iter.next();
                                    Token::MinusEq
                                }
                                '>' => {
                                    iter.next();
                                    Token::Arrow
                                }
                                _ => Token::Eq,
                            }
                        } else {
                            Token::Minus
                        }
                    }
                    '*' => {
                        if let Some((_, '=')) = iter.peek() {
                            iter.next();
                            Token::StarEq
                        } else {
                            Token::Star
                        }
                    }
                    '/' => {
                        if let Some((_, ch)) = iter.peek() {
                            match ch {
                                '=' => {
                                    iter.next();
                                    Token::SlashEq
                                }
                                '/' => {
                                    iter.next();
                                    while let Some(_) = iter.next_if(|&(_, ch)| ch != '\n') {}
                                    continue;
                                }
                                '*' => {
                                    loop {
                                        while let Some(_) = iter.next_if(|&(_, ch)| ch != '*') {}
                                        if let Some((_, ch)) = iter.next() {
                                            if ch == '/' {
                                                break;
                                            }
                                        } else {
                                            return Err(
                                                self.build_error_bad_char(self.source.len())
                                            );
                                        }
                                    }
                                    continue;
                                }
                                _ => Token::Slash,
                            }
                        } else {
                            Token::Slash
                        }
                    }
                    '%' => {
                        if let Some((_, '=')) = iter.peek() {
                            iter.next();
                            Token::PercentEq
                        } else {
                            Token::Percent
                        }
                    }
                    '=' => {
                        if let Some((_, ch)) = iter.peek() {
                            match ch {
                                '=' => {
                                    iter.next();
                                    Token::EqEq
                                }
                                '>' => {
                                    iter.next();
                                    Token::FatArrow
                                }
                                _ => Token::Eq,
                            }
                        } else {
                            Token::Eq
                        }
                    }
                    '!' => {
                        if let Some((_, '=')) = iter.peek() {
                            iter.next();
                            Token::Bang
                        } else {
                            Token::Neq
                        }
                    }
                    '<' => {
                        if let Some((_, '=')) = iter.peek() {
                            iter.next();
                            Token::Le
                        } else {
                            Token::Lt
                        }
                    }
                    '>' => {
                        if let Some((_, '=')) = iter.peek() {
                            iter.next();
                            Token::Ge
                        } else {
                            Token::Gt
                        }
                    }
                    '&' => {
                        if let Some((_, '&')) = iter.peek() {
                            iter.next();
                            Token::And
                        } else {
                            Token::Amp
                        }
                    }
                    '|' => {
                        if let Some((_, '=')) = iter.peek() {
                            iter.next();
                            Token::Or
                        } else {
                            Token::Pipe
                        }
                    }
                    '?' => Token::Question,
                    '.' => {
                        if let Some((_, '.')) = iter.peek() {
                            iter.next();
                            Token::Range
                        } else {
                            Token::Dot
                        }
                    }
                    ':' => {
                        if let Some((_, ':')) = iter.peek() {
                            iter.next();
                            Token::PathSep
                        } else {
                            Token::Colon
                        }
                    }
                    '{' => Token::LBrace,
                    '}' => Token::RBrace,
                    '[' => Token::LBracket,
                    ']' => Token::RBracket,
                    '(' => Token::LParen,
                    ')' => Token::RParen,
                    ';' => Token::Semicolon,
                    ',' => Token::Comma,
                    '#' => Token::Pound,
                    '"' => {
                        while let Some((_, ch)) = iter.next_if(|&(_, ch)| ch != '"') {
                            if ch == '\\' {
                                iter.next();
                            }
                        }
                        if let Some((i, _)) = iter.next() {
                            Token::String(Span::new(start, i + 1))
                        } else {
                            return Err(self.build_error_bad_char(self.source.len()));
                        }
                    }
                    '\'' => {
                        while let Some((_, ch)) = iter.next_if(|&(_, ch)| ch != '\'') {
                            if ch == '\\' {
                                iter.next();
                            }
                        }
                        if let Some((i, _)) = iter.next() {
                            Token::CharLiteral(Span::new(start, i + 1))
                        } else {
                            return Err(self.build_error_bad_char(self.source.len()));
                        }
                    }
                    _ => return Err(self.build_error_bad_char(start)),
                }
            };
            tokens.push(token);
            if trace {
                eprintln!(
                    "[LEXER] pos {}: {} ",
                    start,
                    match token {
                        Token::Ident(symbol) =>
                            format!("Ident \"{}\"", symbol_pool.resolve(symbol)),
                        Token::Integer(span) =>
                            format!("Interger {}", &self.source[span.start..span.end]),
                        Token::Float(span) =>
                            format!("Float {}", &self.source[span.start..span.end]),
                        Token::String(span) =>
                            format!("String \"{}\"", &self.source[span.start..span.end]),
                        Token::CharLiteral(span) =>
                            format!("Char '{}'", &self.source[span.start..span.end]),
                        t => format!("{:?}", t),
                    }
                );
            }
        }
        Ok(tokens)
    }

    fn build_error(&self, message: String, byte_offset: usize) -> LexError {
        let (line, column) = Self::line_col(self.source, byte_offset);

        LexError {
            message,
            byte_offset,
            line,
            column,
        }
    }

    fn build_error_bad_char(&self, byte_offset: usize) -> LexError {
        let bad_char = self.source[byte_offset..].chars().next();
        let (line, column) = Self::line_col(self.source, byte_offset);

        let message = match bad_char {
            Some(ch) => format!("unrecognized character {:?} (U+{:04X})", ch, ch as u32),
            None => "unexpected end of input".to_string(),
        };

        LexError {
            message,
            byte_offset,
            line,
            column,
        }
    }

    fn line_col(source: &str, byte_offset: usize) -> (usize, usize) {
        let mut line = 1;
        let mut col = 1;
        for (i, ch) in source.char_indices() {
            if i >= byte_offset {
                break;
            }
            if ch == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        (line, col)
    }
}

// 对字符串字面量内容进行转义处理，支持常见的转义序列和 Unicode 转义。
fn unescape_string(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some('\'') => result.push('\''),
                Some('0') => result.push('\0'),
                Some('x') => {
                    let mut hex = String::new();
                    for _ in 0..2 {
                        if let Some(c) = chars.next() {
                            hex.push(c);
                        } else {
                            break;
                        }
                    }
                    if hex.len() == 2 {
                        if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                            result.push(byte as char);
                        } else {
                            result.push_str(&format!("\\x{}", hex));
                        }
                    } else {
                        result.push_str("\\x");
                    }
                }
                Some('u') => {
                    if chars.next() == Some('{') {
                        let mut hex = String::new();
                        while let Some(c) = chars.next() {
                            if c == '}' {
                                break;
                            }
                            hex.push(c);
                        }
                        if !hex.is_empty() && hex.len() <= 6 {
                            if let Ok(codepoint) = u32::from_str_radix(&hex, 16) {
                                if let Some(c) = char::from_u32(codepoint) {
                                    result.push(c);
                                } else {
                                    result.push_str(&format!("\\u{{{}}}", hex));
                                }
                            } else {
                                result.push_str(&format!("\\u{{{}}}", hex));
                            }
                        } else {
                            result.push_str(&format!("\\u{{{}}}", hex));
                        }
                    } else {
                        result.push_str("\\u");
                    }
                }
                Some(c) => {
                    result.push_str(&format!("\\{}", c));
                }
                None => result.push('\\'),
            }
        } else {
            result.push(ch);
        }
    }
    result
}
