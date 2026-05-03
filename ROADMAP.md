# roadmap

this is a wishlist, not a promise. the project works today at 0.1.0 and
i'm leaving it usable first, shippable second. everything below is
aspirational and will arrive in whatever order i find time and an
interesting-enough problem to chew on.

## v0.2: strings and a gc (shipped, mostly)

strings, tagged values, interp gc. the jit got literals + `len` +
tag-dispatched `print` but punted on runtime concat and on jitting the
gc itself (no stackmaps yet). see CHANGELOG and `docs/v0.2-plan.md` for
the reality-check.

## v0.3: jit gc + tag-dispatch arithmetic + aarch64 backend

three things the 0.2 cut deferred, merged into one release because
they all touch the jit's cooperation story.

- **jit gc.** safepoints at every call site (already there, just
  unused) + a stackmap table mapping return-pc to live-slot counts +
  calling the collector via a helper that preserves callee-saves. once
  this works, `Op::Concat` stops being a codegen error.
- **tag-dispatch `+` in the jit.** branch on the low bit; ints go
  through the existing fast path, pointers dispatch to the concat
  helper. this is mostly plumbing once jit gc is in.
- **aarch64 backend.** my actual dev box is an apple silicon mac.
  two shapes this could take: a separate `arm64.rs` that duplicates
  the work, or an `Encoder` trait abstracting over the two encoders
  with per-backend glue for calling conventions and cache flushing.
  the trait route is more work but keeps the ir-to-asm translation
  honest about what's shared (not much) between the two isas.

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

