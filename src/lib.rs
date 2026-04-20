//! jitvm: a toy lang with a bytecode vm and an x86-64 jit.

pub mod ast;
pub mod interp;
pub mod ir;
pub mod lexer;
pub mod parser;

#[cfg(all(target_arch = "x86_64", any(target_os = "linux", target_os = "macos")))]
pub mod jit;
#[cfg(all(target_arch = "x86_64", any(target_os = "linux", target_os = "macos")))]
pub mod x86;

use std::fs;
use std::path::Path;

#[derive(Debug)]
pub enum Error {
    Parse(String),
    Runtime(String),
    Codegen(String),
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Parse(s) => write!(f, "parse error: {s}"),
            Error::Runtime(s) => write!(f, "runtime error: {s}"),
            Error::Codegen(s) => write!(f, "codegen error: {s}"),
            Error::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Engine {
    pub program: ir::Program,
}

impl Engine {
    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let src = fs::read_to_string(path)?;
        Self::load_str(&src)
    }

    pub fn load_str(src: &str) -> Result<Self> {
        let toks = lexer::tokenize(src)?;
        let ast = parser::parse(&toks)?;
        let program = ir::lower(&ast)?;
        Ok(Engine { program })
    }

    pub fn run_interp(&self) -> Result<i64> {
        interp::run(&self.program)
    }

    #[cfg(all(target_arch = "x86_64", any(target_os = "linux", target_os = "macos")))]
    pub fn run_jit(&self) -> Result<i64> {
        let module = jit::compile(&self.program)?;
        module.run_main()
    }

    #[cfg(not(all(target_arch = "x86_64", any(target_os = "linux", target_os = "macos"))))]
    pub fn run_jit(&self) -> Result<i64> {
        Err(Error::Codegen(
            "jit only supported on x86_64 linux/macos".into(),
        ))
    }
}
