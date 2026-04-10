//! hand-rolled x86-64 encoder. just enough for jitvm.
//!
//! every free function here appends raw bytes to a `Vec<u8>`. we never
//! validate anything; each function emits exactly the shape documented
//! in its comment. byte sequences were cross-checked against the Intel
//! SDM.
//!
//! note: we always emit the REX.W form for 64-bit operations. condition
//! codes (CC_*) match the low nibble of the Jcc / SETcc opcodes.

#![allow(dead_code)]

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reg {
    Rax = 0,
    Rcx = 1,
    Rdx = 2,
    Rbx = 3,
    Rsp = 4,
    Rbp = 5,
    Rsi = 6,
    Rdi = 7,
    R8 = 8,
    R9 = 9,
    R10 = 10,
    R11 = 11,
    R12 = 12,
    R13 = 13,
    R14 = 14,
    R15 = 15,
}

impl Reg {
    #[inline]
    fn low3(self) -> u8 {
        (self as u8) & 0b111
    }
    #[inline]
    fn is_high(self) -> bool {
        (self as u8) >= 8
    }
}

fn rex_w(r_ext: bool, b_ext: bool) -> u8 {
    0x48 | (if r_ext { 0b0100 } else { 0 }) | (if b_ext { 0b0001 } else { 0 })
}

fn modrm(mod_: u8, reg: u8, rm: u8) -> u8 {
    ((mod_ & 0b11) << 6) | ((reg & 0b111) << 3) | (rm & 0b111)
}

/// mov reg, imm64      REX.W + B8+rd + imm64
pub fn mov_reg_imm64(buf: &mut Vec<u8>, dst: Reg, imm: i64) {
    buf.push(rex_w(false, dst.is_high()));
    buf.push(0xB8 + dst.low3());
    buf.extend_from_slice(&imm.to_le_bytes());
}

/// mov dst, src (both r64)    REX.W + 0x89 /r
pub fn mov_reg_reg(buf: &mut Vec<u8>, dst: Reg, src: Reg) {
    buf.push(rex_w(src.is_high(), dst.is_high()));
    buf.push(0x89);
    buf.push(modrm(0b11, src.low3(), dst.low3()));
}

/// push r64
pub fn push_reg(buf: &mut Vec<u8>, r: Reg) {
    if r.is_high() {
        buf.push(0x41);
    }
    buf.push(0x50 + r.low3());
}

/// pop r64
pub fn pop_reg(buf: &mut Vec<u8>, r: Reg) {
    if r.is_high() {
        buf.push(0x41);
    }
    buf.push(0x58 + r.low3());
}

/// ret
pub fn ret(buf: &mut Vec<u8>) {
    buf.push(0xC3);
}

/// add dst, src        REX.W + 0x01 /r
pub fn add_reg_reg(buf: &mut Vec<u8>, dst: Reg, src: Reg) {
    buf.push(rex_w(src.is_high(), dst.is_high()));
    buf.push(0x01);
    buf.push(modrm(0b11, src.low3(), dst.low3()));
}

/// sub dst, src        REX.W + 0x29 /r
pub fn sub_reg_reg(buf: &mut Vec<u8>, dst: Reg, src: Reg) {
    buf.push(rex_w(src.is_high(), dst.is_high()));
    buf.push(0x29);
    buf.push(modrm(0b11, src.low3(), dst.low3()));
}

/// imul dst, src (both r64)   REX.W + 0x0F 0xAF /r
pub fn imul_reg_reg(buf: &mut Vec<u8>, dst: Reg, src: Reg) {
    buf.push(rex_w(dst.is_high(), src.is_high()));
    buf.push(0x0F);
    buf.push(0xAF);
    buf.push(modrm(0b11, dst.low3(), src.low3()));
}

/// neg r64     REX.W + 0xF7 /3
pub fn neg_reg(buf: &mut Vec<u8>, r: Reg) {
    buf.push(rex_w(false, r.is_high()));
    buf.push(0xF7);
    buf.push(modrm(0b11, 3, r.low3()));
}

/// cqo   48 99   (rax -> rdx:rax sign-extend; used before idiv)
pub fn cqo(buf: &mut Vec<u8>) {
    buf.push(0x48);
    buf.push(0x99);
}

/// idiv r64    REX.W + 0xF7 /7  (rdx:rax / reg)
pub fn idiv_reg(buf: &mut Vec<u8>, r: Reg) {
    buf.push(rex_w(false, r.is_high()));
    buf.push(0xF7);
    buf.push(modrm(0b11, 7, r.low3()));
}

/// cmp lhs, rhs   REX.W + 0x39 /r   (flags = lhs - rhs)
pub fn cmp_reg_reg(buf: &mut Vec<u8>, lhs: Reg, rhs: Reg) {
    buf.push(rex_w(rhs.is_high(), lhs.is_high()));
    buf.push(0x39);
    buf.push(modrm(0b11, rhs.low3(), lhs.low3()));
}

/// test lhs, rhs   REX.W + 0x85 /r  (flags = lhs & rhs)
pub fn test_reg_reg(buf: &mut Vec<u8>, lhs: Reg, rhs: Reg) {
    buf.push(rex_w(rhs.is_high(), lhs.is_high()));
    buf.push(0x85);
    buf.push(modrm(0b11, rhs.low3(), lhs.low3()));
}

/// xor dst, src    REX.W + 0x31 /r
pub fn xor_reg_reg(buf: &mut Vec<u8>, dst: Reg, src: Reg) {
    buf.push(rex_w(src.is_high(), dst.is_high()));
    buf.push(0x31);
    buf.push(modrm(0b11, src.low3(), dst.low3()));
}

/// setCC al    0F 9x /0   (reg field ignored; use 0).
pub fn setcc_al(buf: &mut Vec<u8>, cc: u8) {
    buf.push(0x0F);
    buf.push(0x90 + cc);
    buf.push(modrm(0b11, 0, 0)); // al
}

/// movzx rax, al    48 0F B6 C0
pub fn movzx_rax_al(buf: &mut Vec<u8>) {
    buf.push(0x48);
    buf.push(0x0F);
    buf.push(0xB6);
    buf.push(0xC0);
}

// condition-code low nibbles.
pub const CC_E: u8 = 0x4;
pub const CC_NE: u8 = 0x5;
pub const CC_L: u8 = 0xC;
pub const CC_LE: u8 = 0xE;
pub const CC_G: u8 = 0xF;
pub const CC_GE: u8 = 0xD;

/// jmp rel32. writes `rel`, returns position of the imm32 for later patching.
pub fn jmp_rel32(buf: &mut Vec<u8>, rel: i32) -> usize {
    buf.push(0xE9);
    let pos = buf.len();
    buf.extend_from_slice(&rel.to_le_bytes());
    pos
}

/// jcc rel32 (long-form conditional jump). returns position of imm32.
pub fn jcc_rel32(buf: &mut Vec<u8>, cc: u8, rel: i32) -> usize {
    buf.push(0x0F);
    buf.push(0x80 + cc);
    let pos = buf.len();
    buf.extend_from_slice(&rel.to_le_bytes());
    pos
}

/// call rel32. returns position of imm32.
pub fn call_rel32(buf: &mut Vec<u8>, rel: i32) -> usize {
    buf.push(0xE8);
    let pos = buf.len();
    buf.extend_from_slice(&rel.to_le_bytes());
    pos
}

/// call r64    FF /2
pub fn call_reg(buf: &mut Vec<u8>, r: Reg) {
    if r.is_high() {
        buf.push(0x41);
    }
    buf.push(0xFF);
    buf.push(modrm(0b11, 2, r.low3()));
}

/// patch a rel32 imm at `pos` so that it points to `target` (both offsets in buf).
pub fn patch_rel32(buf: &mut [u8], pos: usize, target: usize) {
    let rel = target as i32 - (pos as i32 + 4);
    buf[pos..pos + 4].copy_from_slice(&rel.to_le_bytes());
}

/// mov reg, [rbp + disp]   REX.W + 0x8B /r  (disp8 or disp32)
pub fn mov_reg_rbp_disp(buf: &mut Vec<u8>, dst: Reg, disp: i32) {
    buf.push(rex_w(dst.is_high(), false));
    buf.push(0x8B);
    if (-128..=127).contains(&disp) {
        buf.push(modrm(0b01, dst.low3(), 5));
        buf.push(disp as i8 as u8);
    } else {
        buf.push(modrm(0b10, dst.low3(), 5));
        buf.extend_from_slice(&disp.to_le_bytes());
    }
}

/// mov [rbp + disp], src   REX.W + 0x89 /r
pub fn mov_rbp_disp_reg(buf: &mut Vec<u8>, disp: i32, src: Reg) {
    buf.push(rex_w(src.is_high(), false));
    buf.push(0x89);
    if (-128..=127).contains(&disp) {
        buf.push(modrm(0b01, src.low3(), 5));
        buf.push(disp as i8 as u8);
    } else {
        buf.push(modrm(0b10, src.low3(), 5));
        buf.extend_from_slice(&disp.to_le_bytes());
    }
}

/// sub rsp, imm32    48 81 EC imm32
pub fn sub_rsp_imm32(buf: &mut Vec<u8>, imm: i32) {
    buf.push(0x48);
    buf.push(0x81);
    buf.push(modrm(0b11, 5, 4));
    buf.extend_from_slice(&imm.to_le_bytes());
}

/// add rsp, imm32    48 81 C4 imm32
pub fn add_rsp_imm32(buf: &mut Vec<u8>, imm: i32) {
    buf.push(0x48);
    buf.push(0x81);
    buf.push(modrm(0b11, 0, 4));
    buf.extend_from_slice(&imm.to_le_bytes());
}

/// push rbp; mov rbp, rsp; sub rsp, frame
pub fn emit_prologue(buf: &mut Vec<u8>, frame: i32) {
    push_reg(buf, Reg::Rbp);
    mov_reg_reg(buf, Reg::Rbp, Reg::Rsp);
    if frame > 0 {
        sub_rsp_imm32(buf, frame);
    }
}

/// mov rsp, rbp; pop rbp; ret
pub fn emit_epilogue(buf: &mut Vec<u8>) {
    mov_reg_reg(buf, Reg::Rsp, Reg::Rbp);
    pop_reg(buf, Reg::Rbp);
    ret(buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mov_rax_5() {
        let mut b = Vec::new();
        mov_reg_imm64(&mut b, Reg::Rax, 5);
        assert_eq!(b, vec![0x48, 0xB8, 5, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn mov_r8_1() {
        let mut b = Vec::new();
        mov_reg_imm64(&mut b, Reg::R8, 1);
        assert_eq!(b, vec![0x49, 0xB8, 1, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn add_rax_rcx() {
        let mut b = Vec::new();
        add_reg_reg(&mut b, Reg::Rax, Reg::Rcx);
        assert_eq!(b, vec![0x48, 0x01, 0xC8]);
    }

    #[test]
    fn push_pop_rax() {
        let mut b = Vec::new();
        push_reg(&mut b, Reg::Rax);
        pop_reg(&mut b, Reg::Rax);
        assert_eq!(b, vec![0x50, 0x58]);
    }

    #[test]
    fn push_r13_bytes() {
        let mut b = Vec::new();
        push_reg(&mut b, Reg::R13);
        assert_eq!(b, vec![0x41, 0x55]);
    }

    #[test]
    fn ret_byte() {
        let mut b = Vec::new();
        ret(&mut b);
        assert_eq!(b, vec![0xC3]);
    }

    #[test]
    fn cmp_rax_rcx() {
        let mut b = Vec::new();
        cmp_reg_reg(&mut b, Reg::Rax, Reg::Rcx);
        assert_eq!(b, vec![0x48, 0x39, 0xC8]);
    }

    #[test]
    fn prologue() {
        let mut b = Vec::new();
        emit_prologue(&mut b, 0);
        assert_eq!(b, vec![0x55, 0x48, 0x89, 0xE5]);
    }

    #[test]
    fn idiv_rcx_bytes() {
        let mut b = Vec::new();
        idiv_reg(&mut b, Reg::Rcx);
        assert_eq!(b, vec![0x48, 0xF7, 0xF9]);
    }
}
