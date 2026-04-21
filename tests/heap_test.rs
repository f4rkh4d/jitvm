//! unit tests for the heap + copying gc.

use jitvm::heap::{fnv1a, Heap};
use jitvm::value::{self, Value};

#[test]
fn alloc_roundtrips() {
    let mut h = Heap::new();
    let mut roots: Vec<Value> = Vec::new();
    let p = h.alloc_str(b"abcdef", &mut roots);
    assert_eq!(h.len_of(p), 6);
    assert_eq!(h.bytes_of(p), b"abcdef");
    assert_eq!(h.hash_of(p), fnv1a(b"abcdef"));
}

#[test]
fn empty_string() {
    let mut h = Heap::new();
    let mut roots: Vec<Value> = Vec::new();
    let p = h.alloc_str(b"", &mut roots);
    assert_eq!(h.len_of(p), 0);
    assert_eq!(h.bytes_of(p), b"");
}

#[test]
fn collect_rewrites_ptrs() {
    let mut h = Heap::new();
    let mut roots: Vec<Value> = Vec::new();
    let p = h.alloc_str(b"keep-me", &mut roots);
    let mut roots = vec![value::pack_ptr(p as *const ())];
    // stir in some garbage so to-space addresses differ.
    let _ = h.alloc_str(b"ignored-one", &mut roots);
    let _ = h.alloc_str(b"ignored-two", &mut roots);
    let before = h.bytes_in_use();
    h.collect(&mut roots);
    let after = h.bytes_in_use();
    assert!(after < before, "collect should shrink arena");
    let new_p = value::unpack_ptr(roots[0]).unwrap() as *const u8;
    assert_eq!(h.bytes_of(new_p), b"keep-me");
}

#[test]
fn threshold_triggers_collection() {
    let mut h = Heap::new();
    h.set_threshold(64);
    let mut dummy_roots: Vec<Value> = Vec::new();
    let p0 = h.alloc_str(b"root", &mut dummy_roots);
    let mut roots = vec![value::pack_ptr(p0 as *const ())];
    for _ in 0..32 {
        let _ = h.alloc_str(b"some throwaway bytes", &mut roots);
    }
    assert!(
        h.collections > 0,
        "expected collection to fire, got 0 collections"
    );
    let p = value::unpack_ptr(roots[0]).unwrap() as *const u8;
    assert_eq!(h.bytes_of(p), b"root");
}

#[test]
fn multiple_roots_preserved_across_collection() {
    let mut h = Heap::new();
    h.set_threshold(64);
    let mut r: Vec<Value> = Vec::new();
    let a = h.alloc_str(b"alpha", &mut r);
    let b = h.alloc_str(b"beta", &mut r);
    let c = h.alloc_str(b"gamma", &mut r);
    let mut roots = vec![
        value::pack_ptr(a as *const ()),
        value::pack_ptr(b as *const ()),
        value::pack_ptr(c as *const ()),
    ];
    // allocate a bunch more while all three are rooted.
    for _ in 0..50 {
        let _ = h.alloc_str(b"fill-up-the-arena-please", &mut roots);
    }
    let a2 = value::unpack_ptr(roots[0]).unwrap() as *const u8;
    let b2 = value::unpack_ptr(roots[1]).unwrap() as *const u8;
    let c2 = value::unpack_ptr(roots[2]).unwrap() as *const u8;
    assert_eq!(h.bytes_of(a2), b"alpha");
    assert_eq!(h.bytes_of(b2), b"beta");
    assert_eq!(h.bytes_of(c2), b"gamma");
}
