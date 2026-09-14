// src/syntactic/parse_stmt.rs
use crate::ast::*;
use crate::token::Token;
use super::module::Parser;

impl Parser {
    pub(crate) fn parse_block(&mut self) -> Result<Block, String> {
        self.expect(Token::LBrace)?;
        let mut stmts = Vec::new();
        while let Some((token, _)) = self.peek() {
            if *token == Token::RBrace { break; }
            stmts.push(self.parse_stmt()?);
        }
        self.expect(Token::RBrace)?;
        Ok(Block { stmts })
    }

    pub(crate) fn parse_unsafe_block(&mut self) -> Result<UnsafeBlockStmt, String> {
        self.expect(Token::Unsafe)?;
        let kind = if let Some((Token::Verify, _)) = self.peek() {
            self.next();
            UnsafeKind::Verify
        } else {
            UnsafeKind::Normal
        };
        let body = self.parse_block()?;
        Ok(UnsafeBlockStmt { kind, body })
    }

    // ===== 语句 =====
    // 关键修复：原来函数开头有一句
    // `eprintln!("parse_stmt at pos {}: {:?}", self.pos, self.peek());`
    // ——这是留在代码里的调试打印，每解析一条语句就往 stderr 里吐一行，
    // 编译真实程序时会把用户的终端刷屏刷到没法看，明显是开发阶段忘了
    // 删。直接去掉，不是这次拆分应该保留的行为。parse_expr 那边开头
    // 也有一句一模一样的，一并处理。
    pub(crate) fn parse_stmt(&mut self) -> Result<Stmt, String> {
        match self.peek() {
            Some((Token::Persist, _)) => {
                self.next();
                return match self.peek() {
                    Some((Token::Let, _)) => {
                        self.next();
                        self.parse_binding(false, true)
                    }
                    Some((Token::Var, _)) => {
                        self.next();
                        self.parse_binding(true, true)
                    }
                    _ => Err("Expected 'let' or 'var' after 'persist'".to_string()),
                };
            }
            Some((Token::Let, _)) => {
                self.next();
                return self.parse_binding(false, false);
            }
            Some((Token::Var, _)) => {
                self.next();
                return self.parse_binding(true, false);
            }
            Some((Token::Break, _)) => return self.parse_break_stmt(),
            Some((Token::Return, _)) => return self.parse_return_stmt(),
            Some((Token::While, _)) => return self.parse_while_stmt(),
            Some((Token::For, _)) => return self.parse_for_stmt(),
            Some((Token::Loop, _)) => return self.parse_loop_stmt(),
            Some((Token::If, _)) | Some((Token::Lack, _)) => {
                let expr = self.parse_expr()?;
                if let Some((Token::Semicolon, _)) = self.peek() {
                    self.next();
                }
                return Ok(Stmt::ExprStmt(expr));
            }
            Some((Token::Unsafe, _)) => {
                let expr = self.parse_expr()?;
                if let Some((Token::Semicolon, _)) = self.peek() {
                    self.next();
                }
                return Ok(Stmt::ExprStmt(expr));
            }
            _ => {}
        }

        // 赋值语句（包括复合赋值 += -= *= /= %=）
        // 关键修改：target 不再局限于裸标识符——self.len = ...、arr[i] = ...
        // 这类写法，第一个 token 是 self/arr，不是 Ident，靠"peek 第一个
        // token 是不是 Ident"这种一次性前瞻的老办法完全判断不出来。
        // 现在统一先把 target 当一个完整表达式解析出来（parse_expr 天然
        // 会在碰到 = 时停下，因为 = 不属于任何运算符优先级链条），再看
        // 后面跟不跟赋值类 token；如果不跟，target 本身就是一条普通表达式
        // 语句，直接复用，不用再解析第二遍。
        let target = self.parse_expr()?;

        let compound_op = match self.peek() {
            Some((Token::Eq, _)) => None,
            Some((Token::PlusEq, _)) => Some(BinaryOp::Add),
            Some((Token::MinusEq, _)) => Some(BinaryOp::Sub),
            Some((Token::StarEq, _)) => Some(BinaryOp::Mul),
            Some((Token::SlashEq, _)) => Some(BinaryOp::Div),
            Some((Token::PercentEq, _)) => Some(BinaryOp::Mod),
            _ => None,
        };
        // 关键修复：改回显式 match，不用 matches! 宏——项目里明确规定
        // 不用 Rust 的任何宏（包括 matches!）。
        let is_assign = match self.peek() {
            Some((Token::Eq, _))
            | Some((Token::PlusEq, _))
            | Some((Token::MinusEq, _))
            | Some((Token::StarEq, _))
            | Some((Token::SlashEq, _))
            | Some((Token::PercentEq, _)) => true,
            _ => false,
        };

        if is_assign {
            self.next(); // 吃掉 = 或 += / -= / *= / /= / %=

            let rhs = self.parse_expr()?;
            self.expect(Token::Semicolon)?;

            let expr = match compound_op {
                None => rhs,
                Some(op) => Expr {
                    id: self.next_expr_id(),
                    kind: ExprKind::BinaryOp {
                        op,
                        left: Box::new(target.clone()),
                        right: Box::new(rhs),
                    },
                },
            };

            return Ok(Stmt::Assign(AssignStmt { target: Box::new(target), expr: Box::new(expr) }));
        }

        // 不是赋值——target 其实就是一条普通表达式语句
        if let Some((Token::Semicolon, _)) = self.peek() {
            self.next();
        }
        Ok(Stmt::ExprStmt(target))
    }

    pub(crate) fn parse_binding(&mut self, mutable: bool, persist: bool) -> Result<Stmt, String> {
        let name = self.parse_ident()?;
        let ty = if let Some((Token::Colon, _)) = self.peek() {
            self.next();
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(Token::Eq)?;
        let init = self.parse_expr()?;
        self.expect(Token::Semicolon)?;
        Ok(Stmt::Let(LetStmt {
            name,
            ty,
            init: Box::new(init),
            mutable,
            persist,
        }))
    }

    pub(crate) fn parse_break_stmt(&mut self) -> Result<Stmt, String> {
        self.expect(Token::Break)?;
        self.expect(Token::Semicolon)?;
        Ok(Stmt::Break(BreakStmt {}))
    }

    pub(crate) fn parse_loop_stmt(&mut self) -> Result<Stmt, String> {
        self.expect(Token::Loop)?;
        let body = self.parse_block()?;
        Ok(Stmt::Loop(LoopStmt { body }))
    }

    pub(crate) fn parse_for_stmt(&mut self) -> Result<Stmt, String> {
        self.expect(Token::For)?;
        let var = self.parse_ident()?;
        self.expect(Token::In)?;
        let prev = self.no_struct_literal;
        self.no_struct_literal = true;
        let iterable = Box::new(self.parse_expr()?);
        self.no_struct_literal = prev;
        let body = self.parse_block()?;
        Ok(Stmt::For(ForStmt { var, iterable, body }))
    }

    pub(crate) fn parse_return_stmt(&mut self) -> Result<Stmt, String> {
        self.expect(Token::Return)?;
        if let Some((Token::Semicolon, _)) = self.peek() {
            self.next();
            Ok(Stmt::Return(None))
        } else {
            let expr = self.parse_expr()?;
            self.expect(Token::Semicolon)?;
            Ok(Stmt::Return(Some(expr)))
        }
    }

    pub(crate) fn parse_while_stmt(&mut self) -> Result<Stmt, String> {
        self.expect(Token::While)?;
        let prev = self.no_struct_literal;
        self.no_struct_literal = true;
        let cond = self.parse_expr()?;
        self.no_struct_literal = prev;
        let body = self.parse_block()?;
        Ok(Stmt::While(WhileStmt { cond: Box::new(cond), body }))
    }

    // 注：这个函数目前在整个文件里没有任何调用点——parse_stmt 自己
    // 内联实现了一遍"解析表达式、可选吃掉分号、包成 ExprStmt"的逻辑
    // （见上面 parse_stmt 末尾那几行），跟这里几乎一样，但没有复用它。
    // 这是拆分前就存在的死代码，不是这次引入的，先如实保留、按原样
    // 搬过来，要不要删或者改成让 parse_stmt 复用它，交给你决定。
    #[allow(dead_code)]
    pub(crate) fn parse_expr_stmt(&mut self) -> Result<Stmt, String> {
        let expr = self.parse_expr()?;
        if let Some((Token::Semicolon, _)) = self.peek() {
            self.next();
        }
        Ok(Stmt::ExprStmt(expr))
    }
}
