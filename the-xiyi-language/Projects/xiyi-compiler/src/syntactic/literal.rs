// src/syntactic/literal.rs
//
// 从 parse_primary 里挖出来的字面量处理：整数、浮点、字符串、字节
// 字符串、布尔、单元。前五种（整数/浮点/字符串/字节字符串/布尔）靠
// 当前 token 就能唯一确定，收进 parse_literal，命中就消费 token 并
// 返回 Some(结果)，不是这几种字面量的开头就返回 None、不消费任何
// token，交还给 parse_primary 按其它情况继续处理。
//
// 单元 `()` 单独用 try_parse_unit_literal 处理，没有并进 parse_literal
// ——它和"普通括号表达式"共享同一个开头 token（LParen），必须多看一个
// token（是不是紧跟着 RParen）才能分辨，这跟其它几种"看一个 token 就
// 能确定"的字面量在识别方式上本质不同，硬塞进同一个函数里会让
// parse_literal 的"要么命中要么不消费"这个简单约定变复杂。

use crate::ast::*;
use crate::token::Token;
use super::module::Parser;

impl Parser {
    /// 尝试把当前 token 识别成一个字面量表达式（整数/浮点/字符串/
    /// 字节字符串/布尔）。命中就消费掉对应 token 并返回
    /// `Some(Ok(expr))`；不是这几种字面量的开头就返回 `None`，不消费
    /// 任何 token。字节字符串解码失败时返回 `Some(Err(..))`——已经
    /// 确认走的是这条路（消费了 `bytes` 和后面的字符串 token），不能
    /// 装作没识别出来退回去，只能把错误如实报出去。
    pub(crate) fn parse_literal(&mut self) -> Option<Result<Expr, String>> {
        let peek_token = self.peek().cloned();
        match peek_token {
            Some((Token::Integer, value)) => {
                self.next();
                let num = value.parse::<i64>().unwrap();
                Some(Ok(Expr {
                    id: self.next_expr_id(),
                    kind: ExprKind::Literal(Literal::Int32(num as i32)),
                }))
            }
            Some((Token::Float, value)) => {
                self.next();
                let num = value.parse::<f64>().unwrap();
                Some(Ok(Expr {
                    id: self.next_expr_id(),
                    kind: ExprKind::Literal(Literal::Float64(num)),
                }))
            }
            Some((Token::String, value)) => {
                self.next();
                let inner = if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
                    value[1..value.len()-1].to_string()
                } else {
                    value
                };
                Some(Ok(Expr {
                    id: self.next_expr_id(),
                    kind: ExprKind::Literal(Literal::String(inner)),
                }))
            }
            // bytes"..." 字节字符串字面量，类型是 &[u8]。bytes 后面必须
            // 紧跟字符串字面量——这是全局关键字，在语法层面就要求它
            // 单独出现时后面立刻是 String token，不允许中间插别的
            // token（空格/换行在词法阶段已经被跳过，没法在这一层区分
            // "有没有空格"，只能保证 token 序列上紧邻）。
            Some((Token::Bytes, _)) => {
                self.next();
                let value = match self.next() {
                    Some((Token::String, v)) => v,
                    other => {
                        return Some(Err(format!(
                            "Expected string literal immediately after 'bytes', got {:?}",
                            other
                        )));
                    }
                };
                let inner = if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
                    value[1..value.len() - 1].to_string()
                } else {
                    value
                };
                let bytes = match Self::decode_byte_string(&inner) {
                    Ok(b) => b,
                    Err(e) => return Some(Err(e)),
                };
                Some(Ok(Expr {
                    id: self.next_expr_id(),
                    kind: ExprKind::Literal(Literal::ByteString(bytes)),
                }))
            }
            Some((Token::True, _)) => {
                self.next();
                Some(Ok(Expr {
                    id: self.next_expr_id(),
                    kind: ExprKind::Literal(Literal::Bool(true)),
                }))
            }
            Some((Token::False, _)) => {
                self.next();
                Some(Ok(Expr {
                    id: self.next_expr_id(),
                    kind: ExprKind::Literal(Literal::Bool(false)),
                }))
            }
            _ => None,
        }
    }

    /// 尝试识别 `()` 单元字面量。需要往后多看一个 token 才能跟"普通
    /// 括号表达式"区分开——只有确认是 `(` 紧跟 `)` 才会真的消费这两个
    /// token，否则一个 token 都不碰，原样交还给调用方按分组括号处理。
    pub(crate) fn try_parse_unit_literal(&mut self) -> Option<Expr> {
        if let Some((Token::LParen, _)) = self.peek() {
            if let Some((Token::RParen, _)) = self.peek_nth(1) {
                self.next(); // consume '('
                self.next(); // consume ')'
                return Some(Expr {
                    id: self.next_expr_id(),
                    kind: ExprKind::Literal(Literal::Unit),
                });
            }
        }
        None
    }

    // ===== 辅助：把 bytes"..." 里的原始文本解码成 Vec<u8> =====
    // 支持规范里列的转义：\n \r \t \\ \" \0 \xNN（NN 两位十六进制，00–7F）；
    // 不支持 \u{...} Unicode 转义（字节字符串不含 Unicode 语义）。非转义
    // 字符必须本身就是单字节 ASCII（0x00–0x7F），否则报 error[BS001]。
    pub(crate) fn decode_byte_string(s: &str) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => bytes.push(b'\n'),
                    Some('r') => bytes.push(b'\r'),
                    Some('t') => bytes.push(b'\t'),
                    Some('\\') => bytes.push(b'\\'),
                    Some('"') => bytes.push(b'"'),
                    Some('0') => bytes.push(0u8),
                    Some('x') => {
                        let hi = chars.next().ok_or("incomplete \\x escape in byte string")?;
                        let lo = chars.next().ok_or("incomplete \\x escape in byte string")?;
                        let hex: String = [hi, lo].iter().collect();
                        let v = u8::from_str_radix(&hex, 16)
                            .map_err(|_| format!("invalid \\x escape in byte string: \\x{}", hex))?;
                        if v > 0x7F {
                            return Err(format!(
                                "error[BS001]: byte string contains non-ASCII byte 0x{:02X}",
                                v
                            ));
                        }
                        bytes.push(v);
                    }
                    Some(other) => {
                        return Err(format!("unknown escape sequence in byte string: \\{}", other))
                    }
                    None => return Err("incomplete escape sequence in byte string".to_string()),
                }
            } else {
                if (c as u32) > 0x7F {
                    return Err(format!(
                        "error[BS001]: byte string contains non-ASCII byte 0x{:X}",
                        c as u32
                    ));
                }
                bytes.push(c as u8);
            }
        }
        Ok(bytes)
    }
}
