// src/semantic/check_path.rs
//
// 集中"名字 -> 实体"这一类存在性/身份判断：类型名到底是 struct 还是
// enum、限定路径的变体查找、限定路径静态方法是否存在、裸变体消歧义。
// 之前这几件事散落在 check_stmt.rs / check_model.rs / check_expr.rs /
// check_pattern.rs 里，每处各自查一遍 self.structs / self.enums /
// self.methods，出错信息、查找顺序都容易在某一处漏改、其它处忘改。
//
// 不放进这个文件（保留在原处的理由见各自文件的注释）：
//   - hunt.rs::hunt_symbol：裸标识符查变量/常量，是作用域查找，不是
//     "类型名/路径名"查找。
//   - lookup.rs::check_method_call / check_qualified_static_call：
//     这两个是完整调用检查（unify 参数、代入返回类型的泛型），不是
//     单纯的存在性判断，跟这个文件"只回答是什么/在不在"的定位不同。
//   - helpers.rs::resolve_import：文件系统层的模块路径解析，跟符号表
//     里的类型名/路径名是两回事。

use crate::ast::*;
use super::check_program::TypeChecker;

/// 具名类型的身份。
///
/// `Enum` 分支**不携带 `EnumDef`**：目前没有任何调用点需要枚举定义——
/// 需要它的时候（遍历字段）一定是 struct。带上 `EnumDef` 只会让
/// `resolve_type_name` 在返回"是枚举"时白克隆一份定义，而所有调用点
/// 都会立刻把它丢掉。真需要枚举定义的场景（如果有），调用点自己
/// `self.enums.get(name)` 就行。
pub enum ResolvedTypeName {
    Struct(StructDef),
    Enum,
    Unknown,
}

/// 裸变体名（`Ok` / `Err` / `Some` / `None`）的消歧结果。
pub enum BareVariantOutcome {
    Unique(String),
    /// 按名字排序，保证错误信息确定（不依赖 HashMap 迭代顺序）。
    Ambiguous(Vec<String>),
    NotFound,
}

impl TypeChecker {
    // ===== 类型名 =====

    /// 查名字是 struct 还是 enum。`Struct` 分支携带定义（调用点通常
    /// 需要遍历字段）；`Enum` 只回答身份、不携带任何数据（见上面
    /// `ResolvedTypeName` 的说明）。
    pub fn resolve_type_name(&self, name: &str) -> ResolvedTypeName {
        if let Some(s) = self.structs.get(name) {
            return ResolvedTypeName::Struct(s.clone());
        }
        if self.enums.contains_key(name) {
            return ResolvedTypeName::Enum;
        }
        ResolvedTypeName::Unknown
    }

    /// 从 `Type` 里解析出正确的具名类型：`Type::Struct(name)` 实际是
    /// enum 时改写成 `Type::Enum(name)`，反之亦然。其它变体原样返回。
    pub fn resolve_type(&self, ty: &Type) -> Result<Type, String> {
        match ty {
            Type::Struct(name) => {
                if self.structs.contains_key(name) {
                    Ok(Type::Struct(name.clone()))
                } else if self.enums.contains_key(name) {
                    Ok(Type::Enum(name.clone()))
                } else {
                    Err(format!("undefined type: {}", name))
                }
            }
            Type::Enum(name) => {
                if self.enums.contains_key(name) {
                    Ok(Type::Enum(name.clone()))
                } else if self.structs.contains_key(name) {
                    Ok(Type::Struct(name.clone()))
                } else {
                    Err(format!("undefined type: {}", name))
                }
            }
            other => Ok(other.clone()),
        }
    }

    /// 解析出一个 `StructDef`。
    /// - 是 struct：返回定义；
    /// - 是 enum：报错 `"`X` is an enum, not a struct"`；
    /// - 都不是：报错 `"undefined struct: X"`。
    pub fn resolve_struct(&self, name: &str) -> Result<StructDef, String> {
        if let Some(s) = self.structs.get(name) {
            Ok(s.clone())
        } else if self.enums.contains_key(name) {
            Err(format!("`{}` is an enum, not a struct", name))
        } else {
            Err(format!("undefined struct: {}", name))
        }
    }

    // ===== 限定路径 =====

    /// `EnumName::Variant`——查变体定义，返回 owned。只用在**需要拿到
    /// `EnumVariant` 定义**的调用点；只需要布尔结果的调用点用
    /// `has_variant`，避免白克隆。
    pub fn resolve_variant_in(
        &self,
        enum_name: &str,
        variant_name: &str,
    ) -> Option<EnumVariant> {
        self.enums
            .get(enum_name)?
            .variants
            .iter()
            .find(|v| v.name == variant_name)
            .cloned()
    }

    /// 只判断"枚举 X 有没有变体 Y"，不克隆 `EnumVariant`。
    pub fn has_variant(&self, enum_name: &str, variant_name: &str) -> bool {
        self.enums
            .get(enum_name)
            .map_or(false, |e| e.variants.iter().any(|v| v.name == variant_name))
    }

    /// `TypeName::func` 是不是方法表里注册的静态方法。
    pub fn has_qualified_static(&self, type_name: &str, func_name: &str) -> bool {
        self.methods
            .get(type_name)
            .map_or(false, |m| m.contains_key(func_name))
    }

    // ===== 裸变体消歧义 =====

    /// 在所有已注册的枚举里找"哪个恰好有这个名字的变体"。
    /// 找不到 → `NotFound`；恰好一个 → `Unique`；多个 → `Ambiguous`。
    pub fn resolve_bare_variant(&self, name: &str) -> BareVariantOutcome {
        let matches: Vec<&str> = self
            .enums
            .iter()
            .filter(|(_, e)| e.variants.iter().any(|v| v.name == name))
            .map(|(n, _)| n.as_str())
            .collect();

        match matches.as_slice() {
            [] => BareVariantOutcome::NotFound,
            [only] => BareVariantOutcome::Unique((*only).to_string()),
            _ => {
                let mut sorted: Vec<String> =
                    matches.iter().map(|s| (*s).to_string()).collect();
                sorted.sort_unstable();
                BareVariantOutcome::Ambiguous(sorted)
            }
        }
    }
}
