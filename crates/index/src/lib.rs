//! Typed indices and dense storage.
//!
//! The compiler uses small integer handles to refer to entities (files, symbols,
//! definitions, types). Wrapping those handles in a distinct index type prevents
//! mixing unrelated handle spaces, which is otherwise an easy source of silent
//! memory corruption bugs.
//!
//! [`IndexVec`] is a `Vec` that can only be indexed by the associated index type.
//! [`newtype_index!`] generates such an index backed by a non-zero `u32`, which
//! keeps `Option<Idx>` the same size as `Idx`.

#![warn(missing_docs)]

use std::fmt;
use std::hash::Hash;
use std::marker::PhantomData;

/// A type-safe index into an [`IndexVec`].
///
/// Implementations must be dense and zero-based: for a container of length `n`,
/// the valid indices are exactly `0..n`.
pub trait Idx: Copy + Eq + Ord + Hash + fmt::Debug + 'static {
    /// Converts a zero-based index into the handle.
    ///
    /// # Panics
    ///
    /// Panics if `index` is not representable by this index type. This is an
    /// internal invariant violation: containers never grow beyond the handle
    /// space, so callers should not observe this panic.
    fn from_usize(index: usize) -> Self;

    /// Returns the zero-based index of the handle.
    fn index(self) -> usize;

    /// Converts a zero-based index into the handle, if representable.
    fn from_usize_checked(index: usize) -> Option<Self>;

    /// Creates a handle from a zero-based index.
    #[inline]
    fn new(index: usize) -> Self {
        <Self as Idx>::from_usize(index)
    }
}

/// A `Vec` indexed by `I` instead of `usize`.
///
/// The raw vector is private so that the index type cannot be bypassed. Use
/// [`IndexVec::into_raw`] to deliberately erase the typing.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct IndexVec<I: Idx, T> {
    raw: Vec<T>,
    // `fn(&I)` keeps the vector `Send`/`Sync` regardless of `I`.
    _marker: PhantomData<fn(&I)>,
}

impl<I: Idx, T> IndexVec<I, T> {
    /// Creates an empty vector.
    #[inline]
    pub fn new() -> Self {
        Self {
            raw: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// Creates an empty vector with the given capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            raw: Vec::with_capacity(capacity),
            _marker: PhantomData,
        }
    }

    /// Wraps a raw vector, treating position `i` as index `i`.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if the vector is longer than the index space.
    #[inline]
    pub fn from_raw(raw: Vec<T>) -> Self {
        debug_assert!(
            I::from_usize_checked(raw.len().saturating_sub(1)).is_some() || raw.is_empty(),
            "vector length exceeds the index space"
        );
        Self {
            raw,
            _marker: PhantomData,
        }
    }

    /// Erases the index type and returns the underlying vector.
    #[inline]
    pub fn into_raw(self) -> Vec<T> {
        self.raw
    }

    /// Returns the number of elements.
    #[inline]
    pub fn len(&self) -> usize {
        self.raw.len()
    }

    /// Returns `true` if the vector contains no elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// Returns the next index that [`IndexVec::push`] will assign.
    #[inline]
    pub fn next_index(&self) -> Option<I> {
        I::from_usize_checked(self.raw.len())
    }

    /// Returns a reference to the element at `index`, or `None` if out of bounds.
    #[inline]
    pub fn get(&self, index: I) -> Option<&T> {
        self.raw.get(index.index())
    }

    /// Returns a mutable reference to the element at `index`, or `None` if out of
    /// bounds.
    #[inline]
    pub fn get_mut(&mut self, index: I) -> Option<&mut T> {
        self.raw.get_mut(index.index())
    }

    /// Returns the last element, if any.
    #[inline]
    pub fn last(&self) -> Option<&T> {
        self.raw.last()
    }

    /// Returns the last element mutably, if any.
    #[inline]
    pub fn last_mut(&mut self) -> Option<&mut T> {
        self.raw.last_mut()
    }

    /// Appends an element and returns its index.
    ///
    /// # Panics
    ///
    /// Panics if the index space is exhausted (more than `u32::MAX - 1`
    /// elements). Exhausting the space is an internal invariant violation.
    #[inline]
    pub fn push(&mut self, value: T) -> I {
        let index = I::from_usize(self.raw.len());
        self.raw.push(value);
        index
    }

    /// Removes and returns the last element, if any.
    #[inline]
    pub fn pop(&mut self) -> Option<T> {
        self.raw.pop()
    }

    /// Shortens the vector, keeping the first `len` elements.
    #[inline]
    pub fn truncate(&mut self, len: usize) {
        self.raw.truncate(len);
    }

    /// Removes all elements.
    #[inline]
    pub fn clear(&mut self) {
        self.raw.clear();
    }

    /// Iterates over the elements.
    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.raw.iter()
    }

    /// Iterates over the elements mutably.
    #[inline]
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.raw.iter_mut()
    }

    /// Iterates over `(index, element)` pairs.
    #[inline]
    pub fn iter_enumerated(&self) -> impl Iterator<Item = (I, &T)> + '_ {
        self.raw
            .iter()
            .enumerate()
            .map(|(index, value)| (I::from_usize(index), value))
    }

    /// Iterates over `(index, element)` pairs mutably.
    #[inline]
    pub fn iter_enumerated_mut(&mut self) -> impl Iterator<Item = (I, &mut T)> + '_ {
        self.raw
            .iter_mut()
            .enumerate()
            .map(|(index, value)| (I::from_usize(index), value))
    }

    /// Iterates over all valid indices in ascending order.
    #[inline]
    pub fn indices(&self) -> impl Iterator<Item = I> + '_ {
        (0..self.raw.len()).map(I::from_usize)
    }

    /// Returns all valid indices as a [`IdxRange`].
    #[inline]
    pub fn index_range(&self) -> IdxRange<I> {
        IdxRange::new(I::from_usize(0), I::from_usize(self.raw.len()))
    }

    /// Returns the underlying slice.
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        &self.raw
    }

    /// Returns the underlying slice mutably.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.raw
    }
}

impl<I: Idx, T> Default for IndexVec<I, T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<I: Idx, T> fmt::Debug for IndexVec<I, T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.raw.iter().enumerate()).finish()
    }
}

impl<I: Idx, T> std::ops::Index<I> for IndexVec<I, T> {
    type Output = T;

    #[inline]
    fn index(&self, index: I) -> &T {
        // Bounds checking is performed here and in `get`; the `Index` trait
        // cannot return `None`, so an out-of-bounds access is a bug.
        &self.raw[index.index()]
    }
}

impl<I: Idx, T> std::ops::IndexMut<I> for IndexVec<I, T> {
    #[inline]
    fn index_mut(&mut self, index: I) -> &mut T {
        &mut self.raw[index.index()]
    }
}

impl<I: Idx, T> FromIterator<T> for IndexVec<I, T> {
    fn from_iter<It: IntoIterator<Item = T>>(iter: It) -> Self {
        Self::from_raw(iter.into_iter().collect())
    }
}

impl<I: Idx, T> Extend<T> for IndexVec<I, T> {
    #[inline]
    fn extend<It: IntoIterator<Item = T>>(&mut self, iter: It) {
        for value in iter {
            self.push(value);
        }
    }
}

impl<'a, I: Idx, T> IntoIterator for &'a IndexVec<I, T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.raw.iter()
    }
}

impl<'a, I: Idx, T> IntoIterator for &'a mut IndexVec<I, T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.raw.iter_mut()
    }
}

impl<I: Idx, T> IntoIterator for IndexVec<I, T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.raw.into_iter()
    }
}

/// A half-open range of typed indices, `start..end`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct IdxRange<I: Idx> {
    start: I,
    end: I,
}

impl<I: Idx> IdxRange<I> {
    /// Creates a range. Both bounds are zero-based indices.
    ///
    /// If `start > end` the range is empty when iterated.
    #[inline]
    pub fn new(start: I, end: I) -> Self {
        Self { start, end }
    }

    /// Returns the first index of the range.
    #[inline]
    pub fn start(self) -> I {
        self.start
    }

    /// Returns the exclusive end of the range.
    #[inline]
    pub fn end(self) -> I {
        self.end
    }

    /// Returns `true` if the range contains no indices.
    #[inline]
    pub fn is_empty(self) -> bool {
        self.start.index() >= self.end.index()
    }

    /// Returns the number of indices in the range.
    #[inline]
    pub fn len(self) -> usize {
        self.end.index().saturating_sub(self.start.index())
    }

    /// Returns `true` if `index` is inside the range.
    #[inline]
    pub fn contains(self, index: I) -> bool {
        self.start.index() <= index.index() && index.index() < self.end.index()
    }

    /// Iterates over the range.
    #[inline]
    pub fn iter(self) -> IdxRangeIter<I> {
        self.into_iter()
    }
}

impl<I: Idx> IntoIterator for IdxRange<I> {
    type Item = I;
    type IntoIter = IdxRangeIter<I>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        IdxRangeIter {
            front: self.start.index(),
            back: self.end.index(),
            _marker: PhantomData,
        }
    }
}

/// Iterator over an [`IdxRange`].
#[derive(Clone, Debug)]
pub struct IdxRangeIter<I: Idx> {
    front: usize,
    back: usize,
    _marker: PhantomData<fn(&I)>,
}

impl<I: Idx> Iterator for IdxRangeIter<I> {
    type Item = I;

    #[inline]
    fn next(&mut self) -> Option<I> {
        if self.front >= self.back {
            return None;
        }
        let index = I::from_usize(self.front);
        self.front += 1;
        Some(index)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.back.saturating_sub(self.front);
        (remaining, Some(remaining))
    }
}

impl<I: Idx> DoubleEndedIterator for IdxRangeIter<I> {
    #[inline]
    fn next_back(&mut self) -> Option<I> {
        if self.front >= self.back {
            return None;
        }
        self.back -= 1;
        Some(I::from_usize(self.back))
    }
}

impl<I: Idx> ExactSizeIterator for IdxRangeIter<I> {}
impl<I: Idx> std::iter::FusedIterator for IdxRangeIter<I> {}

/// Declares a dense, non-zero `u32`-backed index type.
///
/// The generated type is `Copy`, ordered, hashable, and debug-prints its
/// zero-based index (`LocalDefId(3)`). `Option<Idx>` has the same size as `Idx`.
///
/// ```
/// use seal_index::{newtype_index, IndexVec};
///
/// newtype_index! {
///     /// Index of a definition inside a crate.
///     pub struct DefIndex;
/// }
///
/// let mut defs: IndexVec<DefIndex, &str> = IndexVec::new();
/// let first = defs.push("main");
/// assert_eq!(defs[first], "main");
/// assert_eq!(first.index(), 0);
/// assert_eq!(DefIndex::from_u32(0), Some(first));
/// ```
#[macro_export]
macro_rules! newtype_index {
    (
        $(#[$attr:meta])*
        $vis:vis struct $name:ident;
    ) => {
        $(#[$attr])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(transparent)]
        $vis struct $name(::core::num::NonZeroU32);

        impl $name {
            /// Largest valid index value.
            pub const MAX: Self = Self(::core::num::NonZeroU32::MAX);

            /// Creates a handle from a zero-based `u32` index, if representable.
            #[inline]
            pub const fn from_u32(index: u32) -> Option<Self> {
                match index.checked_add(1) {
                    Some(raw) => match ::core::num::NonZeroU32::new(raw) {
                        Some(raw) => Some(Self(raw)),
                        None => None,
                    },
                    None => None,
                }
            }

            /// Creates a handle from a zero-based `usize` index, if representable.
            #[inline]
            pub fn from_usize_checked(index: usize) -> Option<Self> {
                match u32::try_from(index) {
                    Ok(index) => Self::from_u32(index),
                    Err(_) => None,
                }
            }

            /// Returns the zero-based `u32` index.
            #[inline]
            pub const fn index(self) -> u32 {
                self.0.get() - 1
            }

            /// Returns the zero-based index as `usize`.
            #[inline]
            pub const fn as_usize(self) -> usize {
                self.index() as usize
            }
        }

        impl ::core::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(::core::stringify!($name))?;
                f.write_str("(")?;
                ::core::write!(f, "{}", self.index())?;
                f.write_str(")")
            }
        }

        impl $crate::Idx for $name {
            #[inline]
            fn from_usize(index: usize) -> Self {
                match Self::from_usize_checked(index) {
                    Some(value) => value,
                    None => panic!(
                        "index {index} is out of range for {} (max {})",
                        ::core::stringify!($name),
                        Self::MAX.index(),
                    ),
                }
            }

            #[inline]
            fn from_usize_checked(index: usize) -> Option<Self> {
                $name::from_usize_checked(index)
            }

            #[inline]
            fn index(self) -> usize {
                $name::index(self) as usize
            }
        }

        impl ::core::convert::From<$name> for u32 {
            #[inline]
            fn from(value: $name) -> u32 {
                value.index()
            }
        }

        impl ::core::convert::From<$name> for usize {
            #[inline]
            fn from(value: $name) -> usize {
                value.index() as usize
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    newtype_index! {
        /// Test-only entity handle.
        pub struct EntityId;
    }

    #[test]
    fn handle_round_trips() {
        let id = EntityId::from_u32(7).expect("representable");
        assert_eq!(id.index(), 7);
        assert_eq!(id.as_usize(), 7);
        assert_eq!(EntityId::from_usize(7), id);
        assert_eq!(u32::from(id), 7);
        assert_eq!(usize::from(id), 7);
    }

    #[test]
    fn handle_option_is_free() {
        assert_eq!(
            std::mem::size_of::<Option<EntityId>>(),
            std::mem::size_of::<EntityId>()
        );
        assert_eq!(std::mem::size_of::<EntityId>(), 4);
    }

    #[test]
    fn handle_rejects_out_of_range() {
        assert_eq!(EntityId::from_u32(u32::MAX), None);
        assert_eq!(EntityId::MAX.index(), u32::MAX - 1);
        assert_eq!(EntityId::from_usize_checked(usize::MAX), None);
    }

    #[test]
    fn handle_debug_prints_zero_based_index() {
        let id = EntityId::from_u32(3).expect("representable");
        assert_eq!(format!("{id:?}"), "EntityId(3)");
    }

    #[test]
    #[should_panic(expected = "out of range for EntityId")]
    fn handle_panics_when_space_is_exhausted() {
        let _ = EntityId::from_usize(usize::MAX);
    }

    #[test]
    fn index_vec_push_and_index() {
        let mut vec: IndexVec<EntityId, &str> = IndexVec::new();
        let a = vec.push("a");
        let b = vec.push("b");
        assert_eq!(vec[a], "a");
        assert_eq!(vec[b], "b");
        assert_eq!(vec.len(), 2);
        assert_eq!(vec.last(), Some(&"b"));
        assert_eq!(vec.pop(), Some("b"));
        assert_eq!(vec.len(), 1);
    }

    #[test]
    fn index_vec_get_and_iteration() {
        let vec: IndexVec<EntityId, u8> = [10, 20, 30].into_iter().collect();
        assert_eq!(vec.get(EntityId::from_u32(1).unwrap()), Some(&20));
        assert_eq!(vec.get(EntityId::from_u32(9).unwrap()), None);
        let enumerated: Vec<_> = vec
            .iter_enumerated()
            .map(|(index, value)| (index.index(), *value))
            .collect();
        assert_eq!(enumerated, vec![(0, 10), (1, 20), (2, 30)]);
        assert_eq!(
            vec.indices().map(EntityId::index).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn index_range_is_half_open() {
        let vec: IndexVec<EntityId, u8> = [1, 2, 3].into_iter().collect();
        let range = vec.index_range();
        assert_eq!(range.len(), 3);
        assert!(!range.is_empty());
        assert!(range.contains(EntityId::from_u32(2).unwrap()));
        assert!(!range.contains(EntityId::from_u32(3).unwrap()));
        assert_eq!(
            range.iter().map(EntityId::index).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );

        let empty = IdxRange::new(
            EntityId::from_u32(2).unwrap(),
            EntityId::from_u32(1).unwrap(),
        );
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert_eq!(empty.iter().count(), 0);
    }

    #[test]
    fn index_vec_debug_shows_numeric_indices() {
        let vec: IndexVec<EntityId, &str> = ["x"].into_iter().collect();
        assert_eq!(format!("{vec:?}"), "{0: \"x\"}");
    }

    #[test]
    fn index_vec_round_trips_through_raw() {
        let vec: IndexVec<EntityId, u8> = [1, 2].into_iter().collect();
        let raw = vec.clone().into_raw();
        assert_eq!(raw, vec![1, 2]);
        let restored = IndexVec::from_raw(raw);
        assert_eq!(restored, vec);
    }
}
