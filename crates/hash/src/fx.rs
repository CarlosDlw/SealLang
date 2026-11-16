//! Fast, non-cryptographic hashing for in-memory compiler tables.
//!
//! These maps are never persisted, so the hash function is free to change. Use
//! [`crate::StableHasher`] for anything that must be reproducible across
//! compiler versions.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

/// Multiplier from the Fx hash used by rustc (a polynomial hash with a single
/// final rotation avoided by construction).
const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// A fast, deterministic-in-a-run hasher for compiler-internal maps.
#[derive(Clone, Copy, Debug, Default)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    /// Creates a hasher with the standard seed.
    #[inline]
    pub const fn new() -> Self {
        Self { hash: 0 }
    }

    #[inline]
    fn add_to_hash(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, mut bytes: &[u8]) {
        while bytes.len() >= 8 {
            let word = u64::from_le_bytes(bytes[..8].try_into().expect("8-byte word"));
            self.add_to_hash(word);
            bytes = &bytes[8..];
        }
        if !bytes.is_empty() {
            let mut word = 0u64;
            for (offset, &byte) in bytes.iter().enumerate() {
                word |= u64::from(byte) << (8 * offset);
            }
            self.add_to_hash(word);
        }
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add_to_hash(u64::from(value));
    }

    #[inline]
    fn write_u16(&mut self, value: u16) {
        self.add_to_hash(u64::from(value));
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add_to_hash(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add_to_hash(value);
    }

    #[inline]
    fn write_u128(&mut self, value: u128) {
        self.add_to_hash(value as u64);
        self.add_to_hash((value >> 64) as u64);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add_to_hash(value as u64);
    }

    #[inline]
    fn write_i8(&mut self, value: i8) {
        self.add_to_hash(value as u8 as u64);
    }

    #[inline]
    fn write_i16(&mut self, value: i16) {
        self.add_to_hash(value as u16 as u64);
    }

    #[inline]
    fn write_i32(&mut self, value: i32) {
        self.add_to_hash(value as u32 as u64);
    }

    #[inline]
    fn write_i64(&mut self, value: i64) {
        self.add_to_hash(value as u64);
    }

    #[inline]
    fn write_i128(&mut self, value: i128) {
        self.write_u128(value as u128);
    }

    #[inline]
    fn write_isize(&mut self, value: isize) {
        self.add_to_hash(value as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

/// [`std::collections::HashMap`] using [`FxHasher`].
pub type FxHashMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

/// [`std::collections::HashSet`] using [`FxHasher`].
pub type FxHashSet<T> = HashSet<T, BuildHasherDefault<FxHasher>>;

/// [`std::collections::HashMap`] that preserves insertion order on iteration.
///
/// Implemented as a thin wrapper over [`FxHashMap`] plus a key list, because the
/// compiler relies on deterministic iteration for reproducible output.
#[derive(Clone, Debug)]
pub struct FxIndexMap<K, V> {
    entries: FxHashMap<K, V>,
    order: Vec<K>,
}

impl<K, V> Default for FxIndexMap<K, V> {
    fn default() -> Self {
        Self {
            entries: FxHashMap::default(),
            order: Vec::new(),
        }
    }
}

impl<K, V> FxIndexMap<K, V>
where
    K: std::hash::Hash + Eq + Clone,
{
    /// Creates an empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a value, keeping the first insertion position for existing keys.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self.entries.get_mut(&key) {
            Some(slot) => Some(std::mem::replace(slot, value)),
            None => {
                self.order.push(key.clone());
                self.entries.insert(key, value);
                None
            }
        }
    }

    /// Returns a reference to the value for `key`.
    pub fn get(&self, key: &K) -> Option<&V> {
        self.entries.get(key)
    }

    /// Returns the number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the map is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterates over entries in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> + '_ {
        self.order
            .iter()
            .filter_map(|key| self.entries.get(key).map(|value| (key, value)))
    }
}

#[cfg(test)]
mod tests {
    use super::{FxHashMap, FxHasher, FxIndexMap};
    use std::hash::Hasher;

    #[test]
    fn hasher_is_deterministic_and_order_sensitive() {
        let mut a = FxHasher::new();
        a.write(b"seal");
        let mut b = FxHasher::new();
        b.write(b"seal");
        assert_eq!(a.finish(), b.finish());

        let mut c = FxHasher::new();
        c.write(b"laes");
        assert_ne!(a.finish(), c.finish());
    }

    #[test]
    fn hasher_is_not_streaming_across_word_boundaries() {
        // Fx mixes one word per call; chunking therefore matters. `Hash` always
        // feeds a given type with the same chunking, so this is acceptable for
        // in-memory tables, but it is another reason not to persist Fx output.
        let mut whole = FxHasher::new();
        whole.write(b"abcdefgh");
        let mut split = FxHasher::new();
        split.write(b"abcd");
        split.write(b"efgh");
        assert_ne!(whole.finish(), split.finish());
    }

    #[test]
    fn map_works() {
        let mut map: FxHashMap<&str, u32> = FxHashMap::default();
        map.insert("a", 1);
        assert_eq!(map.get("a"), Some(&1));
    }

    #[test]
    fn index_map_preserves_insertion_order() {
        let mut map: FxIndexMap<&str, u32> = FxIndexMap::new();
        map.insert("second", 2);
        map.insert("first", 1);
        map.insert("second", 22);
        let keys: Vec<_> = map.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys, vec!["second", "first"]);
        assert_eq!(map.get(&"second"), Some(&22));
        assert_eq!(map.len(), 2);
    }
}
