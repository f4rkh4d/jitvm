//! stack-machine bytecode + lowering from AST.
//!
//! each function is one `Function`. jumps are *relative* to the pc *after*
//! the jump op, so `pc = pc + offset` after reading.

use crate::ast::{Span, *};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy)]
pub enum Op {
    Const(i64),
    LoadLocal(u16),
    StoreLocal(u16),
    LoadArg(u8),
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,
    Not,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    Jump(i32),
    JumpIfFalse(i32),
    Call(u32, u8),
    Ret,
    Print,
    Pop,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub argc: u8,
    /// number of `let` locals (does not include args).
    pub locals: u16,
    pub code: Vec<Op>,
    /// parallel to `code`. `Span::UNKNOWN` when we have no source info
    /// for an op. only a few ops (currently Div, Mod) actually bother to
    /// record anything, since most ops can't fault at runtime.
    pub spans: Vec<Span>,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub fns: Vec<Function>,
    pub main_id: u32,
}

pub fn lower(prog: &ProgramAst) -> Result<Program> {
    // build name -> id map
    let mut name_to_id = std::collections::HashMap::new();
    for (i, f) in prog.fns.iter().enumerate() {
        if name_to_id.insert(f.name.clone(), i as u32).is_some() {
            return Err(Error::Parse(format!("duplicate function {}", f.name)));
        }
    }
    let main_id = *name_to_id
        .get("main")
        .ok_or_else(|| Error::Parse("no main() function".into()))?;

    let mut fns = Vec::with_capacity(prog.fns.len());
    for f in &prog.fns {
        fns.push(lower_fn(f, &name_to_id)?);
    }
    Ok(Program { fns, main_id })
}

type ProgramAst = crate::ast::Program;

struct Lowerer<'a> {
    code: Vec<Op>,
    spans: Vec<Span>,
    // local names -> slot. args go first (slots 0..argc), then lets.
    locals: Vec<String>,
    argc: u8,
    name_to_id: &'a std::collections::HashMap<String, u32>,
}

fn lower_fn(f: &Fn, name_to_id: &std::collections::HashMap<String, u32>) -> Result<Function> {
    let mut l = Lowerer {
        code: Vec::new(),
        spans: Vec::new(),
        locals: f.params.clone(),
        argc: f.params.len() as u8,
        name_to_id,
    };
    for s in &f.body {
        l.stmt(s)?;
    }
    // implicit return 0 if control falls through
    l.emit(Op::Const(0));
    l.emit(Op::Ret);
    let locals_count = (l.locals.len() as u16).saturating_sub(l.argc as u16);
    debug_assert_eq!(l.code.len(), l.spans.len());
    Ok(Function {
        name: f.name.clone(),
        argc: l.argc,
        locals: locals_count,
        code: l.code,
        spans: l.spans,
    })
}

impl<'a> Lowerer<'a> {
    fn resolve(&self, name: &str) -> Option<(bool, u16)> {
        // returns (is_arg, slot).
        self.locals
            .iter()
            .position(|n| n == name)
            .map(|i| (i < self.argc as usize, i as u16))
    }

    fn declare(&mut self, name: String) -> u16 {
        self.locals.push(name);
        (self.locals.len() - 1) as u16
    }

    /// emit an op with no meaningful source position. the vast majority of
    /// ops can't fault at runtime, so they just record `Span::UNKNOWN`.
    fn emit(&mut self, op: Op) {
        self.code.push(op);
        self.spans.push(Span::UNKNOWN);
    }

    /// emit an op that can fault at runtime. the span lets the interpreter
    /// surface "division by zero at line 7, col 12" rather than a bare string.
    fn emit_at(&mut self, op: Op, span: Span) {
        self.code.push(op);
        self.spans.push(span);
    }

    fn stmt(&mut self, s: &Stmt) -> Result<()> {
        match s {
            Stmt::Let(name, e) => {
                self.expr(e)?;
                let slot = self.declare(name.clone());
                self.emit(Op::StoreLocal(slot));
            }
            Stmt::Assign(name, e) => {
                self.expr(e)?;
                let (_is_arg, slot) = self.resolve(name).ok_or_else(|| {
                    Error::Parse(format!("assignment to undeclared variable {name}"))
                })?;
                // args live in the same slot array as lets, so write is identical.
                self.emit(Op::StoreLocal(slot));
            }
            Stmt::ExprStmt(e) => {
                self.expr(e)?;
                self.emit(Op::Pop);
            }
            Stmt::Print(e) => {
                self.expr(e)?;
                self.emit(Op::Print);
            }
            Stmt::Return(e) => {
                self.expr(e)?;
                self.emit(Op::Ret);
            }
            Stmt::If(cond, then_b, else_b) => {
                self.expr(cond)?;
                let jf_pos = self.code.len();
                self.emit(Op::JumpIfFalse(0)); // patched
                for s in then_b {
                    self.stmt(s)?;
                }
                if else_b.is_empty() {
                    let end = self.code.len();
                    self.patch_jif(jf_pos, end);
                } else {
                    let jmp_pos = self.code.len();
                    self.emit(Op::Jump(0)); // patched
                    let else_start = self.code.len();
                    self.patch_jif(jf_pos, else_start);
                    for s in else_b {
                        self.stmt(s)?;
                    }
                    let end = self.code.len();
                    self.patch_jmp(jmp_pos, end);
                }
            }
            Stmt::While(cond, body) => {
                let start = self.code.len();
                self.expr(cond)?;
                let jf_pos = self.code.len();
                self.emit(Op::JumpIfFalse(0));
                for s in body {
                    self.stmt(s)?;
                }
                // jump back to start
                let back_pos = self.code.len();
                let back_next = (back_pos + 1) as i32;
                self.emit(Op::Jump(start as i32 - back_next));
                let end = self.code.len();
                self.patch_jif(jf_pos, end);
            }
        }
        Ok(())
    }

    fn patch_jif(&mut self, pos: usize, target: usize) {
        let offset = target as i32 - (pos as i32 + 1);
        self.code[pos] = Op::JumpIfFalse(offset);
    }

    fn patch_jmp(&mut self, pos: usize, target: usize) {
        let offset = target as i32 - (pos as i32 + 1);
        self.code[pos] = Op::Jump(offset);
    }

    fn expr(&mut self, e: &Expr) -> Result<()> {
        match e {
            Expr::Int(v) => self.emit(Op::Const(*v)),
            Expr::Var(name) => {
                let (is_arg, slot) = self
                    .resolve(name)
                    .ok_or_else(|| Error::Parse(format!("undeclared variable {name}")))?;
                if is_arg {
                    self.emit(Op::LoadArg(slot as u8));
                } else {
                    self.emit(Op::LoadLocal(slot));
                }
            }
            Expr::Call(name, args) => {
                if name == "print" {
                    return Err(Error::Parse("print must be used as a statement".into()));
                }
                let id = *self
                    .name_to_id
                    .get(name)
                    .ok_or_else(|| Error::Parse(format!("unknown function {name}")))?;
                if args.len() > 6 {
                    return Err(Error::Parse(format!(
                        "call to {name} has >6 args; limit is 6"
                    )));
                }
                // arity check at lowering time: catches the mistake at compile
                // time rather than turning it into a silent stack misalignment.
                let callee_argc = {
                    // we resolve name->id above; fetch the declared argc by id
                    // via the same map + caller-supplied Vec (rebuilt in lower).
                    // to keep the lowerer independent of final fn vec ordering,
                    // we just defer the argc check to runtime for now, where
                    // interp::call already verifies. codegen mirrors that by
                    // always pushing exactly args.len() values.
                    args.len() as u8
                };
                for a in args {
                    self.expr(a)?;
                }
                self.emit(Op::Call(id, callee_argc));
            }
            Expr::Un(op, e) => {
                self.expr(e)?;
                self.emit(match op {
                    UnOp::Neg => Op::Neg,
                    UnOp::Not => Op::Not,
                });
            }
            Expr::Bin(op, a, b, span) => {
                match op {
                    BinOp::And => {
                        // short-circuit: eval a; if false, result is 0; else result is (b != 0).
                        self.expr(a)?;
                        // a is on stack. jif over true-branch, leaving 0.
                        let jf = self.code.len();
                        self.emit(Op::JumpIfFalse(0));
                        // a was truthy; pop'd by jif. now eval b and normalize to bool.
                        self.expr(b)?;
                        self.emit(Op::Const(0));
                        self.emit(Op::Ne);
                        let jmp_end = self.code.len();
                        self.emit(Op::Jump(0));
                        let false_start = self.code.len();
                        self.patch_jif(jf, false_start);
                        self.emit(Op::Const(0));
                        let end = self.code.len();
                        self.patch_jmp(jmp_end, end);
                    }
                    BinOp::Or => {
                        self.expr(a)?;
                        // if a is true, result is 1; else result is (b != 0).
                        let jf = self.code.len();
                        self.emit(Op::JumpIfFalse(0));
                        self.emit(Op::Const(1));
                        let jmp_end = self.code.len();
                        self.emit(Op::Jump(0));
                        let b_start = self.code.len();
                        self.patch_jif(jf, b_start);
                        self.expr(b)?;
                        self.emit(Op::Const(0));
                        self.emit(Op::Ne);
                        let end = self.code.len();
                        self.patch_jmp(jmp_end, end);
                    }
                    _ => {
                        self.expr(a)?;
                        self.expr(b)?;
                        let out = match op {
                            BinOp::Add => Op::Add,
                            BinOp::Sub => Op::Sub,
                            BinOp::Mul => Op::Mul,
                            BinOp::Div => Op::Div,
                            BinOp::Mod => Op::Mod,
                            BinOp::Lt => Op::Lt,
                            BinOp::Le => Op::Le,
                            BinOp::Gt => Op::Gt,
                            BinOp::Ge => Op::Ge,
                            BinOp::Eq => Op::Eq,
                            BinOp::Ne => Op::Ne,
                            BinOp::And | BinOp::Or => unreachable!(),
                        };
                        // attach source position only for the ops that can
                        // actually trap. everything else gets UNKNOWN, saves
                        // bytes in the no-op path.
                        match op {
                            BinOp::Div | BinOp::Mod => self.emit_at(out, *span),
                            _ => self.emit(out),
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
