use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use crate::token::Token;

const K: u64 = 0x517c_c1b7_2722_0a95;

pub struct SymbolHasher(u64);

impl Default for SymbolHasher {
    #[inline]
    fn default() -> Self {
        // 初值给个非零常数，避免空输入时 finish()==0 造成桶 0 聚集
        Self(0x9E37_79B9_7F4A_7C15)
    }
}

impl Hasher for SymbolHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut h = self.0 ^ (bytes.len() as u64).wrapping_mul(K);

        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            let v = u64::from_le_bytes(c.try_into().unwrap());
            h = (h.rotate_left(5) ^ v).wrapping_mul(K);
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rem.len()].copy_from_slice(rem);
            h = (h.rotate_left(5) ^ u64::from_le_bytes(buf)).wrapping_mul(K);
        }
        self.0 = h;
    }

    // 整数键特化：Symbol/NodeId 等直接走这里，避免 to_ne_bytes 再逐字节
    #[inline]
    fn write_u8(&mut self, n: u8) {
        self.0 = (self.0.rotate_left(5) ^ n as u64).wrapping_mul(K);
    }
    #[inline]
    fn write_u16(&mut self, n: u16) {
        self.0 = (self.0.rotate_left(5) ^ n as u64).wrapping_mul(K);
    }
    #[inline]
    fn write_u32(&mut self, n: u32) {
        self.0 = (self.0.rotate_left(5) ^ n as u64).wrapping_mul(K);
    }
    #[inline]
    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(K);
    }
    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.0 = (self.0.rotate_left(5) ^ n as u64).wrapping_mul(K);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Symbol(pub usize);
impl Symbol {
    pub const fn is_predefined(self) -> bool {
        self.0 < SymbolPool::PREDEFINED_SYMBOL_COUNT
    }
    pub const fn into_token(self) -> Token {
        if self.is_predefined() {
            SymbolPool::PREDEFINED_SYMBOL_TOKENS[self.0]
        } else {
            Token::Ident(self)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Keyword(pub Symbol);

type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<SymbolHasher>>;

macro_rules! symbol_predefine {
    ($(($symbol:ident,$def:literal)),*) => {
        pub(self) mod __symbol_predefine {
            #[allow(non_camel_case_types, dead_code)]
            enum __Symbols { $($symbol),* }
            use std::{collections::HashMap, hash::BuildHasherDefault};
            use super::SymbolPool;
            use crate::token::Token;
            impl SymbolPool {
                $(
                    #[allow(dead_code,non_upper_case_globals)]
                    pub const $symbol: super::Symbol = super::Symbol(__Symbols::$symbol as usize);
                )*
                pub const PREDEFINED_SYMBOL_COUNT:usize=[$(($def,Self::$symbol)),*].len();
                pub const PREDEFINED_SYMBOL_TOKENS:[Token;SymbolPool::PREDEFINED_SYMBOL_COUNT]=[$(Token::$symbol),*];
                pub fn new() -> Self {
                    let mut map = HashMap::with_hasher(BuildHasherDefault::default());
                    map.extend([$(($def,Self::$symbol)),*]);
                    let strings = vec![$($def),*];
                    Self { map, strings }
                }
            }
        }
    };
}
symbol_predefine!(
    // ===== 声明与定义 =====
    (Fn, "fn"),
    (Let, "let"),
    (Var, "var"),
    (Mut, "mut"),
    (Const, "const"),
    (Struct, "struct"),
    (Enum, "enum"),
    (Interface, "interface"),
    (Implement, "implement"),
    (TypeKw, "type"),
    (Mod, "mod"),
    (Use, "use"),
    (Pub, "pub"),
    (Priv, "priv"),
    (Extern, "extern"),
    // ===== 路径与模块 =====
    (Crate, "crate"),
    (Super, "super"),
    (Here, "here"),
    // ===== 流程控制 =====
    (If, "if"),
    (Else, "else"),
    (Lack, "lack"),
    (Match, "match"),
    (For, "for"),
    (In, "in"),
    (While, "while"),
    (Loop, "loop"),
    (Break, "break"),
    (Continue, "continue"),
    (Return, "return"),
    (Yield, "yield"),
    // ===== 模式匹配与测试 =====
    (Bind, "bind"),
    (Is, "is"),
    (As, "as"),
    (Where, "where"),
    // ===== 错误处理 =====
    (Try, "try"),
    // ===== 并发与生成器 =====
    (Async, "async"),
    (Await, "await"),
    (Gen, "gen"),
    (Snapshot, "snapshot"),
    (Ref, "ref"),
    (Persist, "persist"),
    (Proto, "proto"),
    // ===== 系统与安全 =====
    (Unsafe, "unsafe"),
    (Verify, "verify"),
    (Deterministic, "deterministic"),
    (Probabilistic, "probabilistic"),
    (Checkpoint, "checkpoint"),
    (Actor, "actor"),
    (Model, "model"),
    (Tensor, "tensor"),
    // ===== 特殊标识符 =====
    (SelfType, "Self"),
    (SelfLower, "self"),
    (True, "true"),
    (False, "false"),
    (Nil, "nil"),
    (Bytes, "bytes"),
    // ===== 基本类型 =====
    (I8, "i8"),
    (I16, "i16"),
    (I32, "i32"),
    (I64, "i64"),
    (I128, "i128"),
    (ISize, "isize"),
    (U8, "u8"),
    (U16, "u16"),
    (U32, "u32"),
    (U64, "u64"),
    (U128, "u128"),
    (USize, "usize"),
    (F16, "f16"),
    (F32, "f32"),
    (F64, "f64"),
    (Bool, "bool"),
    (Char, "char"),
    (Str, "str"),
    (Never, "never")
);
pub struct SymbolPool {
    map: FxMap<&'static str, Symbol>,
    strings: Vec<&'static str>,
}

impl SymbolPool {
    #[inline]
    pub fn intern(&mut self, s: &str) -> Symbol {
        if let Some(&sym) = self.map.get(s) {
            return sym;
        }
        let leaked: &'static str = Box::leak(s.to_owned().into_boxed_str());
        let symbol = Symbol(self.strings.len());
        self.strings.push(leaked);
        self.map.insert(leaked, symbol);
        symbol
    }

    #[inline]
    pub fn get(&self, s: &str) -> Option<Symbol> {
        self.map.get(s).copied()
    }

    #[inline]
    pub fn resolve(&self, sym: Symbol) -> &'static str {
        self.strings[sym.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.strings.len()
    }
}
