//! tree-walking-ish stack interpreter over the bytecode.
//! used as the oracle for jit tests and the baseline for the benchmark.
//!
//! values on the stack and in slots are tagged (see `src/value.rs`). every
//! arithmetic op checks tags on the way in and re-tags on the way out.

use crate::ast::Span;
use crate::heap::Heap;
use crate::ir::{Function, Op, Program};
use crate::value::{self, Value};
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

fn do_print_str(s: &str) {
    let captured = PRINT_CAPTURE.with(|c| {
        if let Some(b) = c.borrow_mut().as_mut() {
            b.push(s.to_string());
            true
        } else {
            false
        }
    });
    if !captured {
        println!("{s}");
    }
}

/// interpreter context. owns the heap and the interned-literal table.
pub struct Context {
    pub heap: Heap,
    pub interned_ptrs: Vec<*const u8>,
}

impl Context {
    pub fn new(prog: &Program) -> Self {
        let mut heap = Heap::new();
        let mut interned_ptrs = Vec::with_capacity(prog.string_pool.len());
        // pre-allocate every literal. these count as roots for the whole
        // program's lifetime; we store them in `interned_ptrs` and they're
        // never collected (the simplest thing that works for 0.2).
        for s in &prog.string_pool {
            let mut no_roots: Vec<Value> = Vec::new();
            let p = heap.alloc_str(s.as_bytes(), &mut no_roots);
            interned_ptrs.push(p);
        }
        Context {
            heap,
            interned_ptrs,
        }
    }
}

pub fn run(prog: &Program) -> Result<i64> {
    let main = &prog.fns[prog.main_id as usize];
    if main.argc != 0 {
        return Err(Error::runtime("main() must take no arguments"));
    }
    let mut ctx = Context::new(prog);
    let rv = call(prog, &mut ctx, main, &[])?;
    // unpack the return value if it's an int (the main use site is `cargo
    // run` printing a status code). if main returned a pointer, the return
    // "int" is the raw tagged value; tests don't care.
    Ok(value::unpack_int(rv).unwrap_or(rv))
}

fn call(prog: &Program, ctx: &mut Context, f: &Function, args: &[Value]) -> Result<Value> {
    if args.len() != f.argc as usize {
        return Err(Error::runtime(format!(
            "arity mismatch calling {}: expected {}, got {}",
            f.name,
            f.argc,
            args.len()
        )));
    }
    // frame: args + locals. slots are [args..., locals...].
    let mut slots: Vec<Value> = Vec::with_capacity(f.argc as usize + f.locals as usize);
    slots.extend_from_slice(args);
    slots.resize(f.argc as usize + f.locals as usize, 0);

    let mut stack: Vec<Value> = Vec::with_capacity(32);
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
            Op::Add => {
                let b = pop(&mut stack)?;
                let a = pop(&mut stack)?;
                if value::is_ptr(a) && value::is_ptr(b) {
                    // string concat. collect may trigger inside alloc_str
                    // and rewrite any live roots; we pass `stack` and
                    // `slots` directly so their pointers stay valid.
                    let new_p = concat_alloc(ctx, a, b, &mut stack, &mut slots);
                    stack.push(value::pack_ptr(new_p as *const ()));
                } else if value::is_int(a) && value::is_int(b) {
                    // tagged-form add: (i<<1) + (j<<1) = (i+j)<<1.
                    stack.push(a.wrapping_add(b));
                } else {
                    return Err(Error::runtime_at(
                        "type error: + requires two ints or two strings",
                        span,
                    ));
                }
            }
            Op::Sub => arith_int(&mut stack, span, "-", |a, b| a.wrapping_sub(b))?,
            Op::Mul => {
                // tagged-form mul: (i<<1) * (j<<1) = (i*j)<<2. recover by
                // shifting right one bit; arithmetic shift keeps the sign.
                let b = pop(&mut stack)?;
                let a = pop(&mut stack)?;
                check_int(a, span, "*")?;
                check_int(b, span, "*")?;
                let r = a.wrapping_mul(b) >> 1;
                stack.push(r);
            }
            Op::Div => {
                let b = pop(&mut stack)?;
                let a = pop(&mut stack)?;
                check_int(a, span, "/")?;
                check_int(b, span, "/")?;
                if b == 0 {
                    return Err(Error::runtime_at("division by zero", span));
                }
                // tagged-form div: (i<<1) / (j<<1) = i/j (untagged). retag.
                let q = (a.wrapping_div(b)) << 1;
                stack.push(q);
            }
            Op::Mod => {
                let b = pop(&mut stack)?;
                let a = pop(&mut stack)?;
                check_int(a, span, "%")?;
                check_int(b, span, "%")?;
                if b == 0 {
                    return Err(Error::runtime_at("mod by zero", span));
                }
                // (i<<1) % (j<<1) = (i%j) << 1, already tagged.
                stack.push(a.wrapping_rem(b));
            }
            Op::Neg => {
                let v = pop(&mut stack)?;
                check_int(v, span, "unary -")?;
                stack.push(v.wrapping_neg());
            }
            Op::Not => {
                let v = pop(&mut stack)?;
                // not treats any non-zero value (tagged int or ptr) as
                // truthy; pointers always have low bit 1 so are non-zero.
                let r = if v == 0 {
                    value::pack_int(1)
                } else {
                    value::pack_int(0)
                };
                stack.push(r);
            }
            Op::Lt => cmp_tagged(&mut stack, span, |a, b| a < b)?,
            Op::Le => cmp_tagged(&mut stack, span, |a, b| a <= b)?,
            Op::Gt => cmp_tagged(&mut stack, span, |a, b| a > b)?,
            Op::Ge => cmp_tagged(&mut stack, span, |a, b| a >= b)?,
            Op::Eq => {
                // equality is defined for any pair of values. both-ptr
                // compares content; both-int compares raw; mixed = 0.
                let b = pop(&mut stack)?;
                let a = pop(&mut stack)?;
                let eq = if value::is_ptr(a) && value::is_ptr(b) {
                    let pa = (a & !1) as *const u8;
                    let pb = (b & !1) as *const u8;
                    ctx.heap.bytes_of(pa) == ctx.heap.bytes_of(pb)
                } else if value::is_int(a) && value::is_int(b) {
                    a == b
                } else {
                    false
                };
                stack.push(if eq {
                    value::pack_int(1)
                } else {
                    value::pack_int(0)
                });
            }
            Op::Ne => {
                let b = pop(&mut stack)?;
                let a = pop(&mut stack)?;
                let eq = if value::is_ptr(a) && value::is_ptr(b) {
                    let pa = (a & !1) as *const u8;
                    let pb = (b & !1) as *const u8;
                    ctx.heap.bytes_of(pa) == ctx.heap.bytes_of(pb)
                } else if value::is_int(a) && value::is_int(b) {
                    a == b
                } else {
                    false
                };
                stack.push(if eq {
                    value::pack_int(0)
                } else {
                    value::pack_int(1)
                });
            }
            Op::Jump(off) => {
                pc = (pc as i32 + off) as usize;
            }
            Op::JumpIfFalse(off) => {
                let v = pop(&mut stack)?;
                // "false" means tagged int 0. a tagged pointer is never 0
                // (low bit is 1) so it's always truthy. a tagged int of 0
                // has raw bits 0, so the same `== 0` check works.
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
                let args: Vec<Value> = stack.split_off(at);
                let r = call(prog, ctx, callee, &args)?;
                stack.push(r);
            }
            Op::Ret => {
                return Ok(stack.pop().unwrap_or(0));
            }
            Op::Print => {
                let v = pop(&mut stack)?;
                if value::is_ptr(v) {
                    let p = (v & !1) as *const u8;
                    let bytes = ctx.heap.bytes_of(p);
                    // bytes may contain arbitrary utf-8; we printed via
                    // push-string so we accept a lossy conversion.
                    do_print_str(&String::from_utf8_lossy(bytes));
                } else {
                    let i = v >> 1;
                    do_print_str(&i.to_string());
                }
            }
            Op::Pop => {
                stack.pop();
            }
            Op::Str(id) => {
                let p = ctx.interned_ptrs[id as usize];
                stack.push(value::pack_ptr(p as *const ()));
            }
            Op::Concat => {
                let b = pop(&mut stack)?;
                let a = pop(&mut stack)?;
                if !(value::is_ptr(a) && value::is_ptr(b)) {
                    return Err(Error::runtime_at("concat on non-strings", span));
                }
                let new_p = concat_alloc(ctx, a, b, &mut stack, &mut slots);
                stack.push(value::pack_ptr(new_p as *const ()));
            }
            Op::StrLen => {
                let v = pop(&mut stack)?;
                if !value::is_ptr(v) {
                    return Err(Error::runtime_at("len() on non-string", span));
                }
                let p = (v & !1) as *const u8;
                let len = ctx.heap.len_of(p) as i64;
                stack.push(value::pack_int(len));
            }
        }
    }
}

/// allocate `bytes_of(a) + bytes_of(b)` on the heap. if the arena crosses
/// the collection threshold during the alloc, `stack` and `slots` are
/// passed as roots so their pointers are rewritten in place; pushing the
/// result afterwards is always safe.
fn concat_alloc(
    ctx: &mut Context,
    a: Value,
    b: Value,
    stack: &mut [Value],
    slots: &mut [Value],
) -> *const u8 {
    let pa = (a & !1) as *const u8;
    let pb = (b & !1) as *const u8;
    let mut combined: Vec<u8> = Vec::new();
    combined.extend_from_slice(ctx.heap.bytes_of(pa));
    combined.extend_from_slice(ctx.heap.bytes_of(pb));
    // build a single flat slice for collect to walk. we include a and b
    // up front because at this point they have been popped from `stack`
    // but we haven't yet written the result anywhere - if collect fires
    // without them as roots we'd still be fine (we already copied the
    // bytes out), but including them is harmless.
    //
    // to pass both stack and slots as one &mut [Value], we use a small
    // scratch vec of pointers... no, easier: concat them into a single
    // root vec with extra entries for a and b, do the alloc, then rewrite.
    // but we specifically need in-place rewrite of stack/slots.
    //
    // trick: call collect explicitly first if the threshold would fire,
    // then alloc (which won't retrigger since the arena just shrank).
    let need = crate::heap::HEADER_SIZE + combined.len();
    let would_collect = ctx.heap.bytes_in_use() + need > ctx.heap.collect_threshold_for_test();
    if would_collect {
        // root set: stack + slots. a and b are stale post-pop and we
        // already extracted their bytes, so we don't need them rooted.
        let n_stack = stack.len();
        let n_slots = slots.len();
        let mut roots = Vec::with_capacity(n_stack + n_slots);
        roots.extend_from_slice(stack);
        roots.extend_from_slice(slots);
        ctx.heap.collect(&mut roots);
        // write back.
        stack.copy_from_slice(&roots[..n_stack]);
        slots.copy_from_slice(&roots[n_stack..n_stack + n_slots]);
    }
    // now alloc into the (possibly freshly-collected) arena. no further
    // collection will trigger inside this call since we already made room.
    let mut no_roots: Vec<Value> = Vec::new();
    ctx.heap.alloc_str(&combined, &mut no_roots)
}

fn pop(stack: &mut Vec<Value>) -> Result<Value> {
    stack.pop().ok_or_else(|| Error::runtime("stack underflow"))
}

fn check_int(v: Value, span: Span, what: &str) -> Result<()> {
    if value::is_int(v) {
        Ok(())
    } else {
        Err(Error::runtime_at(
            format!("type error: {what} on non-int"),
            span,
        ))
    }
}

fn arith_int(
    stack: &mut Vec<Value>,
    span: Span,
    what: &str,
    f: impl FnOnce(i64, i64) -> i64,
) -> Result<()> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    check_int(a, span, what)?;
    check_int(b, span, what)?;
    stack.push(f(a, b));
    Ok(())
}

fn cmp_tagged(stack: &mut Vec<Value>, span: Span, f: impl FnOnce(i64, i64) -> bool) -> Result<()> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    check_int(a, span, "cmp")?;
    check_int(b, span, "cmp")?;
    // both tagged ints. raw comparison is the same as unpacked comparison
    // because shifting by 1 is a monotonic transform.
    stack.push(if f(a, b) {
        value::pack_int(1)
    } else {
        value::pack_int(0)
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Engine;

    fn run_capture(src: &str) -> Vec<String> {
        let eng = Engine::load_str(src).expect("parse");
        capture_prints(|| run(&eng.program)).expect("run")
    }

    #[test]
    fn str_literal_print() {
        assert_eq!(run_capture(r#"print "hi""#), vec!["hi"]);
    }

    #[test]
    fn str_concat_literal() {
        assert_eq!(
            run_capture(r#"print "hello, " + "world""#),
            vec!["hello, world"]
        );
    }

    #[test]
    fn str_len() {
        assert_eq!(run_capture(r#"print len("abcde")"#), vec!["5"]);
        assert_eq!(run_capture(r#"print len("")"#), vec!["0"]);
    }

    #[test]
    fn str_eq() {
        assert_eq!(run_capture(r#"print "a" == "a""#), vec!["1"]);
        assert_eq!(run_capture(r#"print "a" == "b""#), vec!["0"]);
        // dedup in the pool means these two literals alias; content-eq
        // still holds regardless.
        assert_eq!(run_capture(r#"print "abc" != "abd""#), vec!["1"]);
    }

    #[test]
    fn str_repeat_concat_loop() {
        let src = r#"
            let s = ""
            let i = 0
            while i < 5 {
                s = s + "x"
                i = i + 1
            }
            print len(s)
            print s
        "#;
        assert_eq!(run_capture(src), vec!["5", "xxxxx"]);
    }
}
