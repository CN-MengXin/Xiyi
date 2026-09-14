// src/syntactic/parse_func.rs
use crate::ast::*;
use crate::token::Token;
use super::module::Parser;

impl Parser {
    // 原名 parse_fn_sig，按要求改名为 parse_func_sig。
    // ===== 函数签名（支持可见性修饰符） =====
    pub(crate) fn parse_func_sig(&mut self) -> Result<FnSig, String> {
        // 可选可见性修饰符
        if let Some((Token::Pub, _)) = self.peek() {
            self.next();
        } else if let Some((Token::Priv, _)) = self.peek() {
            self.next();
        }
        self.expect(Token::Fn)?;
        let name = self.parse_ident()?;
        let generic_params = if let Some((Token::Lt, _)) = self.peek() {
            self.parse_generic_params()?
        } else {
            Vec::new()
        };
        self.push_generic_scope(&generic_params);
        self.expect(Token::LParen)?;
        let params = self.parse_params()?;
        self.expect(Token::RParen)?;
        let return_type = if let Some((Token::Arrow, _)) = self.peek() {
            self.next();
            Some(self.parse_type()?)
        } else {
            None
        };
        self.pop_generic_scope();
        self.expect(Token::Semicolon)?;
        Ok(FnSig {
            name,
            params,
            return_type,
            generic_params,
        })
    }

    // 原名 parse_fn_def，按要求改名为 parse_func_def。
    // ===== 函数定义（支持可见性修饰符） =====
    pub(crate) fn parse_func_def(&mut self, attributes: Vec<Attribute>) -> Result<FnDef, String> {
        // 可选可见性修饰符
        if let Some((Token::Pub, _)) = self.peek() {
            self.next();
        } else if let Some((Token::Priv, _)) = self.peek() {
            self.next();
        }
        self.expect(Token::Fn)?;
        let name = self.parse_ident()?;
        let generic_params = if let Some((Token::Lt, _)) = self.peek() {
            self.parse_generic_params()?
        } else {
            Vec::new()
        };
        self.push_generic_scope(&generic_params);
        self.expect(Token::LParen)?;
        let params = self.parse_params()?;
        self.expect(Token::RParen)?;
        let ret_type = if let Some((Token::Arrow, _)) = self.peek() {
            self.next();
            Some(self.parse_type()?)
        } else {
            None
        };
        let body = self.parse_block()?;
        self.pop_generic_scope();
        Ok(FnDef {
            attributes,
            name,
            generic_params,
            params,
            return_type: ret_type,
            body,
        })
    }

    // ===== parse_params（支持 &self / &mut self / 裸 self） =====
    pub(crate) fn parse_params(&mut self) -> Result<Vec<Param>, String> {
        let mut params = Vec::new();
        while let Some((token, _)) = self.peek() {
            if *token == Token::RParen { break; }

            // ===== 处理 &mut self / &self =====
            let handled_self = if let Some((Token::Amp, _)) = self.peek() {
                self.next(); // consume '&'
                let is_mut = if let Some((Token::Mut, _)) = self.peek() {
                    self.next();
                    true
                } else {
                    false
                };
                // 检查是否是 self（现在是 Token::SelfLower）
                if let Some((Token::SelfLower, _)) = self.peek() {
                    self.next(); // consume 'self'
                    let ty = Type::Ref {
                        mutable: is_mut,
                        inner: Box::new(Type::SelfType),
                    };
                    params.push(Param {
                        name: "self".to_string(),
                        ty,
                    });
                    if let Some((Token::Comma, _)) = self.peek() {
                        self.next();
                    }
                    true
                } else {
                    return Err("Expected 'self' after '&'".to_string());
                }
            } else if let Some((Token::SelfLower, _)) = self.peek() {
                // ===== 处理裸 self =====
                self.next(); // consume 'self'
                params.push(Param {
                    name: "self".to_string(),
                    ty: Type::SelfType,
                });
                if let Some((Token::Comma, _)) = self.peek() {
                    self.next();
                }
                true
            } else {
                false
            };

            if handled_self {
                continue;
            }

            // ===== 普通参数 =====
            let name = self.parse_ident()?;
            self.expect(Token::Colon)?;
            let ty = self.parse_type()?;
            params.push(Param { name, ty });

            match self.peek() {
                Some((Token::Comma, _)) => { self.next(); continue; }
                _ => break,
            }
        }
        Ok(params)
    }
}
