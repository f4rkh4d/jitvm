//! x86-64 template jit. one function -> one code blob. value stack lives in
//! an mmap'd i64 array; r15 points at its base, r14 is the top index, r13 is
//! the base index of the current frame's locals.
//!
//! only built on x86-64 linux/macos.
//!
//! 0.2: values on the val stack are **tagged** (see `src/value.rs`).
//! arithmetic stays in tagged form where possible:
//!
//! - add/sub on tagged ints: `(i<<1) + (j<<1) = (i+j)<<1`. plain add/sub.
//! - mul: `(i<<1) * (j<<1) = (i*j)<<2`. after imul, sar rax, 1 to untag once.
//! - div: `(i<<1) / (j<<1) = i/j` (untagged). shl rax, 1 to re-tag.
//! - mod: `(i<<1) % (j<<1) = (i%j)<<1`. already tagged.
//! - cmp/setcc: tagged cmp is the same as untagged for ints since shift
//!   is monotonic; setcc gives 0/1 in al, we shl rax, 1 to tag the result.
//! - neg: operates on the whole 64-bit register; `neg (i<<1) = -(i)<<1`. ok.
//!
//! these templates ASSUME both operands are tagged ints. the jit does NOT
//! do tag-dispatch in 0.2 - feeding it a pointer where it expects an int
//! is undefined. the interp (used as the correctness oracle) does check
//! tags. tag-dispatch + jit string arithmetic is v0.3 work.

use crate::ir::{Function, Op, Program};
use crate::x86::*;
use crate::{Error, Result};
use std::cell::RefCell;

thread_local! {
    pub static PRINT_CAPTURE: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// invoked from jitted code when a divisor is zero. rather than letting
/// `idiv rcx` raise SIGFPE (which the host hears as "the jit segfaulted"),
/// we detour here and exit cleanly with the same message the interpreter
/// would have produced. span info isn't threaded into the jit yet, so the
/// message is bare; see docs/jit-internals.md for the plan.
pub extern "sysv64" fn jit_div_by_zero() -> ! {
    eprintln!("runtime error: division by zero");
    unsafe { libc::_exit(1) }
}

/// print a tagged int. the int is passed already in its tagged form; we
/// unpack via `v >> 1` before formatting.
pub extern "sysv64" fn jit_print_int(v: i64) {
    let i = v >> 1;
    let captured = PRINT_CAPTURE.with(|c| {
        if let Some(b) = c.borrow_mut().as_mut() {
            b.push(i.to_string());
            true
        } else {
            false
        }
    });
    if !captured {
        println!("{i}");
    }
}

/// print a heap string. `ptr` is the header address; the `len` field is a
/// u32 at offset 0 and the bytes start at offset 8.
/// # Safety
///
/// `ptr` must point at a live `StrHeader`: a readable `u32` len at offset 0
/// followed by `len` readable bytes starting at offset 8. in the jit path
/// this is always an interned literal in the never-collected arena.
pub unsafe extern "sysv64" fn jit_print_str(ptr: *const u8) {
    // SAFETY: per the function-level contract on `ptr`.
    let (len, bytes) = unsafe {
        let len = std::ptr::read_unaligned(ptr as *const u32) as usize;
        let data = ptr.add(8);
        (len, std::slice::from_raw_parts(data, len))
    };
    let s = String::from_utf8_lossy(bytes).to_string();
    let captured = PRINT_CAPTURE.with(|c| {
        if let Some(b) = c.borrow_mut().as_mut() {
            b.push(s.clone());
            true
        } else {
            false
        }
    });
    if !captured {
        println!("{s}");
        let _ = len; // silence unused when stdout is a tty
    }
}

/// the jit's own never-collected literal arena. the JIT in 0.2 only
/// handles string literals (not runtime concat), so we can leak: allocate
/// every interned literal once at compile time, keep it alive for the
/// module's lifetime, and hand raw pointers into the emitted code.
///
/// this also sidesteps the safepoint + stackmap work that a collecting jit
/// would need; see `docs/v0.2-plan.md` for the rationale.
struct JitHeap {
    buf: Box<[u8]>,
    offsets: Vec<usize>,
}

impl JitHeap {
    fn build(pool: &[String]) -> Self {
        // layout each literal as u32 len, u32 hash, bytes, pad to 8.
        let mut bytes: Vec<u8> = Vec::new();
        let mut offsets: Vec<usize> = Vec::with_capacity(pool.len());
        for s in pool {
            // pad start to 8 so each header is 8-aligned (low 3 bits zero,
            // which our tag scheme relies on).
            while !bytes.len().is_multiple_of(8) {
                bytes.push(0);
            }
            offsets.push(bytes.len());
            let len = s.len() as u32;
            let hash = crate::heap::fnv1a(s.as_bytes());
            bytes.extend_from_slice(&len.to_le_bytes());
            bytes.extend_from_slice(&hash.to_le_bytes());
            bytes.extend_from_slice(s.as_bytes());
        }
        // final pad
        while !bytes.len().is_multiple_of(8) {
            bytes.push(0);
        }
        JitHeap {
            buf: bytes.into_boxed_slice(),
            offsets,
        }
    }

    fn ptr_of(&self, id: usize) -> *const u8 {
        unsafe { self.buf.as_ptr().add(self.offsets[id]) }
    }
}

pub struct CompiledModule {
    mem: *mut libc::c_void,
    size: usize,
    main_entry: usize,
    /// keep the literal arena alive for the lifetime of the module. the
    /// emitted code holds raw pointers into this box.
    #[allow(dead_code)]
    jit_heap: JitHeap,
}

unsafe impl Send for CompiledModule {}
unsafe impl Sync for CompiledModule {}

impl Drop for CompiledModule {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.mem, self.size);
        }
    }
}

impl CompiledModule {
    pub fn run_main(&self) -> Result<i64> {
        // 64k i64 slots = 512kb. enough for realistic recursion depths.
        let stack_len: usize = 1 << 16;
        let mut stack: Vec<i64> = vec![0; stack_len];
        let sp = stack.as_mut_ptr();
        let rv = unsafe { invoke(self.main_entry, sp) };
        drop(stack);
        // rv is tagged. unpack if it's an int; otherwise hand back raw
        // (caller is typically interactive and only looks at prints).
        if rv & 1 == 0 {
            Ok(rv >> 1)
        } else {
            Ok(rv)
        }
    }
}

unsafe fn invoke(entry: usize, stack_ptr: *mut i64) -> i64 {
    let rv: i64;
    unsafe {
        std::arch::asm!(
            "mov r15, {sp}",
            "xor r14d, r14d",
            "call {e}",
            sp = in(reg) stack_ptr,
            e = in(reg) entry,
            out("rax") rv,
            out("rcx") _, out("rdx") _, out("rsi") _, out("rdi") _,
            out("r8") _, out("r9") _, out("r10") _, out("r11") _,
            out("r12") _, out("r13") _, out("r14") _, out("r15") _,
            clobber_abi("sysv64"),
        );
    }
    rv
}

// Extra encodings we need for the jit, using the x86 module's low-level helpers.

fn emit(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(bytes);
}
fn emit_i32(buf: &mut Vec<u8>, v: i32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn inc_r14(b: &mut Vec<u8>) {
    emit(b, &[0x49, 0xFF, 0xC6]);
}
fn dec_r14(b: &mut Vec<u8>) {
    emit(b, &[0x49, 0xFF, 0xCE]);
}
fn add_r14_imm32(b: &mut Vec<u8>, v: i32) {
    emit(b, &[0x49, 0x81, 0xC6]);
    emit_i32(b, v);
}
fn sub_r13_imm32(b: &mut Vec<u8>, v: i32) {
    emit(b, &[0x49, 0x81, 0xED]);
    emit_i32(b, v);
}

// mov [r15 + r14*8], rax
fn store_rax_top(b: &mut Vec<u8>) {
    emit(b, &[0x4B, 0x89, 0x04, 0xF7]);
}
// mov rax, [r15 + r14*8]
fn load_rax_top(b: &mut Vec<u8>) {
    emit(b, &[0x4B, 0x8B, 0x04, 0xF7]);
}
// mov rcx, [r15 + r14*8]
fn load_rcx_top(b: &mut Vec<u8>) {
    emit(b, &[0x4B, 0x8B, 0x0C, 0xF7]);
}

fn push_val_rax(b: &mut Vec<u8>) {
    store_rax_top(b);
    inc_r14(b);
}
fn pop_val_rax(b: &mut Vec<u8>) {
    dec_r14(b);
    load_rax_top(b);
}
fn pop_val_rcx(b: &mut Vec<u8>) {
    dec_r14(b);
    load_rcx_top(b);
}

// mov rax, [r15 + r13*8 + disp32]
fn load_local_rax(b: &mut Vec<u8>, slot: u16) {
    emit(b, &[0x4B, 0x8B, 0x84, 0xEF]);
    emit_i32(b, (slot as i32) * 8);
}
// mov [r15 + r13*8 + disp32], rax
fn store_local_rax(b: &mut Vec<u8>, slot: u16) {
    emit(b, &[0x4B, 0x89, 0x84, 0xEF]);
    emit_i32(b, (slot as i32) * 8);
}
// mov r13, r14
fn mov_r13_r14(b: &mut Vec<u8>) {
    emit(b, &[0x4D, 0x89, 0xF5]);
}
// mov r14, r13
fn mov_r14_r13(b: &mut Vec<u8>) {
    emit(b, &[0x4D, 0x89, 0xEE]);
}

fn emit_divzero_guard(buf: &mut Vec<u8>) {
    test_reg_reg(buf, Reg::Rcx, Reg::Rcx);
    let jne_patch = jcc_rel32(buf, CC_NE, 0);
    push_reg(buf, Reg::Rdi);
    mov_reg_imm64(buf, Reg::Rax, jit_div_by_zero as *const () as i64);
    call_reg(buf, Reg::Rax);
    pop_reg(buf, Reg::Rdi);
    let skip = buf.len();
    patch_rel32(buf, jne_patch, skip);
}

struct CompiledFn {
    start: usize,
    op_offsets: Vec<usize>,
    jump_patches: Vec<(usize, usize)>,
    call_patches: Vec<(usize, u32)>,
}

fn emit_fn(buf: &mut Vec<u8>, f: &Function, jit_heap: &JitHeap) -> Result<CompiledFn> {
    let start = buf.len();
    let mut op_offsets = Vec::with_capacity(f.code.len() + 1);
    let mut jump_patches = Vec::new();
    let mut call_patches = Vec::new();

    push_reg(buf, Reg::Rbp);
    mov_reg_reg(buf, Reg::Rbp, Reg::Rsp);
    push_reg(buf, Reg::R13);
    push_reg(buf, Reg::Rdi);
    mov_r13_r14(buf);
    if f.argc > 0 {
        sub_r13_imm32(buf, f.argc as i32);
    }
    if f.locals > 0 {
        add_r14_imm32(buf, f.locals as i32);
    }

    for (i, op) in f.code.iter().enumerate() {
        op_offsets.push(buf.len());
        match *op {
            Op::Const(n) => {
                // n is already the tagged representation; the lowerer
                // shifted int literals before emitting.
                mov_reg_imm64(buf, Reg::Rax, n);
                push_val_rax(buf);
            }
            Op::Pop => {
                dec_r14(buf);
            }
            Op::LoadLocal(s) => {
                load_local_rax(buf, s);
                push_val_rax(buf);
            }
            Op::LoadArg(a) => {
                load_local_rax(buf, a as u16);
                push_val_rax(buf);
            }
            Op::StoreLocal(s) => {
                pop_val_rax(buf);
                store_local_rax(buf, s);
            }
            Op::Add => {
                // tagged-form add works directly.
                pop_val_rcx(buf);
                pop_val_rax(buf);
                add_reg_reg(buf, Reg::Rax, Reg::Rcx);
                push_val_rax(buf);
            }
            Op::Sub => {
                pop_val_rcx(buf);
                pop_val_rax(buf);
                sub_reg_reg(buf, Reg::Rax, Reg::Rcx);
                push_val_rax(buf);
            }
            Op::Mul => {
                // (i<<1)*(j<<1) = (i*j)<<2, so shift right once to get
                // the correctly-tagged product.
                pop_val_rcx(buf);
                pop_val_rax(buf);
                imul_reg_reg(buf, Reg::Rax, Reg::Rcx);
                sar_reg_1(buf, Reg::Rax);
                push_val_rax(buf);
            }
            Op::Div => {
                // (i<<1) / (j<<1) = i/j untagged. re-tag with shl.
                pop_val_rcx(buf);
                pop_val_rax(buf);
                emit_divzero_guard(buf);
                // untag both sides first so idiv operates on real ints.
                sar_reg_1(buf, Reg::Rax);
                sar_reg_1(buf, Reg::Rcx);
                cqo(buf);
                idiv_reg(buf, Reg::Rcx);
                shl_reg_1(buf, Reg::Rax);
                push_val_rax(buf);
            }
            Op::Mod => {
                // same idea: untag, compute, remainder is untagged, re-tag.
                pop_val_rcx(buf);
                pop_val_rax(buf);
                emit_divzero_guard(buf);
                sar_reg_1(buf, Reg::Rax);
                sar_reg_1(buf, Reg::Rcx);
                cqo(buf);
                idiv_reg(buf, Reg::Rcx);
                mov_reg_reg(buf, Reg::Rax, Reg::Rdx);
                shl_reg_1(buf, Reg::Rax);
                push_val_rax(buf);
            }
            Op::Neg => {
                // neg on the full 64-bit register is still correct: the
                // sign flips, low tag bit flips 0->0 (int stays int).
                pop_val_rax(buf);
                neg_reg(buf, Reg::Rax);
                push_val_rax(buf);
            }
            Op::Not => {
                pop_val_rax(buf);
                test_reg_reg(buf, Reg::Rax, Reg::Rax);
                setcc_al(buf, CC_E);
                movzx_rax_al(buf);
                // setcc gives 0/1 in al. to produce a tagged int we need
                // to shift left by 1 so the result is 0 (false) or 2 (true).
                shl_reg_1(buf, Reg::Rax);
                push_val_rax(buf);
            }
            Op::Lt | Op::Le | Op::Gt | Op::Ge | Op::Eq | Op::Ne => {
                pop_val_rcx(buf);
                pop_val_rax(buf);
                cmp_reg_reg(buf, Reg::Rax, Reg::Rcx);
                let cc = match *op {
                    Op::Lt => CC_L,
                    Op::Le => CC_LE,
                    Op::Gt => CC_G,
                    Op::Ge => CC_GE,
                    Op::Eq => CC_E,
                    Op::Ne => CC_NE,
                    _ => unreachable!(),
                };
                setcc_al(buf, cc);
                movzx_rax_al(buf);
                // tag the 0/1 result.
                shl_reg_1(buf, Reg::Rax);
                push_val_rax(buf);
            }
            Op::Jump(rel) => {
                let target_op = (i as isize + 1 + rel as isize) as usize;
                let patch = jmp_rel32(buf, 0);
                jump_patches.push((patch, target_op));
            }
            Op::JumpIfFalse(rel) => {
                let target_op = (i as isize + 1 + rel as isize) as usize;
                pop_val_rax(buf);
                // tagged int 0 == raw 0; any pointer has low bit set so
                // it's non-zero; tagged int 1 is raw 2 (non-zero). so the
                // plain `test rax, rax; jz` check is correct for truthy.
                test_reg_reg(buf, Reg::Rax, Reg::Rax);
                let patch = jcc_rel32(buf, CC_E, 0);
                jump_patches.push((patch, target_op));
            }
            Op::Call(fn_id, _argc) => {
                push_reg(buf, Reg::R13);
                push_reg(buf, Reg::Rdi); // alignment pad
                let patch = call_rel32(buf, 0);
                call_patches.push((patch, fn_id));
                pop_reg(buf, Reg::Rdi);
                pop_reg(buf, Reg::R13);
            }
            Op::Ret => {
                pop_val_rax(buf); // ret value
                mov_r14_r13(buf);
                push_val_rax(buf);
                mov_reg_reg(buf, Reg::Rsp, Reg::Rbp);
                pop_reg(buf, Reg::Rbp);
                ret(buf);
            }
            Op::Print => {
                // tag-dispatched print. pop value into rax; branch on the
                // low bit. int goes to jit_print_int (which shifts off the
                // tag); ptr goes to jit_print_str (mask low bit off first).
                //
                //   pop_val_rax
                //   test rax, 1
                //   jz  print_int
                //   <set rdi = rax & !1>; call jit_print_str
                //   jmp done
                // print_int:
                //   <set rdi = rax>; call jit_print_int
                // done:
                pop_val_rax(buf);
                test_reg_imm32(buf, Reg::Rax, 1);
                // jz print_int  (zero flag set means low bit was 0 = int)
                let jz_patch = jcc_rel32(buf, CC_E, 0);

                // ---- string path ----
                mov_reg_reg(buf, Reg::Rdi, Reg::Rax);
                and_reg_imm8(buf, Reg::Rdi, !1i8);
                push_reg(buf, Reg::R13);
                push_reg(buf, Reg::R14);
                push_reg(buf, Reg::R15);
                push_reg(buf, Reg::Rdi); // pad + preserve rdi
                mov_reg_imm64(buf, Reg::Rax, jit_print_str as *const () as i64);
                call_reg(buf, Reg::Rax);
                pop_reg(buf, Reg::Rdi);
                pop_reg(buf, Reg::R15);
                pop_reg(buf, Reg::R14);
                pop_reg(buf, Reg::R13);
                let jmp_done = jmp_rel32(buf, 0);

                // ---- int path ----
                let int_start = buf.len();
                patch_rel32(buf, jz_patch, int_start);
                mov_reg_reg(buf, Reg::Rdi, Reg::Rax);
                push_reg(buf, Reg::R13);
                push_reg(buf, Reg::R14);
                push_reg(buf, Reg::R15);
                push_reg(buf, Reg::Rdi);
                mov_reg_imm64(buf, Reg::Rax, jit_print_int as *const () as i64);
                call_reg(buf, Reg::Rax);
                pop_reg(buf, Reg::Rdi);
                pop_reg(buf, Reg::R15);
                pop_reg(buf, Reg::R14);
                pop_reg(buf, Reg::R13);

                let done = buf.len();
                patch_rel32(buf, jmp_done, done);
            }
            Op::Str(id) => {
                // bake the interned-literal pointer in as an imm64.
                // or-in tag bit 1 to produce a tagged pointer.
                let p = jit_heap.ptr_of(id as usize) as i64;
                debug_assert_eq!(p & 1, 0, "jit literal not 8-aligned");
                mov_reg_imm64(buf, Reg::Rax, p | 1);
                push_val_rax(buf);
            }
            Op::StrLen => {
                // pop tagged ptr, mask tag, read u32 len, shift left 1.
                pop_val_rax(buf);
                and_reg_imm8(buf, Reg::Rax, !1i8);
                // mov eax, [rax]   (zero-extends to rax)
                mov_r32_from_mem_reg(buf, Reg::Rax, Reg::Rax);
                // tag: shl rax, 1.
                shl_reg_1(buf, Reg::Rax);
                push_val_rax(buf);
            }
            Op::Concat => {
                // deferred to v0.3; see docs/v0.2-plan.md. the programs
                // that need runtime concat run through the interp.
                return Err(Error::Codegen(
                    "jit does not support runtime string concat in v0.2 \
                     (run with --interp for programs that concat at runtime)"
                        .into(),
                ));
            }
        }
    }
    op_offsets.push(buf.len());

    Ok(CompiledFn {
        start,
        op_offsets,
        jump_patches,
        call_patches,
    })
}

fn page_round_up(n: usize) -> usize {
    let page = 4096;
    (n + page - 1) & !(page - 1)
}

pub fn compile(prog: &Program) -> Result<CompiledModule> {
    let jit_heap = JitHeap::build(&prog.string_pool);

    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut compiled: Vec<CompiledFn> = Vec::with_capacity(prog.fns.len());
    for f in &prog.fns {
        compiled.push(emit_fn(&mut buf, f, &jit_heap)?);
    }

    for cc in &compiled {
        for &(at, target_op) in &cc.jump_patches {
            let target = cc.op_offsets[target_op];
            patch_rel32(&mut buf, at, target);
        }
    }

    for cc in &compiled {
        for &(at, callee) in &cc.call_patches {
            let target = compiled[callee as usize].start;
            patch_rel32(&mut buf, at, target);
        }
    }

    let size = page_round_up(buf.len().max(1));
    #[cfg(target_os = "macos")]
    let (flags, prot) = (
        libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_JIT,
        libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
    );
    #[cfg(not(target_os = "macos"))]
    let (flags, prot) = (
        libc::MAP_ANON | libc::MAP_PRIVATE,
        libc::PROT_READ | libc::PROT_WRITE,
    );

    let mem = unsafe { libc::mmap(std::ptr::null_mut(), size, prot, flags, -1, 0) };
    if mem == libc::MAP_FAILED {
        return Err(Error::Codegen("mmap failed".into()));
    }

    unsafe {
        #[cfg(target_os = "macos")]
        jit_write_protect(false);
        std::ptr::copy_nonoverlapping(buf.as_ptr(), mem as *mut u8, buf.len());
        #[cfg(target_os = "macos")]
        jit_write_protect(true);

        #[cfg(not(target_os = "macos"))]
        {
            let r = libc::mprotect(mem, size, libc::PROT_READ | libc::PROT_EXEC);
            if r != 0 {
                libc::munmap(mem, size);
                return Err(Error::Codegen("mprotect failed".into()));
            }
        }

        #[cfg(target_os = "macos")]
        sys_icache_invalidate(mem, size);
    }

    let base = mem as usize;
    let main_entry = base + compiled[prog.main_id as usize].start;
    Ok(CompiledModule {
        mem,
        size,
        main_entry,
        jit_heap,
    })
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn pthread_jit_write_protect_np(enabled: libc::c_int);
    fn sys_icache_invalidate(start: *mut libc::c_void, len: libc::size_t);
}

#[cfg(target_os = "macos")]
#[inline]
unsafe fn jit_write_protect(enabled: bool) {
    unsafe { pthread_jit_write_protect_np(if enabled { 1 } else { 0 }) };
}

pub fn capture_prints<F: FnOnce() -> Result<i64>>(f: F) -> Result<Vec<String>> {
    PRINT_CAPTURE.with(|c| *c.borrow_mut() = Some(Vec::new()));
    let r = f();
    let out = PRINT_CAPTURE.with(|c| c.borrow_mut().take().unwrap_or_default());
    r?;
    Ok(out)
}
