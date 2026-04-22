//! ast types. small & boring on purpose.

/// source position of a token. `(0, 0)` means "unknown".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Span {
    pub line: u32,
    pub col: u32,
}

impl Span {
    pub const UNKNOWN: Span = Span { line: 0, col: 0 };
    pub fn new(line: u32, col: u32) -> Self {
        Span { line, col }
    }
    pub fn is_known(&self) -> bool {
        self.line != 0
    }
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_known() {
            write!(f, "line {}, col {}", self.line, self.col)
        } else {
            write!(f, "?")
        }
    }
}

#[derive(Debug, Clone)]
pub struct Program {
    pub fns: Vec<Fn>,
}

#[derive(Debug, Clone)]
pub struct Fn {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let(String, Expr),
    Assign(String, Expr),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    Return(Expr),
    ExprStmt(Expr),
    Print(Expr),
}

#[derive(Debug, Clone, Copy)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Debug, Clone, Copy)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Str(String, Span),
    Var(String),
    Call(String, Vec<Expr>, Span),
    /// Bin carries the operator's source span so runtime errors like
    /// div-by-zero can report the faulting position.
    Bin(BinOp, Box<Expr>, Box<Expr>, Span),
    Un(UnOp, Box<Expr>),
}
