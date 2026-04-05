//! jitvm: a toy lang, a bytecode vm, and (eventually) a jit.
//! wip. this commit only parses.

pub mod ast;
pub mod lexer;
pub mod parser;

#[derive(Debug)]
pub enum Error {
    Parse(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Parse(s) => write!(f, "parse error: {s}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub fn parse(src: &str) -> Result<ast::ProgramAst> {
    let toks = lexer::tokenize(src).map_err(Error::Parse)?;
    parser::parse(&toks).map_err(Error::Parse)
}
