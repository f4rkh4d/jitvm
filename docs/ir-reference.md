# ir reference

a short card. every op in the stack-machine bytecode, what it expects on
the val stack (top on the right), and what it leaves. `...` is "whatever
was already on the stack below". `a`, `b` are `i64` operands.

## pushes and pops

| op                 | before       | after         | notes                                              |
|--------------------|--------------|---------------|----------------------------------------------------|
| `Const(v)`         | `...`        | `... v`       | push an i64 literal                                |
| `Pop`              | `... a`      | `...`         | discard top                                        |
| `LoadLocal(s)`     | `...`        | `... slots[s]`| push a let-local by slot index                     |
| `StoreLocal(s)`    | `... a`      | `...`         | pop into let-local slot                            |
| `LoadArg(a)`       | `...`        | `... args[a]` | push a fn argument by index (args are slots 0..argc)|

`LoadArg(i)` and `LoadLocal(i)` are indistinguishable at the value-stack
level; the distinction exists so a future backend could keep args in
registers while spilling lets.

## arithmetic (all i64, wrapping)

| op      | before     | after   | notes                                               |
|---------|------------|---------|-----------------------------------------------------|
| `Add`   | `... a b`  | `... r` | `r = a + b` (wrapping)                              |
| `Sub`   | `... a b`  | `... r` | `r = a - b` (wrapping)                              |
| `Mul`   | `... a b`  | `... r` | `r = a * b` (wrapping)                              |
| `Div`   | `... a b`  | `... r` | `r = a / b`, traps on `b == 0`                      |
| `Mod`   | `... a b`  | `... r` | `r = a % b`, traps on `b == 0`, sign follows `a`    |
| `Neg`   | `... a`    | `... r` | `r = -a` (wrapping)                                 |
| `Not`   | `... a`    | `... r` | `r = 1` if `a == 0` else `r = 0`                    |

## comparisons

all comparisons are signed. result is `1` (true) or `0` (false).

| op      | before     | after   | notes         |
|---------|------------|---------|---------------|
| `Lt`    | `... a b`  | `... r` | `a < b`       |
| `Le`    | `... a b`  | `... r` | `a <= b`      |
| `Gt`    | `... a b`  | `... r` | `a > b`       |
| `Ge`    | `... a b`  | `... r` | `a >= b`      |
| `Eq`    | `... a b`  | `... r` | `a == b`      |
| `Ne`    | `... a b`  | `... r` | `a != b`      |

## control flow

offsets are **relative to the pc after the op is read**, i.e. `pc' = pc +
offset` where `pc` is already past the jump.

| op                  | before   | after   | notes                                         |
|---------------------|----------|---------|-----------------------------------------------|
| `Jump(off)`         | `...`    | `...`   | unconditional                                 |
| `JumpIfFalse(off)`  | `... a`  | `...`   | pop; branch if zero                           |
| `Call(id, argc)`    | `... a1..an` | `... r` | pop `argc` values, invoke fn `id`, push return|
| `Ret`               | `... r`  | (returns) | pop return value, hand it to caller        |

`Call(id, argc)` assumes the callee's declared argc matches; the
interpreter checks this explicitly, the jit trusts the lowerer.

## i/o

| op      | before     | after   | notes                                   |
|---------|------------|---------|-----------------------------------------|
| `Print` | `... a`    | `...`   | pop, write decimal + newline to stdout  |

there is no general print-formatting story. strings don't exist yet.

## meta

`Function` layout:

- `argc`: `u8`, number of function arguments.
- `locals`: `u16`, number of `let` bindings declared inside the body
  (excludes args).
- `code: Vec<Op>`, the bytecode.
- `spans: Vec<Span>`, parallel to `code`. for ops that can't fault, the
  span is `Span::UNKNOWN`. for `Div` and `Mod`, it's the source position
  of the operator, so runtime-error messages can say "at line N, col M".

the slot array indexed by `LoadLocal`/`StoreLocal`/`LoadArg` is
`argc + locals` i64s long and layered `[arg0, arg1, ..., argN, let0, let1, ...]`.
