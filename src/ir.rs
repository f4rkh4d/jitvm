//! stack-machine bytecode + lowering from AST.
//!
//! each function is one `Function`. jumps are *relative* to the pc *after*
//! the jump op, so `pc = pc + offset` after reading.
//!
//! 0.2: `Op::Const(i64)` now stores the **tagged** value directly. the
//! lowerer shifts source integer literals left by 1 before emitting so the
//! interp and jit both see pre-tagged values on the val stack. see
//! `src/value.rs` for the tag scheme.

use crate::ast::{Span, *};
use crate::value;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy)]
pub enum Op {
    /// push an already-tagged 64-bit value. for int literals the lowerer
    /// shifts before emitting; for "logical" constants like 0/1 (used by
    /// `&&`/`||` lowering) the lowerer also shifts.
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
    /// push a tagged pointer to the interned string at `string_pool[id]`.
    Str(u32),
    /// pop two tagged string pointers, push a new tagged string pointer.
    Concat,
    /// pop a tagged string pointer, push its length (tagged int).
    StrLen,
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
    /// literal string pool. `Op::Str(id)` indexes this. duplicates in the
    /// source collapse to the same id.
    pub string_pool: Vec<String>,
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
    let mut string_pool: Vec<String> = Vec::new();
    for f in &prog.fns {
        fns.push(lower_fn(f, &name_to_id, &mut string_pool)?);
    }
    Ok(Program {
        fns,
        main_id,
        string_pool,
    })
}

type ProgramAst = crate::ast::Program;

struct Lowerer<'a> {
    code: Vec<Op>,
    spans: Vec<Span>,
    // local names -> slot. args go first (slots 0..argc), then lets.
    locals: Vec<String>,
    argc: u8,
    name_to_id: &'a std::collections::HashMap<String, u32>,
    string_pool: &'a mut Vec<String>,
}

fn lower_fn(
    f: &Fn,
    name_to_id: &std::collections::HashMap<String, u32>,
    string_pool: &mut Vec<String>,
) -> Result<Function> {
    let mut l = Lowerer {
        code: Vec::new(),
        spans: Vec::new(),
        locals: f.params.clone(),
        argc: f.params.len() as u8,
        name_to_id,
        string_pool,
    };
    for s in &f.body {
        l.stmt(s)?;
    }
    // implicit return 0 if control falls through. 0 tagged as int is 0.
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

    /// intern a literal in the shared string pool. duplicates collapse.
    fn intern(&mut self, s: &str) -> u32 {
        if let Some(i) = self.string_pool.iter().position(|x| x == s) {
            return i as u32;
        }
        let id = self.string_pool.len() as u32;
        self.string_pool.push(s.to_string());
        id
    }

    /// push a tagged int constant, checking the i63 range at compile time.
    fn emit_int_const(&mut self, v: i64) -> Result<()> {
        if !value::fits_int(v) {
            return Err(Error::Parse(format!(
                "integer literal {v} doesn't fit in i63 (range {}..={})",
                value::INT_MIN,
                value::INT_MAX
            )));
        }
        self.emit(Op::Const(value::pack_int(v)));
        Ok(())
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
            Expr::Int(v) => self.emit_int_const(*v)?,
            Expr::Str(s, _span) => {
                let id = self.intern(s);
                self.emit(Op::Str(id));
            }
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
            Expr::Call(name, args, _span) => {
                // privileged builtins.
                if name == "len" {
                    if args.len() != 1 {
                        return Err(Error::Parse(format!(
                            "len() takes exactly 1 argument, got {}",
                            args.len()
                        )));
                    }
                    self.expr(&args[0])?;
                    self.emit(Op::StrLen);
                    return Ok(());
                }
                if name == "print" {
                    // print is usually a statement, but the call form is
                    // useful in expression position - return value is 0.
                    if args.len() != 1 {
                        return Err(Error::Parse(format!(
                            "print() takes exactly 1 argument, got {}",
                            args.len()
                        )));
                    }
                    self.expr(&args[0])?;
                    self.emit(Op::Print);
                    // print has no meaningful return; push tagged 0 so the
                    // expression has a value.
                    self.emit(Op::Const(value::pack_int(0)));
                    return Ok(());
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
                let callee_argc = args.len() as u8;
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
                        let jf = self.code.len();
                        self.emit(Op::JumpIfFalse(0));
                        self.expr(b)?;
                        self.emit(Op::Const(value::pack_int(0)));
                        self.emit(Op::Ne);
                        let jmp_end = self.code.len();
                        self.emit(Op::Jump(0));
                        let false_start = self.code.len();
                        self.patch_jif(jf, false_start);
                        self.emit(Op::Const(value::pack_int(0)));
                        let end = self.code.len();
                        self.patch_jmp(jmp_end, end);
                    }
                    BinOp::Or => {
                        self.expr(a)?;
                        let jf = self.code.len();
                        self.emit(Op::JumpIfFalse(0));
                        self.emit(Op::Const(value::pack_int(1)));
                        let jmp_end = self.code.len();
                        self.emit(Op::Jump(0));
                        let b_start = self.code.len();
                        self.patch_jif(jf, b_start);
                        self.expr(b)?;
                        self.emit(Op::Const(value::pack_int(0)));
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
