// src/semantic/check_program.rs
use std::collections::{HashMap, HashSet};
use crate::ast::*;
use crate::hir;
use crate::hir_builder;
use super::lookup::MethodInfo;

pub struct TypeChecker {
    pub scopes: Vec<HashMap<String, Type>>,
    pub structs: HashMap<String, StructDef>,
    pub enums: HashMap<String, EnumDef>,
    pub consts: HashMap<String, Type>,
    pub functions: HashMap<String, FnDef>,
    // 类型名 -> 方法名 -> MethodInfo。之前 implement 块从来没被收集过，
    // 方法调用只能靠"查不到就返回 I32"这种兜底，这张表补上之后，方法调用
    // 才能真正按方法自己的签名做类型检查。
    // 关键修复（拆文件引入）：这个字段原来是私有的（没有 pub）——当
    // TypeChecker 的方法全部挤在同一个 impl 块、同一个文件里时无所谓，
    // 但现在 lookup.rs（方法表的读写逻辑）、check_expr.rs（方法调用/
    // 限定路径静态调用）都要在各自的文件里通过 `self.methods` 访问它，
    // 分属不同的子模块（semantic::check_program vs semantic::lookup /
    // semantic::check_expr 是平级模块，不是父子关系），私有字段过不了
    // 编译。这个结构体上其它字段本来就都是 pub，这里补 pub 只是让它
    // 跟兄弟字段保持一致，不是新引入的例外。
    pub methods: HashMap<String, HashMap<String, MethodInfo>>,
    pub in_model: bool,
    pub fn_stack: Vec<String>,
    pub model_names: HashSet<String>,
    pub model_return_types: HashMap<String, Type>,
    pub model_sensitivities: HashMap<String, f64>,
    pub current_self_type: Option<Type>,
    // 新增：当前正在检查的函数的声明返回类型，供 Stmt::Return 用来给
    // return 语句里的表达式（比如裸 Err(...)）传递期望类型提示。
    pub current_return_type: Option<Type>,
    pub expr_types: HashMap<usize, Type>,
    // 新增：当前处于第几层循环（While/For/Loop 每进入一层 body 就 +1，
    // 退出时 -1）。Break/Continue 检查这个值是否为 0 来判断"是不是写在
    // 循环外面"——之前 Stmt::Break 完全不检查这件事，任何位置写
    // `break;` 都会被放行。
    pub loop_depth: usize,
    // 新增：interface 名字 -> 定义。之前 Item::Interface 落到 `_ => {}`，
    // 完全没有登记；where_clause 约束检查（`where T: Clone` 里的 Clone
    // 是不是一个真实存在的 interface）第一步就需要能查到"这个名字是不是
    // 已知 interface"，所以补上这张表。存完整定义（不是只存名字的
    // HashSet）是为将来可能需要校验"某个 impl 块是否把 interface 声明的
    // 每个方法都实现了"这类更深的检查留出扩展空间，即使这一轮暂时用不上。
    pub interfaces: HashMap<String, InterfaceDef>,
    // 新增：类型 key（跟 methods 表用的是同一套 key，见
    // lookup.rs::type_key_for_impl_target）-> 这个类型通过
    // `implement Interface for Type` 实际实现过的 interface 名字集合。
    // where_clause 约束检查的第二步——"receiver 的具体类型是否满足
    // `where T: Bound` 里的 Bound"——靠查这张表回答。
    pub interface_impls: HashMap<String, HashSet<String>>,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            structs: HashMap::new(),
            enums: HashMap::new(),
            consts: HashMap::new(),
            functions: HashMap::new(),
            methods: HashMap::new(),
            in_model: false,
            fn_stack: Vec::new(),
            model_names: HashSet::new(),
            model_return_types: HashMap::new(),
            model_sensitivities: HashMap::new(),
            current_self_type: None,
            current_return_type: None,
            expr_types: HashMap::new(),
            loop_depth: 0,
            interfaces: HashMap::new(),
            interface_impls: HashMap::new(),
        }
    }

    // 关键重构：以前 check_program 里直接堆着两个几乎一样结构的
    // `for item in &program.items { match item { ... } }`——第一遍收集
    // 签名、第二遍检查函数体，这个"先收集再检查"的两阶段结构本身是
    // 对的（函数体互相调用前必须先把所有签名收齐），但两次 match 各自
    // 罗列一遍所有 Item 变体，以后加一个新的 Item 种类（比如
    // Item::TraitDef），两处都要记得加，忘了一处也不会立刻报错——
    // 只会在某个用到新变体的地方悄悄走空分支。拆成 collect_item（只
    // 登记、不检查）和 check_item（只检查、不登记）两个方法后，新增
    // 一种 Item 只需要各自去这两个方法里补一条 match 分支，属于
    // "看代码就知道要不要补"的地方，不再是两处分散、容易漏掉的重复。
    pub fn check_program(&mut self, program: &Program) -> Result<hir::HirProgram, String> {
        for item in &program.items {
            self.collect_item(item)?;
        }

        for item in &program.items {
            self.check_item(item)?;
        }

        let hir = hir_builder::HirBuilder::build(program, &self.expr_types)?;
        Ok(hir)
    }

    // ===== 第一遍遍历：只登记签名，不检查函数体 =====
    fn collect_item(&mut self, item: &Item) -> Result<(), String> {
        match item {
            Item::FnDef(f) => {
                self.functions.insert(f.name.clone(), f.clone());
            }
            Item::StructDef(s) => {
                self.structs.insert(s.name.clone(), s.clone());
            }
            Item::EnumDef(e) => {
                self.enums.insert(e.name.clone(), e.clone());
            }
            Item::ConstDef(c) => {
                self.consts.insert(c.name.clone(), c.ty.clone());
            }
            // model 相关的收集逻辑（登记 model 名字、把 model 的字段
            // 转成一个同名 struct、记录 forward 的返回类型/sensitivity
            // 属性）挪到了 check_model.rs 的 collect_model_def 里，这里
            // 只是委托调用。collect_model_def 现在会返回 Err（比如
            // #[sensitivity(const = ...)] 里写了个解析不出来的有理数），
            // 用 `?` 照常传播，不能吞掉。
            Item::ModelDef(m) => self.collect_model_def(m)?,
            // 关键新增：proto 的变体名不允许重复（`match msg { ... }`
            // 靠变体名区分分支，撞名会让穷尽性检查/分支分派本身失去
            // 意义）。之前这里完全是空分支，撞名的 proto 定义能一路
            // 无声无息地通过检查。
            Item::ProtoDef(p) => {
                let mut seen = HashSet::new();
                for v in &p.variants {
                    if !seen.insert(v.name.clone()) {
                        return Err(format!(
                            "duplicate variant `{}` in proto `{}`", v.name, p.name
                        ));
                    }
                }
            }
            // 关键新增：interface 内的方法名同样不允许重复——理由跟
            // proto 变体一样，重复的方法名会让"这个 interface 到底
            // 声明了几个方法"这件事本身变得歧义。
            //
            // 名字去重之后把定义本身存进 self.interfaces——这一步只是
            // 登记，不做任何"这个 interface 是不是被正确实现"之类的
            // 校验。真正用到这张表的 where_clause / interface_name
            // 存在性校验放在 check_item 的第二遍（见下面 check_item 里
            // Item::Implement 分支新增的 check_implement_bounds），而
            // 不是放在这里或 register_impl 里——原因是 collect_item 对
            // 所有 Item 是按文件顺序单趟扫过去的，`implement Foo for Bar`
            // 完全可能在文件里写在 `interface Foo { ... }` 前面；如果
            // 在 collect_item/register_impl 这个"第一遍只登记"的阶段就
            // 校验"Foo 是不是已注册的 interface"，会把这种合法的前向
            // 引用误判成"未定义的 interface"。struct/enum 的存在性检查
            // 现有代码本来就是放在 check_item（第二遍）做的
            // （check_type.rs 的 resolve_type 只在 check_item 触发的
            // check_func/check_param_type 里被调用），这里跟已有架构
            // 保持一致，而不是另开一种"registration 阶段夹带校验"的
            // 先例。
            Item::Interface(i) => {
                let mut seen = HashSet::new();
                for m in &i.methods {
                    if !seen.insert(m.name.clone()) {
                        return Err(format!(
                            "duplicate method `{}` in interface `{}`", m.name, i.name
                        ));
                    }
                }
                self.interfaces.insert(i.name.clone(), i.clone());
            }
            Item::Use(_) => {}
            // implement 块登记进方法表这件事挪到了 lookup.rs 的
            // register_impl 里，这里只是委托调用。
            //
            // register_impl 现在会检测同名方法的重叠实现（规范
            // §3.0.6，error[IMP001]），撞名时返回 Err 而不是像以前
            // 那样静默覆盖。这里用 `?` 把错误原样传播出去。
            Item::Implement(imp) => self.register_impl(imp)?,
            _ => {}
        }
        Ok(())
    }

    // ===== 第二遍遍历：真正检查函数体/常量初始值 =====
    fn check_item(&mut self, item: &Item) -> Result<(), String> {
        match item {
            Item::FnDef(f) => self.check_func(f)?,
            Item::ConstDef(c) => {
                let value_type = self.check_expr(&c.value)?;
                if !self.types_equal(&value_type, &c.ty) {
                    return Err(format!("const type mismatch: expected {:?}, got {:?}", c.ty, value_type));
                }
            }
            // model 块本身"必须有 forward"、检查每个函数体这些逻辑，
            // 挪到了 check_model.rs 的 check_model_def 里。
            Item::ModelDef(m) => self.check_model_def(m)?,
            Item::ProtoDef(_) => {}
            Item::Use(_) => {}
            // implement 块里的方法体在这里真正被检查一遍（`self` 的
            // 类型设成这个 implement 块的 target_type，跟 ModelDef 那边
            // 的处理方式一致）。
            //
            // 关键新增：先校验这个 implement 块自身声明的约束是否成立
            // （`interface_name` 是不是已知 interface、`where_clause`
            // 里每个 bound 是不是已知 interface）——放在方法体检查
            // 之前，理由不是必须的顺序（两者互不依赖），只是"先确认
            // 这个 impl 块的声明本身站得住脚，再去检查它内部的方法体"
            // 更符合阅读顺序。这一步只校验"名字是否存在"，不校验
            // "target_type 是否真的满足这些约束"——那是每一次具体调用
            // 点的事（lookup.rs::check_method_info 里新增的
            // check_where_clause_bounds），不是 impl 块声明时就能确定
            // 的（受 T 约束的是调用点传入的具体类型，不是 impl 块本身）。
            Item::Implement(imp) => {
                self.check_implement_bounds(imp)?;
                self.current_self_type = Some(imp.target_type.clone());
                for fn_def in &imp.functions {
                    self.check_func(fn_def)?;
                }
                self.current_self_type = None;
            }
            _ => {}
        }
        Ok(())
    }
}
