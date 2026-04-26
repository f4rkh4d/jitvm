# architecture

a tour of the pipeline, and a few notes on why each piece is shaped the way
it is. written in the order a source file travels through.

## the pipeline

```
                 +---------+     +----------+     +----------+
  source  ---->  |  lexer  | --> |  parser  | --> |   ast    |
   .jv           +---------+     +----------+     +----------+
                 line+col toks   pratt +                |
                                 recursive descent      v
                                                  +----------+
                                                  |   ir     |   stack-machine
                                                  +----------+   bytecode,
                                                        |        one Function
                                                        |        per user fn
                                       +----------------+----------------+
                                       |                                 |
                                       v                                 v
                                 +----------+                      +----------+
                                 |  interp  |                      |   jit    |
                                 +----------+                      +----------+
                                 tree of rust                      x86-64 machine
                                 match arms; the                   code, mmap'd,
                                 "oracle" for jit                  called with
                                 correctness                       sysv64 asm
```

each stage is one file in `src/`. nothing lives across the seams except plain
data types: tokens, ast nodes, ir ops. no traits, no dyn, no intermediate
representations that pretend to be extensible.

## lexing

`src/lexer.rs`. hand-written, byte-indexed, produces a flat `Vec<Token>`.
every token carries `(line, col)` of its first byte. there are no string
literals and no floats, so the lexer is mostly: ascii digits form an
`i64`, ascii letters/underscores form an identifier or a keyword, a handful
of single or double char punctuation tokens round it out. comments are
`// to end-of-line`. overflow of int literals is checked at lex time and
turns into a parse error.

why track line and col? parse errors already use them; runtime errors will
eventually (div-by-zero already does, via the span that rides on ir ops;
see below).

## parsing

`src/parser.rs`. two parts glued together:

- **recursive descent** for statements. `fn name(args) { ... }`, `let x = e`,
  `if e { ... } else { ... }`, `while e { ... }`, `return e`, `print e`.
  there's also `x = e` for plain assignment and `e` as an expression-
  statement.
- **pratt / operator-precedence** for expressions. one function `parse_prec`
  takes a minimum binding power and consumes operators with `lbp >= min_bp`.
  recursive calls raise the bound for right-hand sides, so left-associative
  operators fall out naturally. unary `-` and `!` are parsed before the
  pratt loop.

why pratt for binops? because it's the shortest correct way to handle
precedence and associativity in one pass, and it generalizes cleanly if
i ever add more operators. the alternative is a tower of `parse_add`,
`parse_mul`, `parse_cmp` functions, which is fine but gets tedious.

top-level statements (outside any `fn`) are legal; they get folded into
a synthesized `main()`. if the user wrote a `main()` themselves, top-level
statements are an error. this is purely for ergonomics in short examples.

## the ast

`src/ast.rs`. boring on purpose. `Program { fns }`, `Fn { name, params, body }`,
`Stmt`, `Expr`. the only non-trivial bit is that `Expr::Bin` carries a `Span`
of its operator so a later stage can attach source positions to runtime
errors (div-by-zero, etc.) without having to dig through the ast again.

## lowering to ir

`src/ir.rs`. one `Function` per user function. one linear `Vec<Op>` of
ops. jumps are *relative* and measured from the pc *after* the jump
(the standard choice; makes concatenation of basic blocks easier).

the ir is **stack-based**:

```
  Const 3
  Const 4
  Add            # top of stack = 7
```

why stack-based and not three-address? three practical reasons.

1. the jit is a template jit; every op emits a fixed sequence of x86
   instructions. a stack ir makes those templates tiny because the operand
   locations are implicit (always "top of val stack" and "second of val
   stack"). register allocation would make the jit half again as much
   code for not much win at this scale.
2. the interpreter loop is trivial to write and to read. `match op`,
   pop operands, do the thing, push result. no register file to maintain.
3. i wanted the first version to be understandable end to end in an
   afternoon. stack vm is the shortest path there.

the cost is that every operand round-trips through the val stack, which
the jit inherits: every arithmetic op is a pair of pops and a push. a
register-allocating backend would delete most of those; that's what v0.4
is about in the roadmap.

a parallel `spans: Vec<Span>` runs alongside `code`. for ops that can't
fault at runtime, the span is `UNKNOWN` and nobody reads it; for `Div`
and `Mod`, it's the source position of the operator, so the interpreter
can report "division by zero at line 7, col 12" rather than a bare
string.

### control flow

there is no basic-block graph. if/else and while are both compiled down
to `Jump(rel)` and `JumpIfFalse(rel)`, with a small two-pass trick inside
the lowerer:

1. emit the jump with a placeholder offset (`0`),
2. keep emitting until the target is known,
3. patch the offset.

for an `if/else`:

```
    <cond expr>
    JumpIfFalse end_or_else   # to else_start if present, else end
    <then block>
    Jump end                  # only when an else block exists
 else_start:
    <else block>
 end:
```

`while` is symmetric:

```
 loop:
    <cond expr>
    JumpIfFalse end
    <body>
    Jump loop
 end:
```

short-circuit `&&` and `||` get lowered to conditional jumps too, rather
than introducing dedicated ops. see the `BinOp::And` and `BinOp::Or`
branches of `Lowerer::expr`.

## two backends

### interp

`src/interp.rs`. `run` picks the main fn; `call` builds a frame, a local
val stack, and walks the ops. recursion into user fns is rust recursion,
so stack overflow in the source program is stack overflow in the host
binary. for the small programs this targets that's a feature, not a bug:
the backtrace is useful.

the interpreter is the **oracle**. any discrepancy between interp and
jit output is a jit bug by convention. `tests/programs_test.rs` diff-
checks every `tests/programs/*.jv` against its `.expected`, through both
backends, on every commit.

### jit

`src/jit.rs` and `src/x86.rs`. one function in, one code blob out, all
patched up, mmap'd, marked executable, called via a single sysv64
trampoline that hands in the val-stack base pointer. see
`docs/jit-internals.md` for the full walk.

## strings and the heap (v0.2)

the v0.2 cut introduces a second value type: heap-allocated strings. to
avoid doubling the val-stack slot width, values are tagged:

- low bit 0: the upper 63 bits are a signed int (so the int range
  shrank from i64 to i63).
- low bit 1: the upper bits (with the tag masked off) are a pointer
  into a heap arena. all heap allocations are 8-byte aligned so the
  low 3 bits are free.

see `src/value.rs` for the tag helpers and `src/heap.rs` for the
arena + collector.

the interp owns a `Heap` and walks it with a stop-the-world mark-and-
copy collector whenever an allocation would cross a size threshold.
roots are the current val stack + the active frame's slots + any
pointers the calling op still holds in locals. literal strings are
pre-interned on program load and kept alive for the whole run.

the jit keeps a **separate, never-collected** arena of interned
literals and bakes pointers into those literals as imm64 into the
emitted code. the jit can print strings, read `len`, and load literal
pointers, but it can't allocate at runtime in 0.2, so `Op::Concat`
errors at codegen time. programs that concat at runtime go through the
interp; the programs test has an `// interp-only` marker for those.

gc-for-the-jit (with stackmaps and safepoints) is v0.3 work. see
`docs/v0.2-plan.md` for the rationale.

## why i64 only (historical, pre-v0.2)

no strings means no heap. no heap means no gc. no gc means the jit never
has to emit a safepoint, never has to cooperate with anyone, and every
ir op has a 1:1 mapping to a short sequence of x86 instructions. the
moment a `String` enters the language, half the jit becomes allocation
boilerplate and the other half becomes dealing with stale pointers. doing
the i64-only cut first keeps the "codegen loop" honest and exposes the
actual hard parts of jitting (alignment, rip-relative math, register
clobbering across calls) without distraction.

v0.2 in the roadmap is the point where strings show up, and it's the
point where i expect to rewrite a sizable chunk of both backends.
