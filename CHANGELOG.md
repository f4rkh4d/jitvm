# changelog

## unreleased

- runtime errors now carry source positions. div-by-zero and mod-by-zero
  in the interpreter report "runtime error at line N, col M: ...".
  implemented via a parallel `spans: Vec<Span>` on `ir::Function`; most
  ops stay at `Span::UNKNOWN` since they can't fault at runtime.
- `Error::Runtime` grew an `Option<Span>` tail. constructors are
  `Error::runtime(msg)` and `Error::runtime_at(msg, span)`. use
  `Error::is_runtime()` to assert the variant without pinning text.
- the jit now guards `Div` and `Mod` with a `test rcx, rcx; jne skip;
  call jit_div_by_zero` prelude. on a zero divisor the helper prints
  a clean message to stderr and exits non-zero instead of raising
  SIGFPE. span-to-pc mapping for jitted runtime errors is still TODO.
- docs: `docs/architecture.md`, `docs/jit-internals.md`, and
  `docs/ir-reference.md`. the jit-internals doc has the REX.B and
  rel32 bug writeups.
- examples: `collatz.jv`, `gcd.jv`, `mutrec.jv`, `factorial_loop.jv`,
  `ackermann_small.jv`, each with a `.expected` captured through the
  interpreter.
- `ROADMAP.md` and `CONTRIBUTING.md`.
- `tests/error_tests.rs`: parse + runtime error coverage.
- `Cargo.toml`: metadata (description, license, repository, keywords,
  categories) for eventual crates.io publication.

## 0.1.0 - 2026-04-20

first cut. everything below works, everything not listed doesn't exist.

- a tiny i64-only language: let, if/else, while, return, functions up to 6 args.
- a lexer, a pratt parser, a stack-machine bytecode ir.
- a tree-walking interpreter used as an oracle for tests.
- a one-pass x86-64 jit. hand-rolled encoder, mmap + mprotect to flip pages
  from RW to RX on linux, MAP_JIT + pthread_jit_write_protect_np on macos.
- `jitvm run <file> [--jit]`, `jitvm bench <file>`, `jitvm disasm <file>`.
- 18 example programs with expected output, cross-checked between interp
  and jit via a subprocess-based integration test.
- fib(35) on jit is about 12x faster than the same program through the
  interpreter on my ubuntu 24.04 x86-64 box.

### known limitations (i.e. what 0.2 might try to fix)

- i64 only. no floats, no strings, no heap, no gc.
- linux + macos on x86-64 only. aarch64 falls back to interp.
- one-pass codegen: every value round-trips through the val stack, no
  register allocation, no peephole, no inlining.
- no module system, one file per program.
- error messages are a single string. no line/col in runtime errors.
