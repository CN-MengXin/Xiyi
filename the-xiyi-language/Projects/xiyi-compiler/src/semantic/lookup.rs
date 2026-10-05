// src/semantic/lookup.rs
use std::collections::{HashMap, HashSet};
use crate::ast::*;
use super::check_program::TypeChecker;

// 存一个 implement 块里的某个方法：方法本身的定义，加上这个 implement 块
// 的 target_type（比如 `implement<T> Option<T> { fn is_some(self) -> bool }`
// 里的 `Option<T>`，里面的 T 会是 Type::TypeParam("T")）。调用点要靠
// target_type 把 receiver 的具体类型（比如 Option<i32>）跟方法签名里的
// 类型变量对上号。
//
// 结构体本身要 pub（check_program.rs 的 `methods` 字段类型要点名它），
// 但 fn_def/target_type 这两个字段不需要 pub——真正读写它们的代码
// （register_impl、check_method_call、check_qualified_static_call）
// 都在这个文件里，没有其它文件需要直接摸这两个字段。
#[derive(Clone)]
pub struct MethodInfo {
    pub(super) fn_def: FnDef,
    pub(super) target_type: Type,
    // 新增：这个方法所属 implement 块自己的 where_clause（比如
    // `implement<T> Vec<T> where T: Clone + Hash` 里的 `T: Clone + Hash`）。
    // 调用点（check_method_info）在 receiver/generic_args 把 T 具体绑定
    // 成某个类型之后，要靠这份 where_clause 反查"这个具体类型是不是
    // 真的实现了 Clone 和 Hash"——这是 receiver 约束校验能落地的前提，
    // 不存这份信息，调用点就无从知道该验证哪些约束。
    pub(super) where_clause: Vec<WhereClause>,
}

impl TypeChecker {
    // 从一个类型里提取"用来查 methods 表的 key"——Struct/Enum/Generic 都是
    // 拿类型名本身当 key（Generic 也只取名字，不含泛型实参，因为同一个
    // `implement<T> Option<T>` 要覆盖所有 Option<具体类型>，不分开存）。
    // 其他类型（Tensor、I32 这些内置标量）暂时没有方法表，返回 None，
    // 调用点会退回旧的兜底行为。
    pub fn type_key_for_impl_target(ty: &Type) -> Option<String> {
        match ty {
            Type::Struct(name) => Some(name.clone()),
            Type::Enum(name) => Some(name.clone()),
            Type::Generic(name, _) => Some(name.clone()),
            _ => None,
        }
    }
    
    // ===== 把一个 implement 块登记进方法表 =====
    // 从 check_program.rs 第一遍遍历的 `Item::Implement(imp) => { ... }`
    // 分支搬过来。
    //
    // 这个函数只负责"登记"，不做任何依赖其它 Item 是否已经出现过的
    // 校验——`imp.interface_name`/`imp.where_clause` 里引用的 interface
    // 是否真的存在，交给 check_program.rs::check_item 阶段的
    // check_implement_bounds（第二遍，此时所有 Item::Interface 都已经
    // 在第一遍里注册完毕，不用担心文件里 `implement Foo for Bar` 写在
    // `interface Foo { ... }` 前面这种合法的前向引用被误判）。这里只做
    // 两件纯数据登记的事：方法表本身，以及"这个类型实现了哪个
    // interface"这张表（interface_impls）——两者都不依赖其它 Item 是否
    // 已注册，随时登记都是安全的。
    pub fn register_impl(&mut self, imp: &ImplementDef) -> Result<(), String> {
        let Some(key) = Self::type_key_for_impl_target(&imp.target_type) else {
            return Ok(());
        };
        let entry = self.methods.entry(key.clone()).or_insert_with(HashMap::new);
        for fn_def in &imp.functions {
            if entry.contains_key(&fn_def.name) {
                return Err(format!(
                    "error[IMP001]: conflicting implementations of method `{}` for type `{}`",
                    fn_def.name, key
                ));
            }
            entry.insert(
                fn_def.name.clone(),
                MethodInfo {
                    fn_def: fn_def.clone(),
                    target_type: imp.target_type.clone(),
                    where_clause: imp.where_clause.clone(),
                },
            );
        }
        // 关键新增：`implement Interface for Type` 形式（interface_name
        // 非空）时，把"这个类型实现了这个 interface"记进
        // interface_impls，供别处（目前是 check_where_clause_bounds）
        // 查询"某个具体类型是否满足某个 interface 约束"。固有实现
        // （`implement Type { ... }`，interface_name 为 None）不涉及
        // interface，不登记。
        if let Some(interface_name) = &imp.interface_name {
            self.interface_impls
                .entry(key)
                .or_insert_with(HashSet::new)
                .insert(interface_name.clone());
        }
        Ok(())
    }

    // ===== 校验一个 implement 块自身声明的约束是否"指名有实据" =====
    // 从 check_program.rs::check_item 的 Item::Implement 分支调用，属于
    // 第二遍（此时所有 Item::Interface 都已经在第一遍注册完毕）。
    // 这里只检查"名字是否存在"，不检查"target_type 是否真的满足这些
    // 约束"——后者是每一次具体调用点的事（不同的调用点，泛型参数可能
    // 绑定成不同的具体类型，约束是否满足因调用而异，不是 impl 块声明时
    // 一次性能确定的），落在下面的 check_where_clause_bounds 里。
    pub fn check_implement_bounds(&self, imp: &ImplementDef) -> Result<(), String> {
        if let Some(interface_name) = &imp.interface_name {
            if !self.interfaces.contains_key(interface_name) {
                return Err(format!(
                    "undefined interface `{}` in `implement {} for {:?}`",
                    interface_name, interface_name, imp.target_type
                ));
            }
        }
        for wc in &imp.where_clause {
            for bound in &wc.bounds {
                if !self.interfaces.contains_key(bound) {
                    return Err(format!(
                        "undefined interface `{}` in where clause `{}: {}`",
                        bound, wc.type_name, bound
                    ));
                }
            }
        }
        Ok(())
    }

    // ===== receiver 约束校验：调用点验证具体类型是否满足 where_clause =====
    // check_method_info 在所有参数 unify 完、bindings 里的类型变量已经
    // 尽可能解出具体类型之后调用这个函数——早于这个时机调用，bindings
    // 可能还没填上该填的类型，会把"还没轮到检查"误判成"约束不满足"。
    fn check_where_clause_bounds(
        &self,
        where_clause: &[WhereClause],
        bindings: &HashMap<String, Type>,
    ) -> Result<(), String> {
        for wc in where_clause {
            // bindings 里没有这个类型变量：要么这次调用根本没用到它
            // （比如 `implement<T, U> Pair<T, U> where U: Default` 里的
            // U，如果这个方法签名完全不涉及 U），要么确实没能推导出来。
            // 两种情况都不该在这里报错——"推不出来"是 unify 阶段该管的
            // 事，这里只负责"推出来了但不满足约束"这一种情况。
            let Some(bound_ty) = bindings.get(&wc.type_name) else {
                continue;
            };
            // 内置标量/张量这类类型目前没有 interface_impls 可查（跟
            // register_impl 对它们的处理一致：type_key_for_impl_target
            // 返回 None 的类型，从一开始就不在这套"类型 -> 实现了哪些
            // interface"的登记范围内）。尽力而为，不在这里挡下调用——
            // 这是现有 impl 注册机制本身的覆盖范围问题，不是这次新增
            // 校验要解决的缺口。
            let Some(key) = Self::type_key_for_impl_target(bound_ty) else {
                continue;
            };
            for bound in &wc.bounds {
                let satisfies = self
                    .interface_impls
                    .get(&key)
                    .map_or(false, |set| set.contains(bound));
                if !satisfies {
                    return Err(format!(
                        "type `{:?}` does not satisfy bound `{}` (required by `where {}: {}`)",
                        bound_ty, bound, wc.type_name, bound
                    ));
                }
            }
        }
        Ok(())
    }

    // ===== 内建方法表：基础类型（str/整数等）身上"自带"的方法 =====
    // 这些类型是原语，没有对应的 implement 块能被查到（str 用户没法给它
    // implement，标准库也没这么做）——不补这张表，任何调用都会落进"查不到
    // 就默认 I32"的兜底，产出一个几乎总是错的类型。这里不是想做一套完整
    // 的基础类型方法体系，只覆盖标准库已经实际用到、会踩坑的这几个；
    // 以后再冒出新的（比如 i32.to_string()），照这个格式加一行就行。
    pub fn builtin_primitive_method_return_type(&self, receiver: &Type, method: &str) -> Option<Type> {
        match (receiver, method) {
            (Type::Str, "len") => Some(Type::U64),
            (Type::Str, "is_empty") => Some(Type::Bool),
            (Type::Str, "as_bytes") => Some(Type::Ref {
                mutable: false,
                inner: Box::new(Type::Slice(Box::new(Type::U8))),
            }),
            // .abs() 只对有符号数值类型有意义，返回类型跟接收者一致
            (t, "abs") if self.is_signed_numeric_type(t) => Some(t.clone()),
            _ => None,
        }
    }

    // ===== 以下是这次重构的核心：把方法调用/限定路径静态调用拆成
    // 几个可以独立测的小步骤，而不是两大段各自内联一遍相同逻辑 =====
    //
    // 重构动机：加 generic_args（`identity::<i32>(1)` 这种显式泛型实参）
    // 之前，check_method_call 和 check_qualified_static_call 各自内联了
    // 一遍"查表 -> unify 参数 -> 代入返回类型"，两份逻辑几乎一样却完全
    // 独立维护。直接在两处都插一遍"生成 bindings 时先塞 generic_args"
    // 只会让这种重复雪上加霜，所以借这次机会把公共步骤拆出来共用。

    // 只查表，不做任何 unify/校验——纯粹的"这个 key 下有没有这个方法"。
    fn lookup_method(&self, type_key: &str, method_name: &str) -> Option<MethodInfo> {
        self.methods
            .get(type_key)
            .and_then(|m| m.get(method_name))
            .cloned()
    }

    // 查表 + 把显式泛型实参绑进一张新的 bindings 表。
    // 返回值用 Result<Option<...>, String> 是因为这里天然叠着两层可能：
    // 方法根本没注册过（Ok(None)，调用方应该去试下一条路径，不是错误）；
    // 方法找到了，但用户写的 `::<...>` 数量跟方法自己声明的泛型参数
    // 个数对不上（Err，这才是真正的错误，不该被 Ok(None) 悄悄吞掉、
    // 让调用方误以为"这个方法不存在"再报一个文不对题的 undefined）。
    fn resolve_method(
        &self,
        type_key: &str,
        method_name: &str,
        generic_args: &[Type],
    ) -> Result<Option<(MethodInfo, HashMap<String, Type>)>, String> {
        let Some(method_info) = self.lookup_method(type_key, method_name) else {
            return Ok(None);
        };
        let mut bindings = HashMap::new();
        Self::bind_generic_args(
            &method_info.fn_def.generic_params,
            generic_args,
            &mut bindings,
        )?;
        Ok(Some((method_info, bindings)))
    }

    // 用给定的 bindings（可能已经预置了 receiver unify 出来的绑定、或者
    // 显式 generic_args 绑定）检查参数列表（不含 self），代入返回类型。
    // check_method_call 和 check_qualified_static_call 走到这一步唯一的
    // 区别只是"bindings 是怎么来的"，参数怎么核对、返回类型怎么代入是
    // 完全一样的，所以抽成这一个共享方法。
    fn check_method_info(
        &mut self,
        method_info: &MethodInfo,
        mut bindings: HashMap<String, Type>,
        args: &[CallArg],
    ) -> Result<Type, String> {
        let non_self_params: Vec<&Param> = method_info
            .fn_def
            .params
            .iter()
            .filter(|p| p.name != "self")
            .collect();

        if non_self_params.len() != args.len() {
            return Err(format!(
                "`{}` expects {} argument(s), got {}",
                method_info.fn_def.name,
                non_self_params.len(),
                args.len()
            ));
        }

        for (param, arg) in non_self_params.iter().zip(args) {
            let arg_ty = self.check_call_arg(arg)?;
            if !self.unify_type(&arg_ty, &param.ty, &mut bindings) {
                return Err(format!(
                    "type mismatch in call to `{}`: parameter `{}` expected {:?}, got {:?}",
                    method_info.fn_def.name, param.name, param.ty, arg_ty
                ));
            }
        }

        // 关键新增：所有参数都 unify 完、bindings 里该解出来的类型变量
        // 都已经解出来之后，校验这个 implement 块自己声明的
        // where_clause——`implement<T> Vec<T> where T: Clone` 被调用
        // 到具体的 `Vec<Foo>` 时，Foo 必须真的实现了 Clone。放在参数
        // unify 之后而不是之前，是因为 bindings 在 unify 完成前可能
        // 还没填上该填的绑定（比如 T 只出现在某个参数类型里，不在
        // receiver 里），过早检查会把"还没轮到检查"误判成"不满足"。
        self.check_where_clause_bounds(&method_info.where_clause, &bindings)?;

        Ok(method_info
            .fn_def
            .return_type
            .clone()
            .map(|ret| self.substitute_type(&ret, &bindings))
            .unwrap_or(Type::Unit))
    }

    // 只检查一批参数、不参与任何 unify——查不到方法时的兜底路径，以及
    // "先查过 receiver、剩下的参数照样得过一遍类型检查"这两种场景都要用。
    fn check_call_args(&mut self, args: &[CallArg]) -> Result<(), String> {
        for arg in args {
            self.check_call_arg(arg)?;
        }
        Ok(())
    }

    // ===== 方法调用尝试之一：内建基础类型方法表（str.len() 之类） =====
    // 内建方法没有泛型参数、也不参与 self.methods 里的 unify 流程，只是
    // "认出来了就返回类型、其余参数照常检查"。返回 Ok(None) 表示"这不是
    // 一个内建方法"，调用方应该继续试下一条路径，不是错误。
    fn try_builtin_primitive_method(
        &mut self,
        receiver_ty: &Type,
        method: &str,
        rest: &[CallArg],
    ) -> Result<Option<Type>, String> {
        let receiver_stripped = self.strip_privacy(receiver_ty);
        let receiver_base = self.strip_ref(&receiver_stripped);
        let Some(ret_ty) = self.builtin_primitive_method_return_type(&receiver_base, method) else {
            return Ok(None);
        };
        self.check_call_args(rest)?;
        Ok(Some(ret_ty))
    }

    // ===== 方法调用尝试之二：self.methods 里登记过的 implement 方法 =====
    // 先用 receiver 的具体类型（比如 Option<i32>）跟 implement 块的
    // target_type（Option<T>）unify，解出 T 绑定成了什么，再跟
    // resolve_method 从 generic_args 绑出来的那份合并，一起用于参数/
    // 返回类型的检查——receiver 推出来的绑定和用户显式写的绑定共享
    // 同一张表，任何一边先绑定过的类型变量，另一边 unify 的时候仍然会
    // 校验一致性，不会互相覆盖。
    fn try_registered_method(
        &mut self,
        receiver_ty: &Type,
        method: &str,
        generic_args: &[Type],
        rest: &[CallArg],
    ) -> Result<Option<Type>, String> {
        let receiver_stripped = self.strip_privacy(receiver_ty);
        let Some(key) = Self::type_key_for_impl_target(&receiver_stripped) else {
            return Ok(None);
        };
        let Some((method_info, mut bindings)) =
            self.resolve_method(&key, method, generic_args)?
        else {
            return Ok(None);
        };
        if !self.unify_type(&receiver_stripped, &method_info.target_type, &mut bindings) {
            return Err(format!(
                "receiver type {:?} does not match implement target {:?} for method `{}`",
                receiver_ty, method_info.target_type, method
            ));
        }
        Ok(Some(self.check_method_info(&method_info, bindings, rest)?))
    }

    // ===== 方法调用：查 methods 表，按方法自己的签名（含泛型）检查 =====
    // 依次尝试"内建基础类型方法" -> "self.methods 里注册过的方法"，
    // 两条路都没命中就退回旧的兜底行为（只检查参数、返回 I32），不阻断
    // 这类调用——receiver 是内置标量/张量类型，或者方法确实没被任何
    // implement 块定义过，都会落到这里。
    pub fn check_method_call(
        &mut self,
        func: &str,
        generic_args: &[Type],
        args: &[CallArg],
    ) -> Result<Type, String> {
        let Some((receiver, rest)) = args.split_first() else {
            return Err("method call requires a receiver".to_string());
        };
        let receiver_ty = self.check_call_arg(receiver)?;

        if let Some(ty) = self.try_builtin_primitive_method(&receiver_ty, func, rest)? {
            return Ok(ty);
        }
        if let Some(ty) = self.try_registered_method(&receiver_ty, func, generic_args, rest)? {
            return Ok(ty);
        }

        self.check_call_args(rest)?;
        Ok(Type::I32)
    }

    // ===== 限定路径静态调用：`TypeName::func(args)`，没有 self 接收者 =====
    // 跟 check_method_call 共享 resolve_method/check_method_info 这两步，
    // 唯一区别是没有 receiver，不需要先做"receiver 类型 vs impl 块
    // target_type"的合一，所有形参直接按位置对实参。
    pub fn check_qualified_static_call(
        &mut self,
        type_name: &str,
        func_name: &str,
        generic_args: &[Type],
        args: &[CallArg],
        expected: Option<&Type>,
    ) -> Result<Type, String> {
        let (method_info, mut bindings) = self
            .resolve_method(type_name, func_name, generic_args)?
            .ok_or_else(|| format!("undefined function `{}::{}`", type_name, func_name))?;

        // 用外部期望类型预置绑定。Vec::new()/Vec::with_capacity() 这类
        // "返回值带泛型参数 T，但参数列表里完全看不到 T"的静态函数，光靠
        // 参数没法推出 T 到底是什么——这里从"这个返回值将被用在什么类型
        // 的位置"反推。尽力而为，unify 不上就放着不报错——真正的类型
        // 不匹配，交给外层（比如 check_struct_init 的字段比较）在真正
        // 比较的时候报错，不在这一步抢先报错。
        if let (Some(expected_ty), Some(ret_ty)) = (expected, &method_info.fn_def.return_type) {
            self.unify_type(expected_ty, ret_ty, &mut bindings);
        }

        self.check_method_info(&method_info, bindings, args)
    }
}
