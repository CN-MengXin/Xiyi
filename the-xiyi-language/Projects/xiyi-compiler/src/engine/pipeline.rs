// src/engine/pipeline.rs
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::ast::{Item, Program};
use crate::codegen::Codegen;
use crate::elaborate::Elaborate;
use crate::hir::HirProgram;
use crate::mir::MirProgram;
use crate::mir_builder::MirBuilder;
use crate::semantic::TypeChecker;
use crate::syntactic::Parser;
use crate::{borrow, control, monomorphic, simplify};

use super::error::Error;

/// 有名字的顶层 Item 都能通过这个 trait 取名字（use/impl 返回 None）。
///
/// 相比自由函数 `item_name(&item)`，`item.name()` 可读性更好，
/// 也方便以后给别的类型实现 `Named`。
pub trait Named {
    fn name(&self) -> Option<&str>;
}

impl Named for Item {
    fn name(&self) -> Option<&str> {
        match self {
            Item::FnDef(f) => Some(&f.name),
            Item::StructDef(s) => Some(&s.name),
            Item::EnumDef(e) => Some(&e.name),
            Item::ConstDef(c) => Some(&c.name),
            Item::ModelDef(m) => Some(&m.name),
            Item::ProtoDef(p) => Some(&p.name),
            Item::Interface(iface) => Some(&iface.name),
            Item::Use(_) => None,
            Item::Implement(_) => None,
        }
    }
}

/// 从文件路径取出用作报错定位的模块标签（去掉扩展名的文件名）。
fn module_label(path: &Path) -> String {
    match path.file_stem().and_then(|s| s.to_str()) {
        Some(stem) => stem.to_string(),
        None => "<unknown>".to_string(),
    }
}

/// 收集目录下所有 .xiyi 文件，按文件名排序，lib.xiyi 永远排最后。
///
/// 顺序确定性对跨平台复现很重要，所以这个小函数值得单独存在。
/// 从 load_stdlib 主体抽出来，load_stdlib 只剩「逐个 parse + 查重」。
fn collect_module_files(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => return Err(Error::io(dir.to_path_buf(), e)),
    };

    let mut modules: Vec<PathBuf> = Vec::new();
    let mut lib: Option<PathBuf> = None;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => return Err(Error::io(dir.to_path_buf(), e)),
        };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("xiyi") {
            continue;
        }
        if path.file_name().and_then(|s| s.to_str()) == Some("lib.xiyi") {
            lib = Some(path);
        } else {
            modules.push(path);
        }
    }
    modules.sort();
    if let Some(lib) = lib {
        modules.push(lib);
    }
    Ok(modules)
}

/// 加载标准库：读取 `<stdlib_path>/xiyi-core/src` 下所有 `.xiyi` 文件并解析，
/// lib.xiyi 单独放到最后加载，同时做跨文件命名冲突检测。
pub fn load_stdlib(stdlib_path: &Path) -> Result<Vec<Item>, Error> {
    let core_src = stdlib_path.join("xiyi-core").join("src");

    if !core_src.is_dir() {
        return Err(Error::Usage(format!(
            "stdlib source directory not found: {}",
            core_src.display()
        )));
    }

    let module_files = collect_module_files(&core_src)?;
    if module_files.is_empty() {
        // 允许空标准库，返回空 Vec
        return Ok(Vec::new());
    }

    let mut all_items: Vec<Item> = Vec::new();
    // 记录每个已定义名字来自哪个模块文件，用于冲突检测和报错定位
    let mut defined_in: HashMap<String, String> = HashMap::new();

    for file_path in &module_files {
        let label = module_label(file_path);

        let source = match fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => return Err(Error::io(file_path.to_path_buf(), e)),
        };

        let mut parser = Parser::new(&source);
        let program = match parser.parse_program() {
            Ok(p) => p,
            Err(e) => {
                return Err(Error::Parse {
                    file: label,
                    message: e.to_string(),
                });
            }
        };

        for item in program.items {
            if let Some(name) = item.name() {
                if let Some(prev_module) = defined_in.get(name) {
                    return Err(Error::DuplicateDef {
                        name: name.to_string(),
                        first: prev_module.clone(),
                        second: label.clone(),
                    });
                }
                defined_in.insert(name.to_string(), label.clone());
            }
            all_items.push(item);
        }
    }

    Ok(all_items)
}

/// 合并标准库与用户程序，若用户定义的名字与标准库冲突则报错。
///
/// user_filename 只用于拼错误信息，用 &Path 而不是 &str，
/// 跟 CompilerConfig::filename 现在的类型（PathBuf）保持一致，
/// 调用方不用在 String/PathBuf 之间转来转去。
pub fn merge_with_conflict_check(
    stdlib_items: Vec<Item>,
    user_items: Vec<Item>,
    user_filename: &Path,
) -> Result<Program, Error> {
    let mut stdlib_names: HashMap<String, ()> = HashMap::new();
    for item in &stdlib_items {
        if let Some(name) = item.name() {
            stdlib_names.insert(name.to_string(), ());
        }
    }

    for item in &user_items {
        if let Some(name) = item.name() {
            if stdlib_names.contains_key(name) {
                return Err(Error::DuplicateDef {
                    name: name.to_string(),
                    first: "标准库".to_string(),
                    second: user_filename.display().to_string(),
                });
            }
        }
    }

    let mut all_items = stdlib_items;
    all_items.extend(user_items);
    Ok(Program { items: all_items })
}

/// 编译流水线的真实状态。
///
/// `Option<T>` 只负责表示阶段产物是否存在；真正决定 API 是否可调用的
/// 是这个状态值。这样就不会出现「MIR 存在，但 cleanup / borrow_check
/// 实际上没有执行」之类的假状态。
enum CompilerStage {
    Empty,
    Parsed,
    Typed,
    Elaborated,
    Lowered,
    Cleaned,
    BorrowChecked,
    Optimized,
}

impl CompilerStage {
    fn name(&self) -> &'static str {
        match self {
            CompilerStage::Empty => "empty",
            CompilerStage::Parsed => "parsed",
            CompilerStage::Typed => "typed",
            CompilerStage::Elaborated => "elaborated",
            CompilerStage::Lowered => "lowered",
            CompilerStage::Cleaned => "cleaned",
            CompilerStage::BorrowChecked => "borrow_checked",
            CompilerStage::Optimized => "optimized",
        }
    }
}

impl PartialEq for CompilerStage {
    fn eq(&self, other: &CompilerStage) -> bool {
        match (self, other) {
            (CompilerStage::Empty, CompilerStage::Empty)
            | (CompilerStage::Parsed, CompilerStage::Parsed)
            | (CompilerStage::Typed, CompilerStage::Typed)
            | (CompilerStage::Elaborated, CompilerStage::Elaborated)
            | (CompilerStage::Lowered, CompilerStage::Lowered)
            | (CompilerStage::Cleaned, CompilerStage::Cleaned)
            | (CompilerStage::BorrowChecked, CompilerStage::BorrowChecked)
            | (CompilerStage::Optimized, CompilerStage::Optimized) => true,
            _ => false,
        }
    }
}

impl Eq for CompilerStage {}

/// 阶段调用顺序错误。
///
/// `required` 是当前操作要求的前置阶段，`actual` 是流水线实际状态。
fn stage_order_error(stage: &str, required: &str, actual: &str) -> Error {
    Error::Usage(format!(
        "阶段顺序错误：无法执行 {}；当前阶段为 {}，需要先完成 {} \
         （顺序：parse → type_check → elaborate → lower → cleanup → \
          borrow_check → optimize → codegen）",
        stage, actual, required
    ))
}

/// 分阶段的编译流水线。
///
/// 每个转换阶段都有一个明确的前置状态，并且只有成功完成后才推进
/// `stage`。阶段失败时，旧阶段及其产物保持不变，因此调用方可以在
/// 修正外部条件后安全地重试该阶段。
///
/// `parse()` 是一个新的编译单元入口：无论当前处于哪个阶段，只要解析和
/// 标准库合并成功，就会原子地替换 AST 并清空所有下游产物。因此复用
/// `Compiler` 不会把上一份源程序的 HIR/MIR 带入下一次编译。
pub struct Compiler {
    stage: CompilerStage,
    program: Option<Program>,
    hir: Option<HirProgram>,
    mir: Option<MirProgram>,
}

impl Compiler {
    /// 显式构造，不依赖 `#[derive(Default)]`。
    pub fn new() -> Compiler {
        Compiler {
            stage: CompilerStage::Empty,
            program: None,
            hir: None,
            mir: None,
        }
    }

    /// 要求流水线当前正处于指定阶段。
    fn require_stage(
        &self,
        stage: &str,
        required: CompilerStage,
    ) -> Result<(), Error> {
        let actual = self.stage.name();
        if self.stage == required {
            Ok(())
        } else {
            Err(stage_order_error(stage, required.name(), actual))
        }
    }

    /// 阶段 1：读源码、读标准库、解析、合并，得到最终的 AST。
    ///
    /// 该函数可以作为一次新的编译入口重复调用。所有解析、标准库加载和
    /// 冲突检查都成功之后才修改 `self`，所以失败不会破坏当前有效状态。
    pub fn parse(
        &mut self,
        filename: &Path,
        stdlib_path: &Path,
    ) -> Result<&mut Compiler, Error> {
        let source = match fs::read_to_string(filename) {
            Ok(s) => s,
            Err(e) => return Err(Error::io(filename.to_path_buf(), e)),
        };

        let stdlib_items = load_stdlib(stdlib_path)?;

        let mut parser = Parser::new(&source);
        let user_program = match parser.parse_program() {
            Ok(p) => p,
            Err(e) => {
                return Err(Error::Parse {
                    file: filename.display().to_string(),
                    message: e.to_string(),
                });
            }
        };

        let program = merge_with_conflict_check(
            stdlib_items,
            user_program.items,
            filename,
        )?;

        // 到这里所有可能失败的工作已经完成。现在一次性提交新的 AST，
        // 同时使全部旧的下游产物失效。
        self.program = Some(program);
        self.hir = None;
        self.mir = None;
        self.stage = CompilerStage::Parsed;

        Ok(self)
    }

    /// 阶段 2a：类型检查，得到尚未展开语法糖的 HIR。
    pub fn type_check(&mut self) -> Result<&mut Compiler, Error> {
        self.require_stage("type_check", CompilerStage::Parsed)?;

        let program = match self.program.as_ref() {
            Some(p) => p,
            None => {
                // 理论上不可能：stage == Parsed 必须与 program == Some
                // 保持同步。保留防御性检查，避免未来修改时产生隐式 panic。
                return Err(stage_order_error(
                    "type_check",
                    "parsed",
                    self.stage.name(),
                ));
            }
        };

        let mut checker = TypeChecker::new();
        let hir = match checker.check_program(program) {
            Ok(h) => h,
            Err(e) => return Err(Error::Type(e.to_string())),
        };

        // 只有成功生成 HIR 后才推进状态。
        self.hir = Some(hir);
        self.mir = None;
        self.stage = CompilerStage::Typed;

        Ok(self)
    }

    /// 阶段 2b：展开语法糖（for、? 等）。
    pub fn elaborate(&mut self) -> Result<&mut Compiler, Error> {
        self.require_stage("elaborate", CompilerStage::Typed)?;

        let hir = match self.hir.as_ref() {
            Some(h) => h,
            None => {
                return Err(stage_order_error(
                    "elaborate",
                    "typed",
                    self.stage.name(),
                ));
            }
        };

        let elaborated = match Elaborate::elaborate(hir.clone()) {
            Ok(h) => h,
            Err(e) => return Err(Error::Elaborate(e.to_string())),
        };

        // Elaborate 失败时原 HIR 保持不变；成功后才提交新 HIR。
        self.hir = Some(elaborated);
        self.mir = None;
        self.stage = CompilerStage::Elaborated;

        Ok(self)
    }

    /// 阶段 3：把 HIR 下沉为 MIR（不做任何清理或检查）。
    pub fn lower(&mut self) -> Result<&mut Compiler, Error> {
        self.require_stage("lower", CompilerStage::Elaborated)?;

        let hir = match self.hir.as_ref() {
            Some(h) => h,
            None => {
                return Err(stage_order_error(
                    "lower",
                    "elaborated",
                    self.stage.name(),
                ));
            }
        };

        let mir = match MirBuilder::build(hir) {
            Ok(m) => m,
            Err(e) => return Err(Error::Mir(e.to_string())),
        };

        // lower 失败时原 HIR 仍然保留。
        self.mir = Some(mir);
        self.stage = CompilerStage::Lowered;

        Ok(self)
    }

    /// 阶段 4a：清理 CFG。只做简化，不做检查，也不会失败。
    pub fn cleanup(&mut self) -> Result<&mut Compiler, Error> {
        self.require_stage("cleanup", CompilerStage::Lowered)?;

        let mir = match self.mir.as_mut() {
            Some(m) => m,
            None => {
                return Err(stage_order_error(
                    "cleanup",
                    "lowered",
                    self.stage.name(),
                ));
            }
        };

        control::Control::simplify(mir);
        self.stage = CompilerStage::Cleaned;

        Ok(self)
    }

    /// 阶段 4b：借用检查。要求 MIR 已经过 cleanup，在干净的 CFG 上运行。
    ///
    /// 借用检查只读 MIR；只有检查成功才推进状态。
    pub fn borrow_check(&mut self) -> Result<&mut Compiler, Error> {
        self.require_stage("borrow_check", CompilerStage::Cleaned)?;

        let mir = match self.mir.as_ref() {
            Some(m) => m,
            None => {
                return Err(stage_order_error(
                    "borrow_check",
                    "cleaned",
                    self.stage.name(),
                ));
            }
        };

        match borrow::BorrowChecker::check(mir) {
            Ok(()) => {
                self.stage = CompilerStage::BorrowChecked;
                Ok(self)
            }
            Err(e) => Err(Error::Borrow(e.to_string())),
        }
    }

    /// 阶段 5：单态化（展开泛型）+ MIR 优化（常量折叠、死代码消除）。
    ///
    /// 这两个优化器当前都不返回 `Result`，因此这里可以安全地取得 MIR
    /// 的所有权并在完成后重新放回。该阶段没有失败路径。
    pub fn optimize(&mut self) -> Result<&mut Compiler, Error> {
        self.require_stage("optimize", CompilerStage::BorrowChecked)?;

        let mir = match self.mir.take() {
            Some(m) => m,
            None => {
                return Err(stage_order_error(
                    "optimize",
                    "borrow_checked",
                    self.stage.name(),
                ));
            }
        };

        let mir = monomorphic::Monomorphic::run(mir);
        let mir = simplify::Simplify::run(mir);

        self.mir = Some(mir);
        self.stage = CompilerStage::Optimized;

        Ok(self)
    }

    /// 阶段 6：代码生成。
    ///
    /// 代码生成是只读操作，不改变流水线状态，因此可以在 `Optimized`
    /// 状态下重复调用，以支持重复发射/调试输出。
    pub fn codegen(&self) -> Result<String, Error> {
        self.require_stage("codegen", CompilerStage::Optimized)?;

        let mir = match self.mir.as_ref() {
            Some(m) => m,
            None => {
                return Err(stage_order_error(
                    "codegen",
                    "optimized",
                    self.stage.name(),
                ));
            }
        };

        Ok(Codegen::generate_from_mir(mir))
    }

    /// 取最终 AST。
    pub fn program(&self) -> Option<&Program> {
        self.program.as_ref()
    }

    /// 取当前 HIR。
    pub fn hir(&self) -> Option<&HirProgram> {
        self.hir.as_ref()
    }

    /// 取当前 MIR。
    pub fn mir(&self) -> Option<&MirProgram> {
        self.mir.as_ref()
    }
}

// 完整的编译流水线：顺序调用 Compiler 的全部阶段，返回生成的 Rust 源码字符串。
//
// 构建产物（写临时目录、cargo build、跑 exe）不在这里，见 engine::drive。
pub fn compile(filename: &Path, stdlib_path: &Path) -> Result<String, Error> {
    let mut compiler = Compiler::new();
    compiler
        .parse(filename, stdlib_path)?
        .type_check()?
        .elaborate()?
        .lower()?
        .cleanup()?
        .borrow_check()?
        .optimize()?
        .codegen()
}

// 从可执行文件自身的位置反推工作区根目录下的 Standard 目录。
// 只要 current_exe() 能拿到路径，且路径层级够深，就能推出来。
// 找不到 current_exe()、路径层级不够深、或者推断出的 Standard 目录
// 实际不存在时，一律返回 None，交给调用方（config::resolve）决定是否兜底。
pub fn default_stdlib_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;

    // exe 所在目录 (target/debug 或 target/release)
    let debug_or_release_dir = exe.parent()?;
    let target_dir = debug_or_release_dir.parent()?;
    let compiler_root = target_dir.parent()?; // .../Projects/xiyi-compiler
    let projects_dir = compiler_root.parent()?; // .../Projects
    let workspace_root = projects_dir.parent()?; // .../the-xiyi-language

    let candidate = workspace_root.join("Standard");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}
