//! tagged 64-bit runtime value. low bit is the tag.
//!
//! - low bit = 0 -> int. actual int = `(raw >> 1)` as i64 (range i63).
//! - low bit = 1 -> heap pointer. actual ptr = `raw & !1`.
//!
//! every heap allocation is 2-byte aligned (in practice 8-byte), so the
//! low bit of a pointer is always 0 before we or-in the tag. that's why we
//! can mask with `& !1` to recover the raw pointer.
//!
//! overflow on the edges of the i63 range is undefined behaviour for 0.2;
//! a proper check fires in 0.3. the parser still rejects literals that
//! won't fit once shifted.

pub type Value = i64;

pub const TAG_INT: i64 = 0;
pub const TAG_PTR: i64 = 1;

/// i63 int range. anything outside this can't be tagged into a `Value`.
pub const INT_MIN: i64 = -(1 << 62);
pub const INT_MAX: i64 = (1 << 62) - 1;

#[inline]
pub fn tag_of(v: Value) -> i64 {
    v & 1
}

#[inline]
pub fn is_int(v: Value) -> bool {
    tag_of(v) == TAG_INT
}

#[inline]
pub fn is_ptr(v: Value) -> bool {
    tag_of(v) == TAG_PTR
}

#[inline]
pub fn pack_int(i: i64) -> Value {
    // we just shift. callers that care about range should check with
    // `fits_int` first.
    i.wrapping_shl(1)
}

#[inline]
pub fn fits_int(i: i64) -> bool {
    (INT_MIN..=INT_MAX).contains(&i)
}

#[inline]
pub fn unpack_int(v: Value) -> Option<i64> {
    if is_int(v) {
        // arithmetic shift right preserves sign.
        Some(v >> 1)
    } else {
        None
    }
}

#[inline]
pub fn pack_ptr(p: *const ()) -> Value {
    (p as i64) | TAG_PTR
}

#[inline]
pub fn unpack_ptr(v: Value) -> Option<*const ()> {
    if is_ptr(v) {
        Some((v & !1) as *const ())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_int() {
        for i in [0_i64, 1, -1, 42, -42, INT_MAX, INT_MIN] {
            let v = pack_int(i);
            assert!(is_int(v));
            assert_eq!(unpack_int(v), Some(i));
        }
    }

    #[test]
    fn tagged_ints_compare_like_raw() {
        // the jit relies on this: cmp on tagged-int values gives the same
        // result as cmp on the underlying i64s.
        let a = pack_int(3);
        let b = pack_int(5);
        assert!(a < b);
        let a = pack_int(-10);
        let b = pack_int(-1);
        assert!(a < b);
    }

    #[test]
    fn roundtrip_ptr() {
        let buf: [u8; 8] = [0; 8];
        let p = buf.as_ptr() as *const ();
        let v = pack_ptr(p);
        assert!(is_ptr(v));
        assert_eq!(unpack_ptr(v), Some(p));
    }

    #[test]
    fn fits_int_boundaries() {
        assert!(fits_int(INT_MIN));
        assert!(fits_int(INT_MAX));
        assert!(!fits_int(INT_MAX + 1));
        assert!(!fits_int(INT_MIN - 1));
    }
}
