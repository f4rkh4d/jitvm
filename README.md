# jitvm

_a tiny x86-64 jit. one pass, no llvm, ~12x over the bytecode interpreter on the bench loop. for learning, not for production._

[![ci](https://github.com/f4rkh4d/jitvm/actions/workflows/ci.yml/badge.svg)](https://github.com/f4rkh4d/jitvm/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/jitvm.svg)](https://crates.io/crates/jitvm)
[![license](https://img.shields.io/crates/l/jitvm.svg)](https://github.com/f4rkh4d/jitvm/blob/main/LICENSE)

tiny toy language with a bytecode interpreter and an x86-64 jit i wrote from
scratch. integers only, functions, recursion. the jit emits raw machine code
into mmap'd pages and jumps to it.

## what it is

one crate. source -> lexer -> pratt parser -> ast -> bytecode. the bytecode
runs either on a stack interpreter (portable) or on native code emitted by a
template jit (x86-64 linux/macos). v0.2 adds heap strings with a
mark-and-copy gc in the interp, print/len/+ for strings, and the i63 tag
scheme that made it all possible.

## install

from crates.io:

```
cargo install jitvm
```

or from source:

```
git clone https://github.com/f4rkh4d/jitvm
cd jitvm
cargo build --release
```

the binary is `target/release/jitvm`.

## usage

run a file (jit by default on x86-64, interpreter otherwise):

```
$ cargo run --release -- run examples/fib.jv
832040
```

force the interpreter:

```
$ cargo run --release -- run examples/fib.jv --interp
832040
```

bench: runs both vm and jit, reports timings side by side:

```
$ cargo run --release -- bench examples/fib.jv
examples/fib.jv
  vm   832040 in 212ms
  jit  832040 in 19ms   (11.0x)

$ cargo run --release -- bench examples/fib35.jv
examples/fib35.jv
  vm   9227465 in 2.23s
  jit  9227465 in 181ms  (12.3x)
```

numbers above are on a random ubuntu 24.04 x86-64 box, one run each.
rerun it yourself, don't take my word for anything.

disasm:

```
$ cargo run --release -- disasm examples/fib.jv
  fn fib (argc=1, locals=0, ops=19)
    0000  LoadArg(0)
    ...
* fn main (argc=0, locals=0, ops=4)
    ...
```

repl:

```
$ cargo run --release -- repl
jitvm repl. :q to quit, :reset, :disasm
> fn sq(n) { return n * n }
> sq(7)
49
> :q
```

## language

```
// comments start with //
fn greet(who) {
  return "hello, " + who
}

fn fib(n) {
  if n < 2 { return n }
  return fib(n - 1) + fib(n - 2)
}

fn main() {
  print greet("world")
  print fib(30)
}
```

operators: `+ - * / %`, `< <= > >= == !=`, `&& ||`, unary `- !`. precedence
is roughly c-like. `print expr` is a statement, not an expression. no `print`
call form.

top-level statements (outside any fn) get wrapped into a synthesized `main()`,
so `examples/fib.jv` works without writing `fn main` yourself.

## internals

lexer: hand-rolled, flat token vec, tracks line/col. trivial keywords.

parser: recursive descent for statements, pratt for expressions. the whole
thing is one file and small.

ir: stack-based bytecode. one `Function` per user fn. jumps are relative to
the pc after the jump op. args live in slots `0..argc`, lets get appended.

interp: tree of rust match arms over `Op`. calls are rust recursion.

jit: template-style. each bytecode op emits a fixed sequence of x86-64
instructions. the value stack lives in an mmap'd i64 array, base in r15,
top index in r14. locals are indexed via r13 (= r14 at function entry minus
argc). jumps and calls are patched in a second pass after all code is emit.
prints call out to a rust helper via rel32. see `src/jit.rs` and `src/x86.rs`.

## status

works:
- everything above, including mutual recursion and short-circuit bool ops
- fib(30) on the jit is ~11x faster than the interpreter on my box

does not work:
- no floats, no arrays, no dicts
- gc is interp-only for now. the jit handles string literals and `len`
  but errors at codegen time if the program tries to `+` strings at
  runtime. those programs go through `--interp`. (v0.3 lifts this.)
- jit is x86-64 only (linux + macos). on aarch64 the `run --jit` path errors
  and `bench` just runs the vm.
- no tail-call optimization
- no debug info. crashes in jit code are extremely unfun.

## what i learned

i had the whole codegen working end-to-end and then spent the better part of
two evenings on a single issue: `Jz` with a large negative rel32 was sign-
extending wrong because i was computing the offset from the start of the
imm32, not from the end. the rule is rip-relative from the *next*
instruction, which means `target - (patch_pos + 4)`. i had it as
`target - patch_pos` for jz and it worked on small programs and broke the
moment a loop was long enough to want a negative branch.

also: macos blocks `PROT_WRITE | PROT_EXEC` on the same page, so you have to
mmap rw, write the code, then mprotect r+x. linux lets you be lazy. second
thing: on sysv amd64 the stack must be 16-byte aligned *before* a `call`,
not after. i was off by 8 on every even-depth call for a while, which
manifested as a crash only inside `printf` via my `jit_print` helper because
println! uses simd internally. two hours to figure out; annoying.

if i rewrote it: i'd pick a register-based bytecode so the jit templates
are shorter, and i'd stop leaking the value stack through every instruction.
but the template jit pattern is great for a first attempt.

## why one-pass and not two

a two-pass register allocator would buy us another 1.5x or so on a few benchmarks. it would also triple the size of the codegen module and the point of jitvm is "small enough to fit in your head". the current allocator is intentionally bad: it picks the next free callee-saved register, falls back to spilling, and the spill code is dumb.

if you want to see what a serious jit looks like, read the V8 baseline compiler. this is the version where you can read every line in an afternoon.
