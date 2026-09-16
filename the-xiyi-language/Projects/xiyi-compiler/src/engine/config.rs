// src/engine/config.rs
use std::path::PathBuf;

use super::error::Error;
use super::pipeline;

/// 编译到哪一步。
///
/// 不用 #[derive(...)]：自举时 xiyi 没有宏系统，这些 trait 都得手写。
/// 与其让将来重写时逐个补，不如现在就显式写出来。需要比较的地方
/// 用 match（穷举，将来加变体会强制处理），不依赖 PartialEq。
pub enum Emit {
    /// 只把生成的 Rust 源码写到 stdout，不构建、不运行
    Rust,
    /// 构建并执行生成的程序（默认）
    Exe,
}

/// 帮助文本。作为数据留在库层，打印与否由 main.rs 决定（库层保持安静）。
pub const USAGE: &str = "\
xiyi - 希夷语言编译器

用法:
  xiyi [选项] <FILE>

选项:
      --stdlib <PATH>   标准库根目录在此，不传则从可执行文件位置推断
  -o, --output <PATH>   把构建出的可执行文件另存到这个路径
      --release         用 release 模式构建（cargo build --release）
      --emit <KIND>     编译到哪一步为止 [默认: exe]（可选: rust, exe）
  -h, --help            打印本帮助
  -V, --version         打印版本号

  --                    之后一律作为位置参数（用于文件名以 - 开头的情况）
";

/// stdlib 路径是怎么来的。
///
/// 单独记成一个字段，而不是在 resolve 里直接 eprintln!，
/// 是为了让「要不要把这件事告诉用户」由调用方（main.rs）决定。
/// 库层被嵌进 LSP / IDE 插件 / Web 服务时，不该往 stderr 写东西。
pub enum StdlibSource {
    /// 用户通过 --stdlib 显式给出
    Explicit,
    /// 未指定，从可执行文件位置推断得到
    Inferred,
    /// 未指定且推断失败，用当前工作目录下的 Standard 兜底得到
    Fallback,
}

/// from_args 的返回：一次命令行解析可能不是要编译，而是要看帮助/版本。
/// 把「打印帮助、打印版本、编译」三件事收进枚举，库层就不需要 print!/exit。
pub enum Args {
    Compile(CompilerConfig),
    Help,
    Version,
}

/// 一次编译运行所需的全部配置。
pub struct CompilerConfig {
    pub filename: PathBuf,
    pub stdlib_path: PathBuf,
    pub stdlib_source: StdlibSource,
    pub output: Option<PathBuf>,
    pub release: bool,
    pub emit: Emit,
}

/// 命令行原始参数（尚未解析 stdlib 路径）。
struct RawArgs {
    action: RawAction,
    filename: Option<String>,
    stdlib: Option<String>,
    output: Option<String>,
    release: bool,
    emit: Emit,
    emit_was_set: bool,
}

impl RawArgs {
    /// 显式构造，不依赖 Default。
    fn new() -> RawArgs {
        RawArgs {
            action: RawAction::Compile,
            filename: None,
            stdlib: None,
            output: None,
            release: false,
            emit: Emit::Exe,
            emit_was_set: false,
        }
    }
}

/// parse_raw 是「编译」「打印帮助」「打印版本」中的哪一种。
enum RawAction {
    Compile,
    Help,
    Version,
}

/// 解析命令行参数：识别 --stdlib、位置参数、未知选项。
///
/// -h/-V 只负责记下 action，打印与退出交给 main.rs——库层不碰 stdout/stderr。
fn parse_raw(args: &[String]) -> Result<RawArgs, Error> {
    let mut raw = RawArgs::new();
    // `--` 之后一律当位置参数，用于文件名以 `-` 开头的情况。
    let mut positional_only = false;

    let mut i = 1;
    while i < args.len() {
        let arg = args[i].as_str();

        // ---- `--` 之后：一律位置参数 ----
        if positional_only {
            push_filename(&mut raw.filename, arg)?;
            i += 1;
            continue;
        }
        if arg == "--" {
            positional_only = true;
            i += 1;
            continue;
        }

        // ---- 无值选项 ----
        // 帮助/版本：立即返回，不再解析后续参数（与原实现 exit(0) 的行为一致）。
        if arg == "-h" || arg == "--help" {
            raw.action = RawAction::Help;
            return Ok(raw);
        }
        if arg == "-V" || arg == "--version" {
            raw.action = RawAction::Version;
            return Ok(raw);
        }
        if arg == "--release" {
            raw.release = true;
            i += 1;
            continue;
        }

        // ---- 带值选项：`--name=value` 形式 ----
        if let Some(eq) = arg.find('=') {
            if arg.starts_with("--") {
                let name = &arg[..eq];
                let value = &arg[eq + 1..];
                if value.is_empty() {
                    return Err(Error::Usage(format!("{} 需要一个值", name)));
                }
                match name {
                    "--stdlib" => {
                        set_once(&mut raw.stdlib, value.to_string(), "--stdlib 重复指定")?
                    }
                    "--output" => {
                        set_once(&mut raw.output, value.to_string(), "--output 重复指定")?
                    }
                    "--emit" => {
                        if raw.emit_was_set {
                            return Err(Error::Usage("--emit 重复指定".to_string()));
                        }
                        raw.emit = parse_emit(value)?;
                        raw.emit_was_set = true;
                    },
                    _ => return Err(Error::Usage(format!("未知选项: {}", name))),
                }
                i += 1;
                continue;
            }
            // 短选项 `-o=value`
            if let Some(value) = arg.strip_prefix("-o=") {
                if value.is_empty() {
                    return Err(Error::Usage("-o 需要一个值".to_string()));
                }
                set_once(&mut raw.output, value.to_string(), "--output 重复指定")?;
                i += 1;
                continue;
            }
        }

        // ---- 带值选项：`--name value` 形式 ----
        match arg {
            "--stdlib" | "-o" | "--output" | "--emit" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| Error::Usage(format!("{} 需要一个值", arg)))?;
                match arg {
                    "--stdlib" => {
                        set_once(&mut raw.stdlib, value.clone(), "--stdlib 重复指定")?
                    }
                    "-o" | "--output" => {
                        set_once(&mut raw.output, value.clone(), "--output 重复指定")?
                    }
                    "--emit" => {
                        if raw.emit_was_set {
                            return Err(Error::Usage("--emit 重复指定".to_string()));
                        }
                        raw.emit = parse_emit(value)?;
                        raw.emit_was_set = true;
                    },
                    _ => unreachable!(),
                }
                i += 2;
                continue;
            }
            _ => {}
        }

        // ---- 未知选项 ----
        if arg.starts_with("--") {
            return Err(Error::Usage(format!("未知选项: {}", arg)));
        }
        // 单个 `-` 当位置参数（stdin 约定），其余 `-x` 视为未知短选项。
        if arg.starts_with('-') && arg.len() > 1 {
            return Err(Error::Usage(format!("未知选项: {}", arg)));
        }

        // ---- 位置参数 ----
        push_filename(&mut raw.filename, arg)?;
        i += 1;
    }

    Ok(raw)
}

/// 收下唯一的源文件；再来一个就报错——不静默丢弃。
fn push_filename(slot: &mut Option<String>, arg: &str) -> Result<(), Error> {
    if slot.is_some() {
        return Err(Error::Usage(format!(
            "多余的位置参数: {}（只接受一个源文件）",
            arg
        )));
    }
    *slot = Some(arg.to_string());
    Ok(())
}

/// 带值选项只允许出现一次：第二次出现（无论 `=` 形式还是空格形式）报错。
fn set_once<T>(slot: &mut Option<T>, value: T, duplicate_msg: &str) -> Result<(), Error> {
    if slot.is_some() {
        return Err(Error::Usage(duplicate_msg.to_string()));
    }
    *slot = Some(value);
    Ok(())
}

fn parse_emit(value: &str) -> Result<Emit, Error> {
    match value {
        "rust" => Ok(Emit::Rust),
        "exe" => Ok(Emit::Exe),
        other => Err(Error::Usage(format!(
            "未知的 --emit 取值: {}（可选: rust, exe）",
            other
        ))),
    }
}

impl Args {
    /// 解析命令行参数，并补齐 stdlib 路径。
    ///
    /// 帮助/版本请求不在这里打印，而是通过 `Args` 交还给调用方。
    pub fn from_args(args: &[String]) -> Result<Args, Error> {
        let raw = parse_raw(args)?;
        match raw.action {
            RawAction::Help => Ok(Args::Help),
            RawAction::Version => Ok(Args::Version),
            RawAction::Compile => {
                CompilerConfig::resolve(raw.filename, raw.stdlib, raw.output, raw.release, raw.emit)
                    .map(Args::Compile)
            }
        }
    }
}

impl CompilerConfig {
    fn resolve(
        filename: Option<String>,
        stdlib_arg: Option<String>,
        output: Option<String>,
        release: bool,
        emit: Emit,
    ) -> Result<CompilerConfig, Error> {
        let filename =
            filename.ok_or_else(|| Error::Usage("未指定输入文件".to_string()))?;

        let (stdlib_path, stdlib_source) = match stdlib_arg {
            Some(p) => (PathBuf::from(p), StdlibSource::Explicit),
            None => match pipeline::default_stdlib_path() {
                Some(p) => (p, StdlibSource::Inferred),
                None => match std::env::current_dir().map(|cwd| cwd.join("Standard")) {
                    Ok(candidate) if candidate.is_dir() => {
                        (candidate, StdlibSource::Fallback)
                    }
                    _ => {
                        return Err(Error::Usage(
                            "未指定 --stdlib，且无法从可执行文件位置推断标准库目录；\
                             请显式传入 --stdlib <path>"
                                .to_string(),
                        ));
                    }
                },
            },
        };

        Ok(CompilerConfig {
            filename: PathBuf::from(filename),
            stdlib_path,
            stdlib_source,
            output: output.map(PathBuf::from),
            release,
            emit,
        })
    }
}