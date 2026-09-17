// src/engine/drive.rs
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::config::{CompilerConfig, Emit};
use super::error::Error;
use super::pipeline;

/// 生成的临时 cargo 项目模板。
/// tch 依赖注释掉，避免 torch-sys 编译失败。
const CARGO_TOML: &str = r#"[package]
name = "xiyi_output"
version = "0.1.0"
edition = "2024"

[dependencies]
# tch = { version = "0.13", features = ["download-libtorch"] }

[[bin]]
name = "xiyi_output"
path = "src/main.rs"
"#;

/// 写文件并自动建父目录。
///
/// 两点防御：
/// 1. 内容没变就不写。cargo 用 mtime 判断要不要重编，每次都覆盖
///    会让「保留增量缓存」的意图落空。
/// 2. `path.parent()` 对 `Cargo.toml` 这种裸文件名返回 `Some("")`，
///    `create_dir_all("")` 的行为平台相关，用 is_empty 拦掉。
///
/// 用显式 match 而不是 `.map_err(Error::io_err(...))`：`io_err` 返回
/// `impl FnOnce`（impl Trait + 闭包），自举时 xiyi 不好表达。
fn write_file(path: &Path, content: &str) -> Result<(), Error> {
    // 内容未变则跳过，保留 mtime，让 cargo 的增量缓存继续有效。
    if let Ok(existing) = fs::read_to_string(path) {
        if existing == content {
            return Ok(());
        }
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            match fs::create_dir_all(parent) {
                Ok(()) => {}
                Err(e) => return Err(Error::io(parent.to_path_buf(), e)),
            }
        }
    }

    match fs::write(path, content) {
        Ok(()) => Ok(()),
        Err(e) => Err(Error::io(path.to_path_buf(), e)),
    }
}

/// 生成可执行文件在 cargo target 下的路径（跨平台、按 profile 分）。
///
/// 不用 `cfg!(windows)`：cfg! 是宏，自举时 xiyi 没有。
/// `std::env::consts::EXE_SUFFIX` 是常量——Windows 上为 ".exe"，
/// 其他平台为空串，用它拼出带/不带后缀的文件名。
fn exe_path(target_dir: &Path, release: bool) -> PathBuf {
    let name = format!("xiyi_output{}", std::env::consts::EXE_SUFFIX);
    let profile = if release { "release" } else { "debug" };
    target_dir.join(profile).join(name)
}

/// 把生成的 Rust 源码写到 stdout（--emit=rust 分支）。
///
/// 用 write_all 而不是 print!：print! 遇到 Broken pipe 会 panic，
/// 而 `xiyi --emit=rust f.xiyi | head -1` 这种「下游提前关闭管道」
/// 是正常用法，不该炸出栈回溯。EPIPE 视作成功（下游不想读了）。
///
/// 末尾补换行：codegen 的输出通常以 \n 结尾，但不能保证；
/// 不补的话 shell 提示符会贴在最后一行后面。
fn emit_rust_to_stdout(rust_code: &str) -> Result<i32, Error> {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();

    match lock.write_all(rust_code.as_bytes()) {
        Ok(()) => {}
        Err(e) => match e.kind() {
            std::io::ErrorKind::BrokenPipe => return Ok(0),
            _ => return Err(Error::Run(format!("failed to write stdout: {}", e))),
        },
    }

    if !rust_code.ends_with('\n') {
        match lock.write_all(b"\n") {
            Ok(()) => {}
            Err(e) => match e.kind() {
                std::io::ErrorKind::BrokenPipe => return Ok(0),
                _ => return Err(Error::Run(format!("failed to write stdout: {}", e))),
            },
        }
    }

    Ok(0)
}

/// 编译并运行一个 xiyi 源文件。
///
/// 先跑完整个编译流水线（见 pipeline::compile）拿到生成的 Rust 代码，
/// 再把它写进一个临时 cargo 项目、跑 `cargo build`，最后执行生成的
/// 可执行文件，返回它的退出码。
///
/// 任何一步失败都返回描述性的 Err(Error)；打印到 stderr 和决定
/// std::process::exit 的退出码是调用方（main.rs）的事。
/// 计算稳定的缓存键。
///
/// 不依赖第三方 hash crate，也不使用随机种子的 DefaultHasher。
/// 同一个输入文件与标准库路径会得到同一个 target 目录，从而保留
/// cargo 的增量编译缓存；不同项目则使用不同缓存，避免互相污染。
fn cache_key(config: &CompilerConfig) -> String {
    let mut hash: u64 = 14695981039346656037;

    fn feed(hash: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *hash ^= *byte as u64;
            *hash = hash.wrapping_mul(1099511628211);
        }
    }

    feed(&mut hash, config.filename.to_string_lossy().as_bytes());
    feed(&mut hash, &[0]);
    feed(&mut hash, config.stdlib_path.to_string_lossy().as_bytes());
    feed(&mut hash, &[0]);
    feed(&mut hash, CARGO_TOML.as_bytes());

    format!("{:016x}", hash)
}

/// 当前编译调用的临时 Cargo 工程。
///
/// 工程目录只负责保存本次生成的 Cargo.toml/main.rs；真正的 Cargo
/// target 目录位于稳定缓存目录，因此删除这个工作区不会丢失增量缓存。
struct BuildWorkspace {
    path: PathBuf,
}

impl BuildWorkspace {
    fn new() -> Result<BuildWorkspace, Error> {
        let path = std::env::temp_dir().join(format!(
            "xiyi_build_{}",
            std::process::id()
        ));

        // 同一进程正常不会重复创建；若上一次异常退出留下同名目录，
        // 先清理再重建。
        if path.exists() {
            match fs::remove_dir_all(&path) {
                Ok(()) => {}
                Err(e) => return Err(Error::io(path.clone(), e)),
            }
        }

        match fs::create_dir_all(path.join("src")) {
            Ok(()) => Ok(BuildWorkspace { path }),
            Err(e) => Err(Error::io(path.clone(), e)),
        }
    }
}

impl Drop for BuildWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn build_and_run(config: &CompilerConfig) -> Result<i32, Error> {
    let rust_code = pipeline::compile(&config.filename, &config.stdlib_path)?;

    // ===== --emit=rust：只把生成的源码写到 stdout，不构建、不运行 =====
    // 用 match 而不是 `==`：Emit 没有 PartialEq（去宏），且穷举更稳。
    match config.emit {
        Emit::Rust => return emit_rust_to_stdout(&rust_code),
        Emit::Exe => {}
    }

    // ===== 构建工作区与稳定增量缓存 =====
    // 工作区按进程隔离，避免并发编译互相覆盖 Cargo.toml/main.rs；
    // Cargo target 目录则按输入文件 + 标准库路径稳定分配，因此不同
    // 进程再次编译同一项目时仍能复用增量缓存。
    let workspace = BuildWorkspace::new()?;
    let cache_dir = std::env::temp_dir()
        .join("xiyi_cache")
        .join(cache_key(config));

    match fs::create_dir_all(&cache_dir) {
        Ok(()) => {}
        Err(e) => return Err(Error::io(cache_dir.clone(), e)),
    }

    write_file(&workspace.path.join("Cargo.toml"), CARGO_TOML)?;
    write_file(&workspace.path.join("src").join("main.rs"), &rust_code)?;

    // ===== cargo build =====
    let mut cargo = Command::new("cargo");
    cargo.arg("build")
        .arg("--target-dir")
        .arg(&cache_dir)
        .current_dir(&workspace.path);
    if config.release {
        cargo.arg("--release");
    }
    let status = match cargo.status() {
        Ok(s) => s,
        Err(e) => return Err(Error::Build(format!("failed to run cargo: {}", e))),
    };

    if !status.success() {
        return Err(Error::Build("cargo build failed".to_string()));
    }

    let built_exe = exe_path(&cache_dir, config.release);
    if !built_exe.exists() {
        return Err(Error::Build(format!(
            "executable not found after build: {}",
            built_exe.display()
        )));
    }

    // 复制到本次独立工作区，再运行副本。这样即使另一个进程随后
    // 使用同一个稳定 target 缓存重新构建，也不会改变本次要运行的文件。
    let run_copy = workspace.path.join(format!(
        "xiyi_output{}",
        std::env::consts::EXE_SUFFIX
    ));
    match fs::copy(&built_exe, &run_copy) {
        Ok(_) => {}
        Err(e) => return Err(Error::io(run_copy.clone(), e)),
    }

    // ===== -o：复制到目标路径，运行那份 =====
    // 指定了 --output 就把产物落到用户要的位置再跑；否则运行本次
    // 工作区里的副本，而不是直接运行共享缓存中的文件。
    let run_path = match &config.output {
        Some(dest) => {
            if let Some(parent) = dest.parent() {
                if !parent.as_os_str().is_empty() {
                    match fs::create_dir_all(parent) {
                        Ok(()) => {}
                        Err(e) => return Err(Error::io(parent.to_path_buf(), e)),
                    }
                }
            }
            match fs::copy(&run_copy, dest) {
                Ok(_) => {}
                Err(e) => return Err(Error::io(dest.clone(), e)),
            }
            dest.clone()
        }
        None => run_copy,
    };

    // ===== 运行生成的可执行文件 =====
    let run_status = match Command::new(&run_path).status() {
        Ok(s) => s,
        Err(e) => {
            return Err(Error::Run(format!(
                "failed to run {}: {}",
                run_path.display(),
                e
            )));
        }
    };

    // 子进程的正常退出码透传。被信号终止时 code() 返回 None——
    // 不能当成功（原来 unwrap_or(0) 会把 segfault 报成 0），退化为 1。
    match run_status.code() {
        Some(code) => Ok(code),
        None => Ok(1),
    }
}