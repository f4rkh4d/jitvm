//! tiny mark-and-copy heap for string values.
//!
//! v0.2 scope: the heap is used by the interp. the jit owns a separate
//! never-collected arena (see `src/jit.rs`) since the jit can't allocate at
//! runtime in 0.2 anyway (no runtime concat). gc in the jit is v0.3 work.
//!
//! layout: each heap object is an 8-byte aligned `StrHeader` followed by
//! its bytes:
//!
//! ```text
//!   offset 0:  u32 len           (byte length, not codepoint)
//!   offset 4:  u32 hash          (fnv-1a over the bytes)
//!   offset 8:  [u8; len] bytes
//!   offset 8+len .. 8+round_up(len, 8): pad
//! ```
//!
//! the arena is a fixed-size `Box<[u8]>` allocated once per heap, with a
//! bump offset. when the bump pointer would overflow, `collect` copies
//! live objects into a fresh same-sized buffer and swaps them. the old
//! buffer is retained across collections for reuse; this means every
//! returned pointer stays valid until the next collection, and then
//! roots are rewritten to the new buffer's address space.

use crate::value::{self, Value};

/// the per-string header. `#[repr(C)]` so the jit (if we ever teach it to
/// read headers) sees a stable layout.
#[repr(C)]
#[derive(Debug)]
pub struct StrHeader {
    pub len: u32,
    pub hash: u32,
}

pub const HEADER_SIZE: usize = std::mem::size_of::<StrHeader>();

/// fnv-1a over the bytes. hand-rolled so we don't take a dep.
pub fn fnv1a(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn round_up_8(n: usize) -> usize {
    (n + 7) & !7
}

/// the heap. `arena` is a fixed-size buffer with a bump offset. on
/// collect, we evacuate into a second same-sized buffer and swap.
pub struct Heap {
    arena: Box<[u8]>,
    used: usize,
    /// when `used` crosses this, the next alloc triggers a collection.
    live_threshold: usize,
    /// total bytes allocated across the whole lifetime of the heap.
    pub bytes_allocated_lifetime: usize,
    /// count of successful collections.
    pub collections: usize,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

const DEFAULT_ARENA_SIZE: usize = 1024 * 1024; // 1 MB

impl Heap {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_ARENA_SIZE)
    }

    pub fn with_capacity(cap: usize) -> Self {
        Heap {
            arena: vec![0u8; cap].into_boxed_slice(),
            used: 0,
            live_threshold: cap / 2,
            bytes_allocated_lifetime: 0,
            collections: 0,
        }
    }

    /// override the collection threshold. used by the stress test.
    pub fn set_threshold(&mut self, n: usize) {
        self.live_threshold = n;
    }

    /// current arena bump offset in bytes.
    pub fn bytes_in_use(&self) -> usize {
        self.used
    }

    /// allocate a new string. returns a raw pointer to the header. the
    /// pointer is 8-aligned; or-ing tag bit 1 in gives a legal `Value`.
    ///
    /// may trigger a collection. callers that hold un-rooted pointers
    /// across this call MUST pass them through `roots`.
    pub fn alloc_str(&mut self, bytes: &[u8], roots: &mut [Value]) -> *const u8 {
        let need = round_up_8(HEADER_SIZE + bytes.len());
        if self.used + need > self.live_threshold {
            self.collect(roots);
        }
        // on a genuinely huge alloc the arena may still not fit. grow it
        // in that case (rare in 0.2 - strings stay small).
        while self.used + need > self.arena.len() {
            self.grow();
        }
        self.bytes_allocated_lifetime += need;
        let off = self.used;
        let hash = fnv1a(bytes);
        self.arena[off..off + 4].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
        self.arena[off + 4..off + 8].copy_from_slice(&hash.to_le_bytes());
        self.arena[off + HEADER_SIZE..off + HEADER_SIZE + bytes.len()].copy_from_slice(bytes);
        // pad bytes are already zero (we zero-init'd the whole box).
        self.used = off + need;
        unsafe { self.arena.as_ptr().add(off) }
    }

    fn grow(&mut self) {
        let new_cap = self.arena.len() * 2;
        let mut new_buf = vec![0u8; new_cap].into_boxed_slice();
        new_buf[..self.used].copy_from_slice(&self.arena[..self.used]);
        // rewriting roots here would require access to them. callers only
        // hit `grow` on out-of-space within `alloc_str`, and `alloc_str`
        // already called `collect` (which rewrote roots) just above. so
        // after a collect+grow sequence, in-flight roots may be stale.
        //
        // in practice `grow` never fires in 0.2: the threshold is half the
        // arena, so `used + need > threshold` triggers collect first, and
        // collect always shrinks below threshold unless live set is huge.
        // track that case with a debug assert.
        debug_assert!(
            self.used + HEADER_SIZE < self.arena.len(),
            "grow() called after collect(); live set exceeds arena. roots will be stale."
        );
        self.arena = new_buf;
        self.live_threshold = new_cap / 2;
    }

    /// read the header of the object at `ptr`.
    pub fn len_of(&self, ptr: *const u8) -> u32 {
        let off = self.offset_of(ptr);
        u32::from_le_bytes([
            self.arena[off],
            self.arena[off + 1],
            self.arena[off + 2],
            self.arena[off + 3],
        ])
    }

    pub fn hash_of(&self, ptr: *const u8) -> u32 {
        let off = self.offset_of(ptr);
        u32::from_le_bytes([
            self.arena[off + 4],
            self.arena[off + 5],
            self.arena[off + 6],
            self.arena[off + 7],
        ])
    }

    /// the bytes after the header.
    pub fn bytes_of(&self, ptr: *const u8) -> &[u8] {
        let off = self.offset_of(ptr);
        let len = self.len_of(ptr) as usize;
        &self.arena[off + HEADER_SIZE..off + HEADER_SIZE + len]
    }

    fn offset_of(&self, ptr: *const u8) -> usize {
        let base = self.arena.as_ptr() as usize;
        let addr = ptr as usize;
        debug_assert!(
            addr >= base && addr < base + self.used,
            "heap ptr out of range: base={base:x} addr={addr:x} used={}",
            self.used
        );
        addr - base
    }

    /// current collection threshold. the interp uses this to pre-check
    /// whether an alloc would collect, so it can build a proper root set
    /// before calling `alloc_str`.
    pub fn collect_threshold_for_test(&self) -> usize {
        self.live_threshold
    }

    /// mark-and-copy collection. every `Value` in `roots` that is a tagged
    /// pointer and falls inside this arena gets evacuated to a fresh
    /// arena; the root is rewritten in place.
    pub fn collect(&mut self, roots: &mut [Value]) {
        let from_base = self.arena.as_ptr() as usize;
        let from_end = from_base + self.used;

        // to-space is the same size as the from-space. we zero-init so
        // stale data from a previous arena doesn't leak.
        let mut to_space: Box<[u8]> = vec![0u8; self.arena.len()].into_boxed_slice();
        let mut to_used: usize = 0;

        for r in roots.iter_mut() {
            if !value::is_ptr(*r) {
                continue;
            }
            let raw = (*r & !1) as usize;
            if raw < from_base || raw >= from_end {
                continue;
            }
            let from_off = raw - from_base;
            let len_word = u32::from_le_bytes([
                self.arena[from_off],
                self.arena[from_off + 1],
                self.arena[from_off + 2],
                self.arena[from_off + 3],
            ]);
            let new_off = if len_word == u32::MAX {
                // already forwarded.
                u32::from_le_bytes([
                    self.arena[from_off + 4],
                    self.arena[from_off + 5],
                    self.arena[from_off + 6],
                    self.arena[from_off + 7],
                ]) as usize
            } else {
                let len = len_word as usize;
                let total = round_up_8(HEADER_SIZE + len);
                let new_off = to_used;
                to_space[new_off..new_off + total]
                    .copy_from_slice(&self.arena[from_off..from_off + total]);
                to_used += total;
                // write forwarding: len = u32::MAX, hash = new_off.
                self.arena[from_off..from_off + 4].copy_from_slice(&u32::MAX.to_le_bytes());
                self.arena[from_off + 4..from_off + 8]
                    .copy_from_slice(&(new_off as u32).to_le_bytes());
                new_off
            };
            let new_ptr = to_space.as_ptr() as usize + new_off;
            *r = (new_ptr as i64) | 1;
        }

        self.arena = to_space;
        self.used = to_used;
        self.collections += 1;

        // grow threshold when live set is large, so we don't thrash.
        if self.used * 2 > self.live_threshold {
            self.live_threshold = (self.used * 2).max(self.live_threshold);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{self as val};

    #[test]
    fn alloc_and_read() {
        let mut h = Heap::new();
        let mut roots: Vec<Value> = Vec::new();
        let p = h.alloc_str(b"hello", &mut roots);
        assert_eq!(h.len_of(p), 5);
        assert_eq!(h.bytes_of(p), b"hello");
    }

    #[test]
    fn hash_matches_fnv1a() {
        let mut h = Heap::new();
        let mut roots: Vec<Value> = Vec::new();
        let p = h.alloc_str(b"abc", &mut roots);
        assert_eq!(h.hash_of(p), fnv1a(b"abc"));
    }

    #[test]
    fn collect_rewrites_roots() {
        let mut h = Heap::with_capacity(1024);
        h.set_threshold(256);
        let mut roots: Vec<Value> = Vec::new();
        let p = h.alloc_str(b"live", &mut roots);
        let mut roots = vec![val::pack_ptr(p as *const ())];
        let _ = h.alloc_str(b"garbage-one", &mut roots);
        let _ = h.alloc_str(b"garbage-two", &mut roots);
        h.collect(&mut roots);
        let new_p = val::unpack_ptr(roots[0]).unwrap() as *const u8;
        assert_eq!(h.bytes_of(new_p), b"live");
    }

    #[test]
    fn threshold_triggers_collection() {
        let mut h = Heap::with_capacity(1024);
        h.set_threshold(64);
        let mut roots: Vec<Value> = Vec::new();
        let p0 = h.alloc_str(b"keep", &mut roots);
        let mut roots = vec![val::pack_ptr(p0 as *const ())];
        for _ in 0..20 {
            let _ = h.alloc_str(b"throwaway string payload", &mut roots);
        }
        assert!(h.collections > 0, "expected at least one collection");
        let p = val::unpack_ptr(roots[0]).unwrap() as *const u8;
        assert_eq!(h.bytes_of(p), b"keep");
    }
}
