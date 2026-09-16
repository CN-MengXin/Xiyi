// src/main.rs
use xiyi_compiler::engine::{build_and_run, Args, Error, StdlibSource, USAGE};

// 错误类别 → 退出码：
//   1 = 用户输入错（用法 / 参数）
//   2 = 编译错（I/O、解析、类型、展开、MIR、借用、重复定义）
//   3 = 构建错（cargo build）
//   4 = 运行错（生成的可执行文件）
fn exit_code(err: &Error) -> i32 {
    match err {
        Error::Usage(_) => 1,
        Error::Io { .. }
        | Error::Parse { .. }
        | Error::Type(_)
        | Error::Elaborate(_)
        | Error::Mir(_)
        | Error::Borrow(_)
        | Error::DuplicateDef { .. } => 2,
        Error::Build(_) => 3,
        Error::Run(_) => 4,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // 帮助 / 版本 / 编译三种走向都在这里决定打印与退出——
    // 库层不碰 stdout/stderr，也不 process::exit。
    let config = match Args::from_args(&args) {
        Ok(Args::Help) => {
            print!("{}", USAGE);
            std::process::exit(0);
        }
        Ok(Args::Version) => {
            println!("Xiyi {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        Ok(Args::Compile(config)) => config,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(exit_code(&e));
        }
    };

    // 信息性提示由调用方决定要不要打印；库层只记录来源，保持安静。
    // 用 match 穷举（而不是 if/else if），将来 StdlibSource 加变体编译器会强制处理。
    match &config.stdlib_source {
        StdlibSource::Explicit => {}
        StdlibSource::Inferred => {
            eprintln!(
                "未指定 --stdlib，使用推断出的标准库路径: {}",
                config.stdlib_path.display()
            );
        }
        StdlibSource::Fallback => {
            eprintln!(
                "未指定 --stdlib，推断失败，使用当前目录下的兜底标准库路径: {}",
                config.stdlib_path.display()
            );
        }
    }

    // 被运行程序的退出码原样透传。
    match build_and_run(&config) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(exit_code(&e));
        }
    }
}