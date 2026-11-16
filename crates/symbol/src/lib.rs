//! Interned strings.
//!
//! Comparing identifiers by text is common in a compiler and expensive for long
//! names. [`Interner`] stores each distinct string once and returns a
//! [`Symbol`], a `Copy` `u32` handle whose equality is integer equality.
//!
//! # Lifetime and stability
//!
//! Interned strings are intentionally leaked for the lifetime of the process.
//! This is the same trade-off rustc makes: compilers are short-lived processes,
//! and leaking removes every lifetime from the hot path. Consequently a
//! [`Symbol`] is only meaningful inside the process that created it and must
//! never be serialized directly; persist `resolve(symbol)` instead.
//!
//! The predefined symbols are interned first, in the order of
//! [`PredefinedSymbol::ALL`], so their numeric values are stable for a given
//! symbol-table layout. Removing or reordering an entry is an ABI change: append
//! only.

#![warn(missing_docs)]

use std::fmt;
use std::num::NonZeroU32;
use std::sync::{Mutex, MutexGuard, PoisonError};

use seal_hash::FxHashMap;

/// A compact, interned string handle.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Symbol(NonZeroU32);

impl Symbol {
    /// The symbol for the empty string.
    ///
    /// This is always index `0`, because [`PredefinedSymbol::Empty`] is the first
    /// predefined symbol.
    pub const EMPTY: Self = Self(match NonZeroU32::new(1) {
        Some(raw) => raw,
        None => unreachable!(),
    });

    /// Returns the dense zero-based table index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0.get() as usize - 1
    }

    /// Creates a symbol from a dense zero-based table index.
    ///
    /// # Panics
    ///
    /// Panics if `index` is `u32::MAX` or larger. The interner refuses to create
    /// such symbols, so reaching this panic indicates an internal invariant
    /// violation.
    #[inline]
    pub fn from_index(index: usize) -> Self {
        match u32::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
        {
            Some(raw) => match NonZeroU32::new(raw) {
                Some(raw) => Self(raw),
                None => panic!("symbol index {index} exceeds the symbol table capacity"),
            },
            None => panic!("symbol index {index} exceeds the symbol table capacity"),
        }
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Symbol({})", self.index())
    }
}

/// Strings the compiler needs before it has parsed anything.
///
/// The declaration order is part of the symbol-table ABI. Append new entries at
/// the end.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum PredefinedSymbol {
    /// The empty string.
    Empty,
    /// `main`
    Main,
    /// `true`
    True,
    /// `false`
    False,
    /// `bool`
    Bool,
    /// `char`
    Char,
    /// `str`
    Str,
    /// `i8`
    I8,
    /// `i16`
    I16,
    /// `i32`
    I32,
    /// `i64`
    I64,
    /// `i128`
    I128,
    /// `isize`
    Isize,
    /// `u8`
    U8,
    /// `u16`
    U16,
    /// `u32`
    U32,
    /// `u64`
    U64,
    /// `u128`
    U128,
    /// `usize`
    Usize,
    /// `f32`
    F32,
    /// `f64`
    F64,
}

impl PredefinedSymbol {
    /// All predefined symbols, in table order.
    pub const ALL: [Self; 21] = [
        Self::Empty,
        Self::Main,
        Self::True,
        Self::False,
        Self::Bool,
        Self::Char,
        Self::Str,
        Self::I8,
        Self::I16,
        Self::I32,
        Self::I64,
        Self::I128,
        Self::Isize,
        Self::U8,
        Self::U16,
        Self::U32,
        Self::U64,
        Self::U128,
        Self::Usize,
        Self::F32,
        Self::F64,
    ];

    /// Returns the interned text of this symbol.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "",
            Self::Main => "main",
            Self::True => "true",
            Self::False => "false",
            Self::Bool => "bool",
            Self::Char => "char",
            Self::Str => "str",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::I128 => "i128",
            Self::Isize => "isize",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::U128 => "u128",
            Self::Usize => "usize",
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }

    /// Returns the dense table index of this symbol.
    ///
    /// Because the interner interned [`PredefinedSymbol::ALL`] in order, the
    /// index is the position in that array.
    #[inline]
    pub const fn index(self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Main => 1,
            Self::True => 2,
            Self::False => 3,
            Self::Bool => 4,
            Self::Char => 5,
            Self::Str => 6,
            Self::I8 => 7,
            Self::I16 => 8,
            Self::I32 => 9,
            Self::I64 => 10,
            Self::I128 => 11,
            Self::Isize => 12,
            Self::U8 => 13,
            Self::U16 => 14,
            Self::U32 => 15,
            Self::U64 => 16,
            Self::U128 => 17,
            Self::Usize => 18,
            Self::F32 => 19,
            Self::F64 => 20,
        }
    }

    /// Returns the corresponding [`Symbol`].
    #[inline]
    pub fn symbol(self) -> Symbol {
        Symbol::from_index(self.index())
    }
}

struct Inner {
    map: FxHashMap<&'static str, Symbol>,
    strings: Vec<&'static str>,
}

/// A thread-safe string interner.
///
/// Interning takes a lock; resolving copies the leaked `&'static str` out of the
/// table and releases the lock immediately, so resolved strings can be held
/// across arbitrary work.
pub struct Interner {
    inner: Mutex<Inner>,
}

impl fmt::Debug for Interner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Interner")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl Default for Interner {
    fn default() -> Self {
        Self::new()
    }
}

impl Interner {
    /// Creates an interner containing every [`PredefinedSymbol`].
    pub fn new() -> Self {
        let mut inner = Inner {
            map: FxHashMap::default(),
            strings: Vec::with_capacity(PredefinedSymbol::ALL.len()),
        };
        for predefined in PredefinedSymbol::ALL {
            let text = predefined.as_str();
            // The table is empty apart from the symbols added in this loop, so
            // the index is exactly the predefined index.
            debug_assert_eq!(inner.strings.len(), predefined.index());
            let symbol = Symbol::from_index(inner.strings.len());
            inner.strings.push(text);
            inner.map.insert(text, symbol);
        }
        Self {
            inner: Mutex::new(inner),
        }
    }

    /// Interns `text` and returns its symbol, creating it if necessary.
    ///
    /// # Panics
    ///
    /// Panics if the table would need more than `u32::MAX - 1` symbols. That is
    /// an internal capacity invariant, not user input validation.
    pub fn intern(&self, text: &str) -> Symbol {
        let mut inner = self.lock();
        if let Some(&symbol) = inner.map.get(text) {
            return symbol;
        }
        let leaked: &'static str = Box::leak(text.to_owned().into_boxed_str());
        let symbol = Symbol::from_index(inner.strings.len());
        inner.strings.push(leaked);
        inner.map.insert(leaked, symbol);
        symbol
    }

    /// Returns the text of `symbol`.
    ///
    /// # Panics
    ///
    /// Panics if `symbol` did not come from this interner's symbol space. Symbols
    /// from another [`Interner`] are a programming error; the check is exact
    /// because table indices are dense.
    pub fn resolve(&self, symbol: Symbol) -> &'static str {
        let inner = self.lock();
        match inner.strings.get(symbol.index()) {
            Some(text) => text,
            None => panic!(
                "symbol {:?} is not present in this interner (len {})",
                symbol,
                inner.strings.len()
            ),
        }
    }

    /// Returns `Some(symbol)` if `text` is already interned.
    pub fn lookup(&self, text: &str) -> Option<Symbol> {
        self.lock().map.get(text).copied()
    }

    /// Returns the number of interned strings, including predefined symbols.
    pub fn len(&self) -> usize {
        self.lock().strings.len()
    }

    /// Returns `true` if the interner holds no strings. A fresh interner always
    /// holds the predefined symbols, so this is always `false` in practice; it
    /// exists to satisfy the collection convention.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the symbol for a predefined string.
    pub fn predefined(&self, predefined: PredefinedSymbol) -> Symbol {
        predefined.symbol()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock cannot leave the table in an invalid
        // state: every mutation is completed before any panicking operation.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::{Interner, PredefinedSymbol, Symbol};

    #[test]
    fn predefined_symbols_have_stable_indices() {
        let interner = Interner::new();
        for predefined in PredefinedSymbol::ALL {
            let symbol = interner.intern(predefined.as_str());
            assert_eq!(symbol, predefined.symbol());
            assert_eq!(symbol.index(), predefined.index());
            assert_eq!(interner.resolve(symbol), predefined.as_str());
        }
        assert_eq!(PredefinedSymbol::Empty.symbol(), Symbol::EMPTY);
        assert_eq!(interner.resolve(Symbol::EMPTY), "");
    }

    #[test]
    fn interning_is_deduplicated() {
        let interner = Interner::new();
        let first = interner.intern("alpha");
        let second = interner.intern("alpha");
        let third = interner.intern("beta");
        assert_eq!(first, second);
        assert_ne!(first, third);
        assert_eq!(interner.lookup("alpha"), Some(first));
        assert_eq!(interner.lookup("gamma"), None);
    }

    #[test]
    fn distinct_strings_get_distinct_symbols() {
        let interner = Interner::new();
        let symbols: Vec<_> = (0..1000)
            .map(|index| interner.intern(&format!("name{index}")))
            .collect();
        for (index, symbol) in symbols.iter().enumerate() {
            assert_eq!(interner.resolve(*symbol), format!("name{index}"));
        }
        let mut sorted = symbols.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), symbols.len());
    }

    #[test]
    fn resolved_strings_survive_later_interning() {
        let interner = Interner::new();
        let symbol = interner.intern("stable");
        let text = interner.resolve(symbol);
        for index in 0..100 {
            interner.intern(&format!("filler{index}"));
        }
        assert_eq!(text, "stable");
        assert_eq!(interner.resolve(symbol), "stable");
    }

    #[test]
    fn empty_string_is_interned_once() {
        let interner = Interner::new();
        assert_eq!(interner.intern(""), Symbol::EMPTY);
        assert_eq!(interner.resolve(Symbol::EMPTY), "");
    }

    #[test]
    fn symbol_option_has_no_overhead() {
        assert_eq!(
            std::mem::size_of::<Option<Symbol>>(),
            std::mem::size_of::<Symbol>()
        );
        assert_eq!(std::mem::size_of::<Symbol>(), 4);
    }

    #[test]
    fn interning_is_thread_safe_and_canonical() {
        let interner = std::sync::Arc::new(Interner::new());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let interner = std::sync::Arc::clone(&interner);
            handles.push(std::thread::spawn(move || {
                let mut symbols = Vec::new();
                for index in 0..100 {
                    symbols.push(interner.intern(&format!("shared{index}")));
                }
                symbols
            }));
        }
        let results: Vec<Vec<Symbol>> = handles
            .into_iter()
            .map(|handle| handle.join().expect("worker thread must not panic"))
            .collect();
        for window in results.windows(2) {
            assert_eq!(window[0], window[1]);
        }
        assert_eq!(interner.len(), PredefinedSymbol::ALL.len() + 100);
    }

    #[test]
    #[should_panic(expected = "not present in this interner")]
    fn resolving_a_foreign_symbol_panics() {
        let first = Interner::new();
        let second = Interner::new();
        let symbol = first.intern("only-in-first");
        let _ = second.resolve(symbol);
    }
}
