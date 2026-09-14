// src/syntactic/parse_generic.rs
use crate::ast::*;
use crate::token::Token;
use super::module::Parser;

impl Parser {
    // ===== 泛型作用域栈的三个辅助方法 =====
    pub(crate) fn push_generic_scope(&mut self, params: &[GenericParam]) {
        let names: Vec<String> = params
            .iter()
            .map(|gp| match gp {
                GenericParam::Type { name, .. } => name.clone(),
            })
            .collect();
        self.generic_scopes.push(names);
    }

    pub(crate) fn pop_generic_scope(&mut self) {
        self.generic_scopes.pop();
    }

    pub(crate) fn is_active_generic(&self, name: &str) -> bool {
        self.generic_scopes
            .iter()
            .any(|scope| scope.iter().any(|n| n == name))
    }

    // ===== 泛型参数 =====
    pub(crate) fn parse_generic_params(&mut self) -> Result<Vec<GenericParam>, String> {
        self.expect(Token::Lt)?;
        let mut params = Vec::new();
        while let Some((token, _)) = self.peek() {
            if *token == Token::Gt { break; }
            let name = self.parse_ident()?;
            let mut bounds = Vec::new();
            if let Some((Token::Colon, _)) = self.peek() {
                self.next();
                loop {
                    bounds.push(self.parse_ident()?);
                    if let Some((Token::Lt, _)) = self.peek() {
                    self.parse_generic_params()?;
                    }
                match self.peek() {
                    Some((Token::Plus, _)) => { self.next(); continue; }
                    _ => break,
                    }
                }
            }
            params.push(GenericParam::Type { name, bounds });
            match self.peek() {
                Some((Token::Comma, _)) => { self.next(); }
                _ => break,
            }
        }
        self.expect(Token::Gt)?;
        Ok(params)
    }

    // ===== where 子句 =====
    pub(crate) fn parse_where_clause(&mut self) -> Result<Vec<WhereClause>, String> {
        let mut clauses = Vec::new();
        while let Some((Token::Ident, name)) = self.peek() {
            let name = name.clone();
            self.next();
            self.expect(Token::Colon)?;
            let mut bounds = Vec::new();
            while let Some((Token::Ident, bound)) = self.peek() {
                bounds.push(bound.clone());
                self.next();
                if let Some((Token::Comma, _)) = self.peek() {
                    self.next();
                    break;
                }
            }
            clauses.push(WhereClause { type_name: name, bounds });
        }
        Ok(clauses)
    }
}
