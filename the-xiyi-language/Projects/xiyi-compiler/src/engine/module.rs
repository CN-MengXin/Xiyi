// src/engine/module.rs
mod config;
mod pipeline;
mod drive;
mod error;

pub use config::{Args, CompilerConfig, Emit, StdlibSource, USAGE};
pub use pipeline::{
    compile,
    load_stdlib,
    merge_with_conflict_check,
    default_stdlib_path,
    Compiler,
};
pub use drive::build_and_run;
pub use error::Error;