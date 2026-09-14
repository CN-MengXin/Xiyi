// src/syntactic/module.rs
mod parse_attr;
mod parse_expr;
mod parse_func;
mod parse_generic;
mod parse_item;
mod parse_model;
mod parse_pattern;
mod parse_stmt;
mod parse_type;
mod helpers;
mod literal;

use crate::ast::*;
use crate::lexer::Lexer;
use crate::token::Token;

pub struct Parser {
    pub(crate) tokens: Vec<(Token, String)>,
    pub(crate) pos: usize,
    pub(crate) expr_id_counter: usize,
    pub(crate) generic_scopes: Vec<Vec<String>>,
    pub(crate) no_struct_literal: bool,
}

impl Parser {
    pub fn new(input: &str) -> Self {
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize().expect("Lexer error");
        Parser {
            tokens,
            pos: 0,
            expr_id_counter: 0,
            generic_scopes: Vec::new(),
            no_struct_literal: false,
        }
    }

    pub fn parse_program(&mut self) -> Result<Program, String> {
        let mut items = Vec::new();
        while self.peek().is_some() {
            items.extend(self.parse_item()?);
        }
        Ok(Program { items })
    }

    pub(crate) fn peek(&self) -> Option<&(Token, String)> {
        self.tokens.get(self.pos)
    }

    pub(crate) fn peek_nth(&self, n: usize) -> Option<&(Token, String)> {
        self.tokens.get(self.pos + n)
    }

    pub(crate) fn next(&mut self) -> Option<(Token, String)> {
        if self.pos < self.tokens.len() {
            let token = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(token)
        } else {
            None
        }
    }

    pub(crate) fn expect(&mut self, expected: Token) -> Result<String, String> {
        if let Some((token, value)) = self.next() {
            if token == expected {
                Ok(value)
            } else {
                Err(format!("Expected {:?}, got {:?}", expected, token))
            }
        } else {
            Err("Unexpected end of input".to_string())
        }
    }

    pub(crate) fn parse_ident(&mut self) -> Result<String, String> {
        if let Some((Token::Ident, value)) = self.next() {
            Ok(value)
        } else {
            Err("Expected identifier".to_string())
        }
    }

    pub(crate) fn next_expr_id(&mut self) -> usize {
        let id = self.expr_id_counter;
        self.expr_id_counter += 1;
        id
    }
}
