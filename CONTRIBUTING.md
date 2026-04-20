# contributing

this is a solo project but happy to take patches.

## before you send a pr

run the usual three. all must pass clean.

```
cargo fmt
cargo clippy --release --all-targets -- -D warnings
cargo test --release
```

ci runs the same on linux-x86_64 (with the jit path) and macos-arm64
(interp only). the jit path is cfg'd out on non-x86-64 builds, which
is why local testing on apple silicon only exercises the interpreter.

## where tests live

- `tests/lang_tests.rs`: hand-written integration tests over the
  interpreter. one function per language feature. add to this when you
  touch parsing, lowering, or the interp.
- `tests/programs_test.rs`: every `tests/programs/*.jv` gets run through
  both backends (interp in-process, jit as a subprocess) and the output
  diff-checked against its `.expected` file. add a new program here
  when you want end-to-end coverage.
- `tests/error_tests.rs`: parse/runtime error paths. covers the
  specific messages and error variants users rely on.
- unit tests live inline in each module (`mod tests` at the bottom
  of `src/lexer.rs`, `src/parser.rs`, `src/x86.rs`).

## style notes

no em-dashes, no emoji, lowercase commit messages. the codebase tries
to be honest about what it is and why: comments explain *why*, not
*what*. if you ship a bugfix for something non-obvious, write a one-
paragraph note in `docs/jit-internals.md` or wherever it fits. future
readers (including me, in three months) will thank you.

## bigger changes

if you're planning a big refactor, a new backend, or anything that
touches the ir layout, please open an issue first. the project has a
small surface area and i'd rather talk about direction before a diff
lands.
