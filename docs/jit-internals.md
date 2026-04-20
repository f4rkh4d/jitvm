# jit internals

everything the x86-64 backend assumes about the world, how a `Function`
gets turned into machine code, and the two bugs i got stuck on for long
enough to write them down.

## register layout

the jit reserves three callee-saved registers across the whole blob:

| reg  | role                                                       |
|------|------------------------------------------------------------|
| r15  | base pointer into the val stack (an `mmap`'d `i64[]`)      |
| r14  | top index of the val stack; `[r15 + r14*8]` is the hole    |
| r13  | base index of the current frame's slots (args + locals)    |

the val stack is a contiguous `i64` array, 64k slots, allocated in rust
on entry and handed to the code blob via the invoke asm (`sp` -> `r15`).
`r14` starts at 0 and grows as values are pushed. `r13` is `r14` at the
moment of function entry, minus `argc` so that `[r15 + r13*8]` is the
zeroth argument.

the remaining general-purpose registers are used as scratch: rax, rcx,
rdx for arithmetic; rdi as the first outgoing arg for sysv64 calls (our
`jit_print` helper), and as an alignment-pad push when we need to keep
rsp 16-aligned.

### why three callee-saved registers

because the value stack index and the local-frame base have to survive
every `call`. on sysv64 the registers guaranteed to be preserved across
a call are rbx, rbp, r12-r15. parking our two indices in r14/r15 means
a call into another jitted function or into `jit_print` doesn't have to
save them manually.

## calling convention

the jit uses **sysv64** throughout. the top-level trampoline in
`CompiledModule::run_main` is a thin `asm!` block that:

```
  mov r15, <stack base>
  xor r14d, r14d
  call <main entry>
```

and reads the return value out of rax. no args are passed to `main`. the
asm block lists every scratch register as a clobber so rust knows not
to keep anything live across it.

jit-to-jit calls use a short `Call(fn_id, argc)` sequence:

```
  push r13            # preserve frame base
  push rdi            # 8-byte pad so rsp stays 16-aligned at the call
  call rel32          # patched after all functions are emitted
  pop rdi
  pop r13
```

the `call` instruction itself pushes the return address, which is 8
bytes, so rsp is at `16k + 8` *before* the call. we need `16k` *at* the
call site. two extra pushes of 8 bytes each take us from `16k` to `16k +
16 = 16k`, and then the call turns it back into `16k + 8` for the
callee's prologue to see.

calls into rust (currently only `jit_print`) look similar, but save more
registers, since we don't actually trust rust functions to preserve r13
and r14 through the thread-local + format-args machinery:

```
  push r13
  push r14
  push r15
  push rdi            # pad + we want rdi preserved for our own reasons
  mov rax, &jit_print
  call rax
  pop rdi
  pop r15
  pop r14
  pop r13
```

four 8-byte pushes, same alignment story as above.

## prologue and epilogue

on function entry:

```
  push rbp
  mov  rbp, rsp
  push r13
  push rdi            # not a real arg; it's a scratch-space pad
  mov  r13, r14       # new frame base = current top
  sub  r13, argc      # shift back so r13 points at arg0
  add  r14, locals    # reserve slots for let-locals
```

rsp is `16k + 8` on entry (return address just pushed by `call`). after
`push rbp` it's `16k`; after `push r13` it's `16k + 8` again; after
`push rdi` it's `16k`, which is what sysv requires at the next call site
in the body. three pushes is the minimum that keeps the arithmetic
clean.

on `Ret`:

```
  pop_val_rax         # grab the return value off the val stack
  mov  r14, r13       # discard this frame's slots (args + locals)
  push_val_rax        # publish the return value to the caller
  mov  rsp, rbp       # collapses the three saved pushes in one go
  pop  rbp
  ret
```

the `mov rsp, rbp` trick sidesteps needing to pop r13 and rdi one at a
time. we don't care about their values; the caller preserved whatever
it needed.

## arithmetic

templates are direct. for `Op::Add`:

```
  pop_val_rcx         # dec r14; mov rcx, [r15 + r14*8]
  pop_val_rax         # dec r14; mov rax, [r15 + r14*8]
  add  rax, rcx
  push_val_rax        # mov [r15 + r14*8], rax; inc r14
```

sub, mul (imul), neg, mod (idiv rdx:rax / rcx, then mov rax, rdx) follow
the same pattern. `Div` and `Mod` get a pre-check:

```
  test rcx, rcx
  jne  skip
  <call jit_div_by_zero>   # never returns; prints + exits(1)
skip:
  cqo
  idiv rcx
```

without the check, `idiv` raises SIGFPE when the divisor is 0, which
looks like a segfault from the host's perspective. a clean exit is a
much nicer failure mode. span-to-code mapping for jitted runtime errors
is v0.1.1 work; for now the message doesn't carry a line/col.

### comparisons

`<`, `<=`, `>`, `>=`, `==`, `!=` are all the same shape:

```
  pop_val_rcx
  pop_val_rax
  cmp  rax, rcx
  setcc al
  movzx rax, al
  push_val_rax
```

the condition-code low nibble switches between setl, setle, setg, setge,
sete, setne. `setcc` writes to the low byte of `al`, then `movzx` widens
to the full 64-bit register, giving `0` or `1`.

## jumps and patching

`Jump(rel)` and `JumpIfFalse(rel)` in the ir mean "add `rel` to pc after
reading this op". in x86 the analog is `jmp rel32` and `j<cc> rel32`,
both of which encode the offset relative to the *next* instruction. the
jit uses a two-pass patch:

1. when emitting the jump, write a zero rel32 and remember
   `(byte_position_of_imm32, target_op_index)`.
2. also remember `op_offsets[i]` = byte position where op `i` starts.
3. after the whole function is emitted, walk the patch list and
   overwrite each imm32 with `target_op_offset - (imm32_pos + 4)`.

the `+ 4` is the "rel32 is relative to the end of the imm32, not the
start of the imm32" rule. forgetting it is one of the two bugs below.

inter-function `Call` patches work the same way but over the whole code
blob, using each function's start offset.

## on macOS, MAP_JIT

linux lets you `mmap` with `PROT_READ | PROT_WRITE`, copy your bytes in,
then `mprotect` to `PROT_READ | PROT_EXEC`. on macOS that's blocked by
hardened runtime: pages can't be writable and executable at the same
time, and you can't `mprotect` a regular mapping to `PROT_EXEC`. the
fix is:

1. `mmap` with `MAP_JIT` and `PROT_READ | PROT_WRITE | PROT_EXEC`
   up-front. the kernel tracks this mapping specially.
2. wrap the `memcpy` of emitted bytes in
   `pthread_jit_write_protect_np(false) ... (true)`.
3. call `sys_icache_invalidate` afterwards. on x86-64 the icache is
   coherent with the dcache in hardware, so this is semantically a
   no-op, but Apple documents it as the blessed API and it's cheap.

## things i learned

### the REX.B story

i spent a full evening on this one. the symptom: the first call to
`jit_print` in a program worked, but the second crashed somewhere inside
`println!`. rerun with the same program, same crash. swap to a program
with only one print and everything was fine.

what was actually happening: the SIB-byte encoding for
`[r15 + r14*8]` needs REX.W + REX.X + REX.B = `0x4B`. i had `0x4A`
(missing REX.B for the r15 base), which encodes
`[rdi + r14*8]`. by pure luck, the invoke asm block in rust happens to
put the stack_ptr in some register that rustc then moves to rdi before
my code starts, so my first few `mov [r15 + r14*8], rax` instructions
were writing to the right address. the moment `jit_print` ran and
clobbered rdi with its argument, the next val-stack write landed on
whatever rdi now pointed at, the frame fell apart, and the next
`println!` exploded.

the fix was one bit: `0x4A` -> `0x4B`. debugging took four hours because
the bug was silent until rdi was overwritten, and the overwriter was
twenty instructions away from the actual broken store.

### relative jumps are relative to the end of the imm32

the other classic one. i had `patch_rel32` computing
`target - patch_pos`, which works for small positive offsets inside a
single function (the error is a constant 4 bytes, and if all your
targets are ahead of the current pc you're always off by the same
amount). it breaks the moment you have a negative branch: a `jz` back
to the top of a long `while` loop overshoots by 4 bytes, which happens
to land mid-instruction, and the program either crashes or silently
executes wrong ops.

the rule is in the Intel SDM, plain text: "the offset is added to the
EIP of the instruction immediately following the jump". concretely,
`rel = target - (imm32_pos + 4)`. once i wrote the helper and audited
every call site through it, the whole class of bug disappeared.

## what would be different next time

the template jit is a great first approach because every op produces a
fixed byte sequence and you can debug it with `objdump`. but every
value round-trips through the val stack, which means `x = x + 1` emits
four memory accesses when a register file could have made it one. a
single-pass register allocator over the ir hot path (locals with a
single def and a single use in a basic block) would delete most of the
load/store boilerplate without introducing the book-keeping of a full
linear-scan allocator. that's what v0.4 in the roadmap is for.
