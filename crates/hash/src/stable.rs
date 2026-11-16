//! Stable hashing.
//!
//! [`StableHasher`] produces a [`Fingerprint`] that is independent of the host
//! (endianness, pointer addresses, process randomness) and of the stdlib hash
//! implementation. It is the only hash allowed in anything persisted or compared
//! across runs: incremental-compilation cache keys, crate metadata, and
//! reproducible build reports.
//!
//! # Stability contract
//!
//! The output only changes when [`FORMAT_VERSION`] changes. Any change to the
//! algorithm, to the keys, or to a `StableHash` implementation that is already
//! persisted requires bumping [`FORMAT_VERSION`]. Changing this value invalidates
//! all caches, which is safe; reusing it with a different meaning is not.

use std::fmt;

use crate::sip::SipHasher24;

/// Version of the stable hash format. Bump on any incompatible change.
pub const FORMAT_VERSION: u32 = 1;

// Domain-separated fixed keys. Changing them is a format change.
const KEY_A_0: u64 = 0x7365_616c_5f73_7461;
const KEY_A_1: u64 = 0x626c_655f_6861_7368;
const KEY_B_0: u64 = 0x9e37_79b9_7f4a_7c15;
const KEY_B_1: u64 = 0xf39c_c060_5b8a_3d2e;

/// A 128-bit stable fingerprint.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Fingerprint(u128);

impl Fingerprint {
    /// The all-zero fingerprint, reserved as the "absent" sentinel.
    pub const ZERO: Self = Self(0);

    /// Creates a fingerprint from raw bits.
    #[inline]
    pub const fn from_u128(value: u128) -> Self {
        Self(value)
    }

    /// Returns the raw bits.
    #[inline]
    pub const fn as_u128(self) -> u128 {
        self.0
    }

    /// Returns the low 64 bits.
    #[inline]
    pub const fn low(self) -> u64 {
        self.0 as u64
    }

    /// Returns the high 64 bits.
    #[inline]
    pub const fn high(self) -> u64 {
        (self.0 >> 64) as u64
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({self})")
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl From<u128> for Fingerprint {
    fn from(value: u128) -> Self {
        Self(value)
    }
}

/// Accumulates a stable 128-bit fingerprint.
///
/// Writes are length-delimited for strings, slices, and sequences, so
/// `("ab", "c")` and `("a", "bc")` do not collide.
#[derive(Clone)]
pub struct StableHasher {
    first: SipHasher24,
    second: SipHasher24,
}

impl Default for StableHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl StableHasher {
    /// Creates an empty hasher.
    #[inline]
    pub fn new() -> Self {
        Self {
            first: SipHasher24::new(KEY_A_0, KEY_A_1),
            second: SipHasher24::new(KEY_B_0, KEY_B_1),
        }
    }

    /// Feeds raw bytes. Callers that hash structured data must delimit it.
    #[inline]
    pub fn write(&mut self, bytes: &[u8]) {
        self.first.write(bytes);
        self.second.write(bytes);
    }

    /// Feeds a `u8`.
    #[inline]
    pub fn write_u8(&mut self, value: u8) {
        self.write(&[value]);
    }

    /// Feeds a `u16`.
    #[inline]
    pub fn write_u16(&mut self, value: u16) {
        self.write(&value.to_le_bytes());
    }

    /// Feeds a `u32`.
    #[inline]
    pub fn write_u32(&mut self, value: u32) {
        self.write(&value.to_le_bytes());
    }

    /// Feeds a `u64`.
    #[inline]
    pub fn write_u64(&mut self, value: u64) {
        self.write(&value.to_le_bytes());
    }

    /// Feeds a `u128`.
    #[inline]
    pub fn write_u128(&mut self, value: u128) {
        self.write(&value.to_le_bytes());
    }

    /// Feeds a `usize` as a 64-bit value, so the fingerprint does not depend on
    /// the host pointer width.
    #[inline]
    pub fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    /// Feeds an `isize` as a sign-extended 64-bit value.
    #[inline]
    pub fn write_isize(&mut self, value: isize) {
        self.write_u64(value as i64 as u64);
    }

    /// Feeds a length prefix.
    #[inline]
    pub fn write_length(&mut self, length: usize) {
        self.write_usize(length);
    }

    /// Feeds a string: length prefix followed by UTF-8 bytes.
    #[inline]
    pub fn write_str(&mut self, value: &str) {
        self.write_length(value.len());
        self.write(value.as_bytes());
    }

    /// Feeds a delimited byte slice.
    #[inline]
    pub fn write_bytes(&mut self, value: &[u8]) {
        self.write_length(value.len());
        self.write(value);
    }

    /// Consumes the hasher and returns the fingerprint.
    #[inline]
    pub fn finish(self) -> Fingerprint {
        let low = self.first.finish();
        let high = self.second.finish();
        Fingerprint((u128::from(high) << 64) | u128::from(low))
    }
}

/// Types that can be fed into a [`StableHasher`].
///
/// Implementations must be deterministic and must feed every field that
/// participates in `PartialEq`, otherwise equal values can produce different
/// fingerprints (breaking cache correctness).
pub trait StableHash {
    /// Feeds `self` into `hasher`.
    fn stable_hash(&self, hasher: &mut StableHasher);
}

/// Convenience wrapper: hashes a single value.
pub fn stable_hash_of<T: StableHash + ?Sized>(value: &T) -> Fingerprint {
    let mut hasher = StableHasher::new();
    value.stable_hash(&mut hasher);
    hasher.finish()
}

macro_rules! impl_stable_hash_unsigned {
    ($($ty:ty => $method:ident),* $(,)?) => {
        $(
            impl StableHash for $ty {
                #[inline]
                fn stable_hash(&self, hasher: &mut StableHasher) {
                    hasher.$method(*self);
                }
            }
        )*
    };
}

impl_stable_hash_unsigned! {
    u8 => write_u8,
    u16 => write_u16,
    u32 => write_u32,
    u64 => write_u64,
    u128 => write_u128,
}

impl StableHash for usize {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        hasher.write_usize(*self);
    }
}

impl StableHash for i8 {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        hasher.write_u8(*self as u8);
    }
}

impl StableHash for i16 {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        hasher.write_u16(*self as u16);
    }
}

impl StableHash for i32 {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        hasher.write_u32(*self as u32);
    }
}

impl StableHash for i64 {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        hasher.write_u64(*self as u64);
    }
}

impl StableHash for i128 {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        hasher.write_u128(*self as u128);
    }
}

impl StableHash for isize {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        hasher.write_isize(*self);
    }
}

impl StableHash for bool {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        hasher.write_u8(u8::from(*self));
    }
}

impl StableHash for char {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        hasher.write_u32(*self as u32);
    }
}

impl StableHash for str {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        hasher.write_str(self);
    }
}

impl StableHash for String {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        hasher.write_str(self);
    }
}

impl StableHash for Fingerprint {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        hasher.write_u128(self.0);
    }
}

impl<T: StableHash + ?Sized> StableHash for &T {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        (**self).stable_hash(hasher);
    }
}

impl<T: StableHash + ?Sized> StableHash for Box<T> {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        (**self).stable_hash(hasher);
    }
}

impl<T: StableHash + ?Sized> StableHash for std::sync::Arc<T> {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        (**self).stable_hash(hasher);
    }
}

impl<T: StableHash> StableHash for [T] {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        hasher.write_length(self.len());
        for element in self {
            element.stable_hash(hasher);
        }
    }
}

impl<T: StableHash> StableHash for Vec<T> {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        self.as_slice().stable_hash(hasher);
    }
}

impl<T: StableHash> StableHash for Option<T> {
    #[inline]
    fn stable_hash(&self, hasher: &mut StableHasher) {
        match self {
            None => hasher.write_u8(0),
            Some(value) => {
                hasher.write_u8(1);
                value.stable_hash(hasher);
            }
        }
    }
}

impl StableHash for () {
    #[inline]
    fn stable_hash(&self, _hasher: &mut StableHasher) {}
}

macro_rules! impl_stable_hash_tuple {
    ($($name:ident),+ $(,)?) => {
        impl<$($name: StableHash),+> StableHash for ($($name,)+) {
            #[inline]
            #[allow(non_snake_case)]
            fn stable_hash(&self, hasher: &mut StableHasher) {
                let ($($name,)+) = self;
                $($name.stable_hash(hasher);)+
            }
        }
    };
}

impl_stable_hash_tuple!(A);
impl_stable_hash_tuple!(A, B);
impl_stable_hash_tuple!(A, B, C);
impl_stable_hash_tuple!(A, B, C, D);
impl_stable_hash_tuple!(A, B, C, D, E);
impl_stable_hash_tuple!(A, B, C, D, E, F);

#[cfg(test)]
mod tests {
    use super::{FORMAT_VERSION, Fingerprint, StableHash, StableHasher, stable_hash_of};

    #[test]
    fn format_version_is_pinned() {
        // Changing this line requires an explicit, reviewed decision: it
        // invalidates every persisted cache.
        assert_eq!(FORMAT_VERSION, 1);
    }

    #[test]
    fn fingerprint_is_deterministic_and_non_zero() {
        let first = stable_hash_of(&());
        let second = stable_hash_of(&());
        assert_eq!(first, second);
        assert_ne!(first, Fingerprint::ZERO);
        assert_eq!(format!("{first}").len(), 32);
    }

    #[test]
    fn strings_are_delimited() {
        let mut a = StableHasher::new();
        "ab".stable_hash(&mut a);
        "c".stable_hash(&mut a);

        let mut b = StableHasher::new();
        "a".stable_hash(&mut b);
        "bc".stable_hash(&mut b);

        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn slices_are_delimited() {
        let first = stable_hash_of(&vec![1u8, 2, 3]);
        let second = stable_hash_of(&vec![1u8, 2][..]);
        assert_ne!(first, second);
    }

    #[test]
    fn write_chunking_does_not_change_the_result() {
        let mut whole = StableHasher::new();
        whole.write(b"abcdefghijklmnop");
        let mut split = StableHasher::new();
        for chunk in b"abcdefghijklmnop".chunks(3) {
            split.write(chunk);
        }
        assert_eq!(whole.finish(), split.finish());
    }

    #[test]
    fn option_tags_distinguish_none_from_some() {
        assert_ne!(
            stable_hash_of(&Option::<u8>::None),
            stable_hash_of(&Some(0u8))
        );
        assert_ne!(stable_hash_of(&Some(0u8)), stable_hash_of(&Some(1u8)));
        assert_ne!(stable_hash_of(&false), stable_hash_of(&true));
    }

    #[test]
    fn pointer_sized_values_are_width_independent() {
        let mut narrow = StableHasher::new();
        narrow.write_u64(7);
        let mut wide = StableHasher::new();
        wide.write_usize(7);
        assert_eq!(narrow.finish(), wide.finish());

        let mut negative = StableHasher::new();
        negative.write_isize(-1);
        let mut sign_extended = StableHasher::new();
        sign_extended.write_u64(u64::MAX);
        assert_eq!(negative.finish(), sign_extended.finish());
    }

    #[test]
    fn fingerprint_halves_match() {
        let value = Fingerprint::from_u128(0x0123_4567_89ab_cdef_fedc_ba98_7654_3210);
        assert_eq!(value.low(), 0xfedc_ba98_7654_3210);
        assert_eq!(value.high(), 0x0123_4567_89ab_cdef);
    }
}
