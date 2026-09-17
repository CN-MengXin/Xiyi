// src/engine/error.rs
use std::path::PathBuf;

/// 编译器的统一错误类型：每个 variant 对应一条独立的问题类别。
///
/// 不用 `Result<_, String>` 而用枚举，是为了让上层（main.rs、LSP、测试）
/// 能按类别 match：决定退出码、决定要不要追加「建议」、断言具体是哪类错误，
/// 而不是靠 `starts_with("Type error:")` 猜。
///
/// 不用 #[derive(Debug, thiserror::Error)]：自举时 xiyi 没有 proc macro，
/// 这些 trait 都得手写。Display 就在下面，Debug 不实现——main.rs 里
/// 只用 `{}`（Display）打印错误，没有人用 `{:?}`。
pub enum Error {
    /// 命令行用法错误：未知选项、缺参数、多余位置参数、阶段调用顺序错误等。
    Usage(String),

    /// 文件系统错误，`path` 指明是哪一步涉及哪个路径。
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    /// 解析错误：`file` 是出错文件（路径或模块标签），`message` 是解析器给出的原因。
    Parse { file: String, message: String },

    /// 类型检查错误。
    Type(String),

    /// 语法糖展开（elaborate）错误。
    Elaborate(String),

    /// MIR 构建错误。
    Mir(String),

    /// 借用检查错误。
    Borrow(String),

    /// 重复定义：标准库内部两个模块撞名，或用户代码与标准库撞名。
    DuplicateDef {
        name: String,
        first: String,
        second: String,
    },

    /// 构建（cargo build）阶段错误。
    Build(String),

    /// 运行生成的可执行文件阶段错误。
    Run(String),
}

impl Error {
    /// 把 `std::io::Error` 转成带路径的 `Error::Io`。
    ///
    /// 之前用 `impl FnOnce` 返回闭包，能直接塞进 `.map_err(...)`；
    /// 但 impl Trait + 闭包类型在自举时不好表达，改成普通函数。
    /// 调用点从 `.map_err(Error::io_err(&p))` 变成
    /// `.map_err(|e| Error::io(p.to_path_buf(), e))`——多写一个闭包，
    /// 但每个字都能在 xiyi 里直译。
    pub(crate) fn io(path: PathBuf, source: std::io::Error) -> Error {
        Error::Io { path, source }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Usage(msg) => write!(f, "{}", msg),
            Error::Io { path, source } => {
                write!(f, "I/O error on {}: {}", path.display(), source)
            }
            Error::Parse { file, message } => {
                write!(f, "parse error in {}: {}", file, message)
            }
            Error::Type(msg) => write!(f, "type error: {}", msg),
            Error::Elaborate(msg) => write!(f, "elaboration error: {}", msg),
            Error::Mir(msg) => write!(f, "MIR error: {}", msg),
            Error::Borrow(msg) => write!(f, "borrow error: {}", msg),
            Error::DuplicateDef { name, first, second } => {
                write!(f, "duplicate definition `{}` in {} and {}", name, first, second)
            }
            Error::Build(msg) => write!(f, "build error: {}", msg),
            Error::Run(msg) => write!(f, "run error: {}", msg),
        }
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
