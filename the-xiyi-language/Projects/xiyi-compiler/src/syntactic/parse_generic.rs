// src/syntactic/parse_generic.rs
use crate::ast::*;
use crate::token::Token;
use super::Parser;

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

    // ===== 显式泛型实参：`::<i32>`、`::<i32, f64>` 这类写法 =====
    // 跟 parse_generic_params 长得像，但完全不是一回事：那边读的是
    // 声明处的泛型*参数*（一串名字加各自的 bound，比如
    // `<T: Clone, U>`），这里读的是调用/构造处显式指定的泛型*实参*
    // （一串具体类型，比如 `<i32, f64>`），元素是 Type 不是
    // GenericParam，也没有 bound 要处理，两者共享的只是"尖括号里逗号
    // 分隔一串东西"这个外形，硬合并成一个函数反而要塞一堆判断去区分
    // "现在到底是在读参数还是实参"。
    pub(crate) fn parse_generic_args(&mut self) -> Result<Vec<Type>, String> {
        self.expect(Token::Lt)?;
        let mut args = Vec::new();
        while let Some((token, _)) = self.peek() {
            if *token == Token::Gt {
                break;
            }
            args.push(self.parse_type()?);
            match self.peek() {
                Some((Token::Comma, _)) => { self.next(); }
                Some((Token::Gt, _)) => break,
                _ => return Err("Expected ',' or '>' in generic args".to_string()),
            }
        }
        self.expect(Token::Gt)?;
        Ok(args)
    }

    // ===== where 子句 =====
    // 关键修复：旧版本遇到 `T: A + B` 时，读完 A 之后 peek 到的是 '+'，
    // 内层 while 的匹配条件是 `Token::Ident`，对不上、直接跳出循环——
    // 后面的 "+ B" 被原地丢弃，既不报错也不解析，静默把用户写的约束
    // 漏掉一半。parse_generic_params 那边已经用 Token::Plus 正确处理了
    // 多 bound，这里改成同样的写法：每读完一个 bound 就看下一个 token，
    // 是 '+' 就继续读下一个 bound，是 ',' 就结束这一条子句去读下一个
    // 类型参数，别的情况直接跳出。
    pub(crate) fn parse_where_clause(&mut self) -> Result<Vec<WhereClause>, String> {
        let mut clauses = Vec::new();
        while let Some((Token::Ident, name)) = self.peek() {
            let name = name.clone();
            self.next();
            self.expect(Token::Colon)?;
            let mut bounds = Vec::new();
            loop {
                if let Some((Token::Ident, bound)) = self.peek() {
                    bounds.push(bound.clone());
                    self.next();
                } else {
                    break;
                }
                match self.peek() {
                    Some((Token::Plus, _)) => { self.next(); continue; }
                    Some((Token::Comma, _)) => { self.next(); break; }
                    _ => break,
                }
            }
            clauses.push(WhereClause { type_name: name, bounds });
        }
        // 关键修复：`while let Some((Token::Ident, name)) = self.peek()`
        // 一次都不成立时（比如 `where { ... }`，`where` 后面直接就是
        // `{`），循环体一次都没跑，clauses 是空的，函数照样 Ok(vec![])
        // 返回——一个空的 where 子句被悄悄接受了，而 `where` 关键字都
        // 写出来了却一条约束都没有，正常情况下只能是写漏了，不该放行。
        if clauses.is_empty() {
            return Err("where clause must have at least one bound".to_string());
        }
        Ok(clauses)
    }
}
