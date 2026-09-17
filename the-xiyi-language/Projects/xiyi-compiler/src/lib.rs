// ===== 引擎 =====
#[path = "engine/module.rs"]
pub mod engine;

// ===== 词法 =====
pub mod token;
pub mod lexer;

// ===== 语法 =====
pub mod ast;
pub mod symbol_pool;
pub mod unicode_ident;
#[path = "syntactic/module.rs"]
pub mod syntactic;

// ===== 语义 =====
#[path = "semantic/module.rs"]
pub mod semantic;

// ===== HIR =====
pub mod hir;
pub mod hir_builder;
pub mod elaborate;

// ===== MIR =====
pub mod intrinsic;
pub mod mir;
pub mod mir_builder;
pub mod state;
pub mod guide;

// ===== 简化与检查 =====
pub mod control;
pub mod borrow;
pub mod monomorphic;
pub mod simplify;
pub mod calc;

// ===== 生成 =====
pub mod codegen;

pub use ast::*;

#[cfg(test)]
mod tests {
    use crate::lexer::Lexer;
    use crate::syntactic::Parser;
    use crate::semantic::TypeChecker;

    #[test]
    fn test_lexer_basic() {
        let input = "fn main() { let x: i32 = 10; }";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 13);
    }

    #[test]
    fn test_parse_basic() {
        let input = "fn main() -> i32 { let x: i32 = 10; x }";
        let mut parser = Parser::new(input);
        let program = parser.parse_program();
        assert!(program.is_ok());
    }

    #[test]
    fn test_type_check_ok() {
        let input = "fn main() -> i32 { let x: i32 = 10; x }";
        let mut parser = Parser::new(input);
        let program = parser.parse_program().unwrap();
        let mut checker = TypeChecker::new();
        // check_program 现在返回 Result<hir::HirProgram, String>，is_ok() 仍可用
        assert!(checker.check_program(&program).is_ok());
    }

    #[test]
    fn test_type_check_type_mismatch() {
        let input = "fn main() -> i32 { let x: bool = 10; x }";
        let mut parser = Parser::new(input);
        let program = parser.parse_program().unwrap();
        let mut checker = TypeChecker::new();
        assert!(checker.check_program(&program).is_err());
    }
}
