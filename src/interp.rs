//! tree-walking-ish stack interpreter over the bytecode.
//! used as the oracle for jit tests and the baseline for the benchmark.

use crate::ast::Span;
use crate::ir::{Function, Op, Program};
use crate::{Error, Result};
use std::cell::RefCell;

thread_local! {
    static PRINT_CAPTURE: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

pub fn capture_prints<F: FnOnce() -> Result<i64>>(f: F) -> Result<Vec<String>> {
    PRINT_CAPTURE.with(|c| *c.borrow_mut() = Some(Vec::new()));
    let r = f();
    let out = PRINT_CAPTURE.with(|c| c.borrow_mut().take().unwrap_or_default());
    r?;
    Ok(out)
}

fn do_print(v: i64) {
    let captured = PRINT_CAPTURE.with(|c| {
        if let Some(b) = c.borrow_mut().as_mut() {
            b.push(v.to_string());
            true
        } else {
            false
        }
    });
    if !captured {
        println!("{v}");
    }
}

pub fn run(prog: &Program) -> Result<i64> {
    let main = &prog.fns[prog.main_id as usize];
    if main.argc != 0 {
        return Err(Error::runtime("main() must take no arguments"));
    }
    call(prog, main, &[])
}

fn call(prog: &Program, f: &Function, args: &[i64]) -> Result<i64> {
    if args.len() != f.argc as usize {
        return Err(Error::runtime(format!(
            "arity mismatch calling {}: expected {}, got {}",
            f.name,
            f.argc,
            args.len()
        )));
    }
    // frame: args + locals. slots are [args..., locals...].
    let mut slots: Vec<i64> = Vec::with_capacity(f.argc as usize + f.locals as usize);
    slots.extend_from_slice(args);
    slots.resize(f.argc as usize + f.locals as usize, 0);

    let mut stack: Vec<i64> = Vec::with_capacity(32);
    let mut pc: usize = 0;

    loop {
        let op = f.code[pc];
        let span = f.spans.get(pc).copied().unwrap_or(Span::UNKNOWN);
        pc += 1;
        match op {
            Op::Const(v) => stack.push(v),
            Op::LoadLocal(s) => {
                let v = slots[s as usize];
                stack.push(v);
            }
            Op::StoreLocal(s) => {
                let v = stack
                    .pop()
                    .ok_or_else(|| Error::runtime("stack underflow"))?;
                slots[s as usize] = v;
            }
            Op::LoadArg(a) => {
                let v = slots[a as usize];
                stack.push(v);
            }
            Op::Add => binop(&mut stack, |a, b| a.wrapping_add(b))?,
            Op::Sub => binop(&mut stack, |a, b| a.wrapping_sub(b))?,
            Op::Mul => binop(&mut stack, |a, b| a.wrapping_mul(b))?,
            Op::Div => {
                let b = stack.pop().ok_or_else(|| Error::runtime("underflow"))?;
                let a = stack.pop().ok_or_else(|| Error::runtime("underflow"))?;
                if b == 0 {
                    return Err(Error::runtime_at("division by zero", span));
                }
                stack.push(a.wrapping_div(b));
            }
            Op::Mod => {
                let b = stack.pop().ok_or_else(|| Error::runtime("underflow"))?;
                let a = stack.pop().ok_or_else(|| Error::runtime("underflow"))?;
                if b == 0 {
                    return Err(Error::runtime_at("mod by zero", span));
                }
                stack.push(a.wrapping_rem(b));
            }
            Op::Neg => {
                let v = stack.pop().ok_or_else(|| Error::runtime("underflow"))?;
                stack.push(v.wrapping_neg());
            }
            Op::Not => {
                let v = stack.pop().ok_or_else(|| Error::runtime("underflow"))?;
                stack.push(if v == 0 { 1 } else { 0 });
            }
            Op::Lt => cmp(&mut stack, |a, b| a < b)?,
            Op::Le => cmp(&mut stack, |a, b| a <= b)?,
            Op::Gt => cmp(&mut stack, |a, b| a > b)?,
            Op::Ge => cmp(&mut stack, |a, b| a >= b)?,
            Op::Eq => cmp(&mut stack, |a, b| a == b)?,
            Op::Ne => cmp(&mut stack, |a, b| a != b)?,
            Op::Jump(off) => {
                pc = (pc as i32 + off) as usize;
            }
            Op::JumpIfFalse(off) => {
                let v = stack.pop().ok_or_else(|| Error::runtime("underflow"))?;
                if v == 0 {
                    pc = (pc as i32 + off) as usize;
                }
            }
            Op::Call(id, argc) => {
                let callee = &prog.fns[id as usize];
                if callee.argc != argc {
                    return Err(Error::runtime_at(
                        format!(
                            "arity mismatch calling {}: expected {}, got {}",
                            callee.name, callee.argc, argc
                        ),
                        span,
                    ));
                }
                let at = stack.len() - argc as usize;
                let args: Vec<i64> = stack.split_off(at);
                let r = call(prog, callee, &args)?;
                stack.push(r);
            }
            Op::Ret => {
                return Ok(stack.pop().unwrap_or(0));
            }
            Op::Print => {
                let v = stack.pop().ok_or_else(|| Error::runtime("underflow"))?;
                do_print(v);
            }
            Op::Pop => {
                stack.pop();
            }
        }
    }
}

fn binop(stack: &mut Vec<i64>, f: impl FnOnce(i64, i64) -> i64) -> Result<()> {
    let b = stack.pop().ok_or_else(|| Error::runtime("underflow"))?;
    let a = stack.pop().ok_or_else(|| Error::runtime("underflow"))?;
    stack.push(f(a, b));
    Ok(())
}

fn cmp(stack: &mut Vec<i64>, f: impl FnOnce(i64, i64) -> bool) -> Result<()> {
    let b = stack.pop().ok_or_else(|| Error::runtime("underflow"))?;
    let a = stack.pop().ok_or_else(|| Error::runtime("underflow"))?;
    stack.push(if f(a, b) { 1 } else { 0 });
    Ok(())
}
