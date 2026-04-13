//! x86-64 template jit. one function -> one code blob. value stack lives in
//! an mmap'd i64 array; r15 points at its base, r14 is the top index, r13 is
//! the base index of the current frame's locals.
//!
//! only built on x86-64 linux/macos.

use crate::ir::{Function, Op, Program};
use crate::x86::*;
use crate::{Error, Result};
use std::cell::RefCell;

thread_local! {
    pub static PRINT_CAPTURE: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

pub extern "sysv64" fn jit_print(v: i64) {
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

pub struct CompiledModule {
    mem: *mut libc::c_void,
    size: usize,
    main_entry: usize,
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
        Ok(rv)
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
// The x86 module mostly covers reg/reg ops; here we add r14 imm ops and
// sib-memory addressing for [r15 + r14*8] and [r15 + r13*8 + disp].

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

// REX.W + REX.X + REX.B = 0x4B for SIB ops where both index and base are r8-r15.
// earlier versions had 0x4A here (missing REX.B for the r15 base); the effect
// was that [r15 + ...] accesses silently went through rdi. they worked by luck
// while rdi still held the initial stack_ptr value that rust's asm block
// happened to place there, then crashed the moment jit_print clobbered rdi.
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

struct CompiledFn {
    start: usize,
    op_offsets: Vec<usize>,
    jump_patches: Vec<(usize, usize)>,
    call_patches: Vec<(usize, u32)>,
}

fn emit_fn(buf: &mut Vec<u8>, f: &Function) -> CompiledFn {
    let start = buf.len();
    let mut op_offsets = Vec::with_capacity(f.code.len() + 1);
    let mut jump_patches = Vec::new();
    let mut call_patches = Vec::new();

    // prologue. stack alignment on entry is 8 mod 16 (retaddr just pushed by call).
    // push rbp (+8) -> 0 mod 16. push r13 (+8) -> 8 mod 16. push rdi (+8) -> 0 mod 16.
    // so inside the body rsp is 16-aligned, which is what sysv requires at a call site.
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
                pop_val_rcx(buf);
                pop_val_rax(buf);
                imul_reg_reg(buf, Reg::Rax, Reg::Rcx);
                push_val_rax(buf);
            }
            Op::Div => {
                pop_val_rcx(buf);
                pop_val_rax(buf);
                cqo(buf);
                idiv_reg(buf, Reg::Rcx);
                push_val_rax(buf);
            }
            Op::Mod => {
                pop_val_rcx(buf);
                pop_val_rax(buf);
                cqo(buf);
                idiv_reg(buf, Reg::Rcx);
                mov_reg_reg(buf, Reg::Rax, Reg::Rdx);
                push_val_rax(buf);
            }
            Op::Neg => {
                pop_val_rax(buf);
                neg_reg(buf, Reg::Rax);
                push_val_rax(buf);
            }
            Op::Not => {
                pop_val_rax(buf);
                test_reg_reg(buf, Reg::Rax, Reg::Rax);
                setcc_al(buf, CC_E);
                movzx_rax_al(buf);
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
                test_reg_reg(buf, Reg::Rax, Reg::Rax);
                let patch = jcc_rel32(buf, CC_E, 0);
                jump_patches.push((patch, target_op));
            }
            Op::Call(fn_id, _argc) => {
                // rsp is 16-aligned inside the body. save r13 (+8) and pad (+8)
                // so rsp stays 16-aligned before the call instruction.
                push_reg(buf, Reg::R13);
                push_reg(buf, Reg::Rdi); // alignment pad
                let patch = call_rel32(buf, 0);
                call_patches.push((patch, fn_id));
                pop_reg(buf, Reg::Rdi);
                pop_reg(buf, Reg::R13);
            }
            Op::Ret => {
                pop_val_rax(buf); // ret value
                mov_r14_r13(buf); // discard this frame's slots
                push_val_rax(buf); // push ret value for caller
                                   // epilogue. mov rsp, rbp + pop rbp undoes rbp push; but we still
                                   // have our extra push rdi and push r13. mov rsp, rbp actually
                                   // jumps over them all in one go.
                mov_reg_reg(buf, Reg::Rsp, Reg::Rbp);
                pop_reg(buf, Reg::Rbp);
                ret(buf);
            }
            Op::Print => {
                pop_val_rax(buf);
                mov_reg_reg(buf, Reg::Rdi, Reg::Rax);
                // callee-saved per sysv: rbx, rbp, r12-r15. jit_print is a
                // rust fn and *should* preserve these, but some builds i've
                // hit seem to leak r13/r14/r15 through the thread_local+
                // format machinery. save them explicitly to be robust.
                // rsp is 16-aligned inside the body; 4 pushes keep it so.
                push_reg(buf, Reg::R13);
                push_reg(buf, Reg::R14);
                push_reg(buf, Reg::R15);
                push_reg(buf, Reg::Rdi); // alignment pad + preserves rdi
                mov_reg_imm64(buf, Reg::Rax, jit_print as *const () as i64);
                call_reg(buf, Reg::Rax);
                pop_reg(buf, Reg::Rdi);
                pop_reg(buf, Reg::R15);
                pop_reg(buf, Reg::R14);
                pop_reg(buf, Reg::R13);
            }
        }
    }
    op_offsets.push(buf.len());

    CompiledFn {
        start,
        op_offsets,
        jump_patches,
        call_patches,
    }
}

fn page_round_up(n: usize) -> usize {
    let page = 4096;
    (n + page - 1) & !(page - 1)
}

pub fn compile(prog: &Program) -> Result<CompiledModule> {
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut compiled: Vec<CompiledFn> = Vec::with_capacity(prog.fns.len());
    for f in &prog.fns {
        compiled.push(emit_fn(&mut buf, f));
    }

    // intra-fn jumps
    for cc in &compiled {
        for &(at, target_op) in &cc.jump_patches {
            let target = cc.op_offsets[target_op];
            patch_rel32(&mut buf, at, target);
        }
    }

    // inter-fn call patches (rel32 within the code blob, so we can patch in buf directly)
    for cc in &compiled {
        for &(at, callee) in &cc.call_patches {
            let target = compiled[callee as usize].start;
            patch_rel32(&mut buf, at, target);
        }
    }

    let size = page_round_up(buf.len().max(1));
    // platform flags:
    // - linux: regular anon mapping, starts RW, mprotect to RX after write.
    // - macos: needs MAP_JIT for unsigned processes. request RWX up-front and
    //   toggle between writeable and executable via pthread_jit_write_protect_np.
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

    // SAFETY: standard mmap call; arguments are validated above.
    let mem = unsafe { libc::mmap(std::ptr::null_mut(), size, prot, flags, -1, 0) };
    if mem == libc::MAP_FAILED {
        return Err(Error::Codegen("mmap failed".into()));
    }

    // SAFETY: writing our emitted bytes into the freshly-allocated mapping.
    // on macos we disable jit write-protect around the copy.
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

        // flush the instruction cache so the freshly-written code is visible
        // for execution. on x86-64 the icache is coherent with the dcache
        // so this is a no-op in hardware, but sys_icache_invalidate is
        // still the blessed macOS api.
        #[cfg(target_os = "macos")]
        sys_icache_invalidate(mem, size);
    }

    let base = mem as usize;
    let main_entry = base + compiled[prog.main_id as usize].start;
    Ok(CompiledModule {
        mem,
        size,
        main_entry,
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
