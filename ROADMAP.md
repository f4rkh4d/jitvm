# roadmap

this is a wishlist, not a promise. the project works today at 0.1.0 and
i'm leaving it usable first, shippable second. everything below is
aspirational and will arrive in whatever order i find time and an
interesting-enough problem to chew on.

## v0.2: strings and a gc

the big one. add a `String` type (immutable, heap-allocated) and a
trivial mark-and-sweep gc over a `Gc<T>` handle. the hard part isn't
the allocator, it's teaching the jit to cooperate: every `Call` site
becomes a safepoint where the val stack needs a precise map of which
slots are `Gc<T>` and which are `i64`. i expect this to reshape the ir
(probably typed slots, probably a second parallel `Vec<Ty>` on
`Function`) and force a real look at calling convention. concatenation,
equality, indexing by i64, and a `len` builtin would be the minimum
surface area to make it feel like a language.

## v0.3: aarch64 backend

currently the jit is x86-64 linux/macos only; on aarch64 the
`run --jit` path errors out and `bench` silently runs only the
interpreter. i'd like to fix that so my actual development machine
(an apple silicon mac) can exercise the jit too. two shapes this could
take: a separate `arm64.rs` that duplicates the work, or an `Encoder`
trait abstracting over the two encoders with per-backend glue for
calling conventions and cache flushing. the trait route is more work
but keeps the ir-to-asm translation honest about what's actually shared
between the two isas (not much, honestly, beyond the basic-block layout
and the patch tables).

## v0.4: register allocation

this is where the jit stops being embarrassed by gcc's `-O0`. a single-
pass allocator over the ir for the hot case where a let-local has a
single def and a small number of uses in the same basic block could
eliminate most of the load/store pairs that the template jit emits. i'd
start with a greedy scheme that keeps the top-of-val-stack in `rax`
across op boundaries and only spills when a `Call` or `Print` needs the
register. a full linear-scan allocator with lifetime intervals is a
much bigger lift and probably wants the ir to grow a proper basic-block
graph first.

## v0.5 and beyond: legibility

this is the bucket where the remaining papercuts go. span-everything so
every runtime error points at real source code, including inside the
jit (needs a small side-table that maps pc ranges in generated code
back to ir spans). a real module system, so programs can span files. a
repl worth calling a repl: multi-line input, line-editing, persistent
functions across inputs, maybe colorized disasm. none of these are
research-grade, but the project starts to feel polished when they're
done.

## non-goals

i'm explicitly not doing:

- floats. they're a whole separate calling convention and allocator
  problem, and i don't have a use case for them in this toy.
- mutable strings, arrays, maps. if i add arrays for real it's going to
  be tuples first, then arrays-as-tuples-with-runtime-length.
- a self-hosted compiler. jitvm is written in rust and is going to stay
  written in rust.
