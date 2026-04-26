# changelog

## unreleased (v0.2)

added:

- heap-allocated immutable strings. `"..."` literals with `\n`, `\t`,
  `\\`, `\"` escapes. `+` on strings concatenates (interp); `len(s)`
  builtin; `print` dispatches between int and string.
- string pool: source-level duplicates collapse to a single entry; the
  engine pre-allocates each entry once on program load.
- new `src/value.rs`: tagged 64-bit values. low bit 0 = int (i63), low
  bit 1 = heap pointer (8-aligned so the tag bit is free). the int
  range shrank from i64 to i63 as a result.
- new `src/heap.rs`: bump-allocated arena with a stop-the-world
  mark-and-copy collector. threshold-triggered, roots are the current
  val stack + slots, forwarding-pointer scheme in-place.
- jit tagged-form arithmetic. `(i<<1) + (j<<1) = (i+j)<<1` already
  tagged, so `add/sub/neg` stay one instruction; `imul` needs a
  follow-up `sar rax, 1`; `idiv` does `sar, sar, idiv, shl`; comparisons
  get a trailing `shl rax, 1` to tag the boolean.
- jit string support (literals only): `Op::Str(id)` bakes an imm64
  pointer into a separate never-collected jit-owned arena, `Op::StrLen`
  reads the header, `Op::Print` branches on tag and dispatches to
  `jit_print_int` or `jit_print_str`.
- tests: new `tests/heap_test.rs` (5 unit tests). new `str_hello`,
  `str_concat`, `str_len`, `str_empty_concat`, `str_multi` programs in
  `tests/programs/`. new `tests/programs_interp/` with `str_loop` and
  `str_gc_stress` for programs the jit can't run in 0.2.
- `examples/hello.jv`.

known limitations:

- the jit can't `+` strings at runtime; `Op::Concat` is a codegen-time
  error. use `--interp` for programs that concat at runtime. (fixed in
  v0.3, along with jit gc.)
- gc is interp-only. the jit leaks its literal arena for the process
  lifetime. safe in 0.2 because the jit has no other allocation sites.
- int literals outside the i63 range are a parse error. previously the
  language supported the full i64 range; `wrap.jv` was rewritten.
- string equality is content-based (fnv-1a fast reject then memcmp).
  interning is not forced on runtime-constructed strings.

prior (unreleased) changes, carried over from the v0.1 -> v0.2 window:

## unreleased (carried over)

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
