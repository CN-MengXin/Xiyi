// src/syntactic/parse_pattern.rs
use crate::ast::*;
use crate::token::Token;
use super::Parser;

impl Parser {
    // ===== parse_pattern =====
    // 关键重构：AST 的 Pattern 枚举早就加了 IntLiteral/BoolLiteral/
    // Struct/Tuple/Array 这五种（外加一个 CharLiteral，见下面单独的
    // 说明），但这个函数一直只认 Wildcard/EnumVariant/
    // EnumVariantWithBinding 三种——上游 parser 从来没产出过其它五种，
    // MIR 层已经能 lower 它们也没用。现在先看第一个 token 是什么，把
    // 新增变体的入口分流出去，分不出来的才落到最后"读一个标识符，按
    // Wildcard/Struct/Enum::Variant 三选一"的原有路径。
    //
    // 没接的一种：CharLiteral(char)。翻了一遍 literal.rs，这个项目里
    // 表达式层面从来没实现过字符字面量的词法/语法支持（那边只处理
    // 整数/浮点/字符串/字节字符串/布尔，没有字符字面量对应的 token），
    // 手头也没有 token.rs 能核实真实的 token 名字——与其猜一个大概率
    // 编不过的 token 名字，不如先如实留空，等字符字面量的词法支持真正
    // 做出来、有确定的 token 可用时再补这一种模式。
    pub(crate) fn parse_pattern(&mut self) -> Result<Pattern, String> {
        match self.peek() {
            // 通配符 `_`：Ident 分支要跟"普通标识符开头的模式"共用同一个
            // token 种类，得先看清楚具体是不是 `_` 这个特殊名字，不是
            // 才继续往下走 Ident 主路径。
            Some((Token::Ident, name)) if name == "_" => {
                self.next();
                Ok(Pattern::Wildcard)
            }
            Some((Token::Integer, value)) => {
                let value = value.clone();
                self.next();
                let n = value
                    .parse::<i64>()
                    .map_err(|_| format!("invalid integer pattern literal: {}", value))?;
                Ok(Pattern::IntLiteral(n))
            }
            // 关键新增：Token::CharLit——Lexer 已经把 \n/\t/\xNN/\u{...}
            // 这些转义序列解码成实际字符了（跟 Token::String 一样，
            // parser 这边只是从原始文本里把包裹用的引号剥掉），不需要
            // 重新做一遍转义解析。
            Some((Token::CharLit, value)) => {
                let value = value.clone();
                self.next();
                let ch = Self::extract_char_literal(&value)?;
                Ok(Pattern::CharLiteral(ch))
            }
            Some((Token::True, _)) => {
                self.next();
                Ok(Pattern::BoolLiteral(true))
            }
            Some((Token::False, _)) => {
                self.next();
                Ok(Pattern::BoolLiteral(false))
            }
            // 元组解构：`(a, b, _)`。按位置绑定，`_` 表示这个位置存在
            // 但不关心（对应 Pattern::Tuple 里的 None）。
            Some((Token::LParen, _)) => {
                self.next();
                let mut elems = Vec::new();
                while let Some((token, _)) = self.peek() {
                    if *token == Token::RParen {
                        break;
                    }
                    elems.push(self.parse_tuple_or_array_slot()?);
                    match self.peek() {
                        Some((Token::Comma, _)) => {
                            self.next();
                        }
                        _ => break,
                    }
                }
                self.expect(Token::RParen)?;
                Ok(Pattern::Tuple(elems))
            }
            // 定长数组解构：`[a, b, _]`，跟 Tuple 是同一个"按位置绑定，
            // `_` 表示跳过"的规则，只是括号种类不同、AST 变体不同。
            Some((Token::LBracket, _)) => {
                self.next();
                let mut elems = Vec::new();
                while let Some((token, _)) = self.peek() {
                    if *token == Token::RBracket {
                        break;
                    }
                    elems.push(self.parse_tuple_or_array_slot()?);
                    match self.peek() {
                        Some((Token::Comma, _)) => {
                            self.next();
                        }
                        _ => break,
                    }
                }
                self.expect(Token::RBracket)?;
                Ok(Pattern::Array(elems))
            }
            // 剩下的都是"以标识符开头"的三种：Wildcard 已经在最前面
            // 特判过了，走到这里只可能是 Struct 或 Enum::Variant，具体
            // 是哪个要往后多看才能确定，交给 parse_ident_led_pattern。
            _ => self.parse_ident_led_pattern(),
        }
    }

    // ===== 辅助：从 Token::CharLit 的原始文本里取出真正的 char =====
    // Lexer 已经把转义序列（\n、\t、\xNN、\u{...} 等）解码成实际字符，
    // 这里只需要把两端的单引号剥掉——剥掉之后应该恰好剩一个 Unicode
    // 标量值。空字符字面量 `''`（规范里明确非法）、剥完引号之后还剩
    // 不止一个字符（理论上不该出现，但防御性检查一下）都如实报错，
    // 不装作侥幸猜一个字符。
    fn extract_char_literal(raw: &str) -> Result<char, String> {
        let inner = raw
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .unwrap_or(raw);
        let mut chars = inner.chars();
        let ch = chars.next().ok_or_else(|| "empty char literal".to_string())?;
        if chars.next().is_some() {
            return Err(format!("char literal contains more than one character: {}", raw));
        }
        Ok(ch)
    }

    // ===== 元组/数组解构里的一个位置：要么是 `_`（不绑定），要么是
    // 一个用来绑定的新局部变量名 =====
    fn parse_tuple_or_array_slot(&mut self) -> Result<Option<String>, String> {
        if let Some((Token::Ident, name)) = self.peek() {
            if name == "_" {
                self.next();
                return Ok(None);
            }
        }
        Ok(Some(self.parse_ident()?))
    }

    // ===== 以标识符开头、且不是 `_` 的模式：Struct 解构 或 Enum::Variant =====
    // 两者语法上都以"一个标识符"开头，必须再往后看一个 token 才能
    // 区分：紧跟 `{` 是结构体解构，紧跟 `::` 是枚举变体。
    fn parse_ident_led_pattern(&mut self) -> Result<Pattern, String> {
        let name = self.parse_ident()?;

        // ===== 结构体解构：`Point { x, y }` / `Point { x: a, y: b }` =====
        // 不要求列出结构体的全部字段——跟 Rust 的 `Point { x, .. }` 部分
        // 模式类似，只解构关心的那几个，写几个就只处理几个。每个字段
        // 支持简写（`x` 绑定到同名局部变量 `x`）和显式重命名
        // （`x: a` 把字段 x 绑定到局部变量 a），跟 Rust 结构体模式的
        // 两种写法一致。
        if let Some((Token::LBrace, _)) = self.peek() {
            self.next(); // consume '{'
            let mut fields = Vec::new();
            while let Some((token, _)) = self.peek() {
                if *token == Token::RBrace {
                    break;
                }
                let field_name = self.parse_ident()?;
                let binding = if let Some((Token::Colon, _)) = self.peek() {
                    self.next();
                    self.parse_ident()?
                } else {
                    field_name.clone()
                };
                fields.push((field_name, binding));
                match self.peek() {
                    Some((Token::Comma, _)) => {
                        self.next();
                    }
                    _ => break,
                }
            }
            self.expect(Token::RBrace)?;
            return Ok(Pattern::Struct { struct_name: name, fields });
        }

        // "peek 到 :: 就消费并读下一段"这条原语现在收在 parse_path.rs
        // 的 try_read_path_segment 里，跟 parse_expr.rs 读 Ident::Ident
        // 限定名时用的是同一个函数。但这里读不到 :: 时要报的错，跟
        // parse_expr.rs 不一样——parse_expr.rs 读不到 :: 就落到别的
        // 分支继续解析（合法情况，不是错误），而模式语法里"标识符后面
        // 不是 ::"本身就是错，还要进一步区分"根本没写 ::"和"把 ::
        // 写成了单个 :"这两种不同的提示。这条区分只对模式语法有意义，
        // 不属于路径原语该管的事，所以放在这里、原语返回 None 之后
        // 自己再 peek 一次决定说哪句话。
        let variant_name = match self.try_read_path_segment() {
            Some(result) => result?,
            None => {
                return match self.peek() {
                    Some((Token::Colon, _)) => Err("Unexpected ':' in pattern, expected '::'".to_string()),
                    _ => Err("expected '::' after enum name in pattern".to_string()),
                };
            }
        };

        // ===== 检查是否带绑定：Enum::Variant(binding) =====
        if let Some((Token::LParen, _)) = self.peek() {
            self.next(); // consume '('
            let binding = self.parse_ident()?;
            self.expect(Token::RParen)?;
            return Ok(Pattern::EnumVariantWithBinding {
                enum_name: name,
                variant_name,
                binding,
            });
        }

        Ok(Pattern::EnumVariant {
            enum_name: name,
            variant_name,
        })
    }
}
