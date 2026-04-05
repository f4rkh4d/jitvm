//! ast types. small & boring on purpose.

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
    Var(String),
    Call(String, Vec<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Un(UnOp, Box<Expr>),
}
