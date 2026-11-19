//! The `bool` locals that guard conditional drop sites.
//!
//! A place can hold a value on one path and not on another, so the drop at the
//! end of its scope cannot be unconditional. Elaboration gives each such place a
//! flag that is set when a value is written and cleared when one is moved or
//! dropped; the drop site then tests the flag instead of guessing.
//!
//! Flags are created before any statement is rewritten, because the maintenance
//! statements that keep them exact have to be inserted from the very first
//! statement of the body. Their order is the order the sites requested them, so
//! a body elaborates the same way every time.

use seal_hash::FxHashMap;
use seal_span::Span;
use seal_ty::{PathElem, TyId};

use crate::body::{Body, LocalDecl, LocalKind};
use crate::id::LocalId;
use crate::init;
use crate::place::Place;

/// The place a flag tracks, named the same way initialization facts name it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FlagKey {
    /// The local the path starts from.
    pub(crate) local: LocalId,
    /// The fields from the local to the tracked place.
    pub(crate) path: Vec<PathElem>,
}

impl FlagKey {
    /// Returns the key of `place`, or `None` when it is not frame storage.
    ///
    /// A place reached through a reference is never tracked: its storage
    /// belongs to the caller and always holds a value.
    pub(crate) fn of(place: &Place) -> Option<Self> {
        let (local, path) = init::path_of(place)?;
        Some(Self { local, path })
    }
}

/// The flags of one body, indexed by the place they track.
#[derive(Default)]
pub(crate) struct Flags {
    by_path: FxHashMap<FlagKey, LocalId>,
}

impl Flags {
    /// Declares a flag for `key` and returns its local.
    ///
    /// The flag is a plain `bool`; code generation and debug information treat
    /// it as compiler storage because of its [`LocalKind`].
    pub(crate) fn declare(
        &mut self,
        body: &mut Body,
        key: FlagKey,
        bool_ty: TyId,
        span: Span,
    ) -> LocalId {
        if let Some(existing) = self.by_path.get(&key) {
            return *existing;
        }
        let local = body.locals.push(LocalDecl {
            ty: bool_ty,
            name: None,
            kind: LocalKind::DropFlag,
            span,
        });
        self.by_path.insert(key, local);
        local
    }

    /// Returns the flag of `key`, when a site needed one.
    pub(crate) fn get(&self, key: &FlagKey) -> Option<LocalId> {
        self.by_path.get(key).copied()
    }

    /// Returns the flags to clear when `key` stops holding a value.
    ///
    /// Moving or dropping a place also moves the fields inside it and takes
    /// away the whole value an enclosing place was tracking, so every flag whose
    /// path and `key`'s path are prefix-comparable is cleared.
    pub(crate) fn cleared(&self, key: &FlagKey) -> Vec<LocalId> {
        let mut flags: Vec<LocalId> = self
            .by_path
            .iter()
            .filter(|(candidate, _)| {
                same_root(candidate, key)
                    && (prefix_of(&candidate.path, &key.path)
                        || prefix_of(&key.path, &candidate.path))
            })
            .map(|(_, flag)| *flag)
            .collect();
        sort_flags(&mut flags);
        flags
    }

    /// Returns the flags to set when `key` receives a value.
    ///
    /// Writing a place initializes every field inside it, so the flags at and
    /// below `key` become true. An enclosing place is deliberately not
    /// promoted: a partial initialization is rejected before elaboration, and
    /// guessing here would let a flag claim a whole value that is not there.
    pub(crate) fn raised(&self, key: &FlagKey) -> Vec<LocalId> {
        let mut flags: Vec<LocalId> = self
            .by_path
            .iter()
            .filter(|(candidate, _)| {
                same_root(candidate, key) && prefix_of(&candidate.path, &key.path)
            })
            .map(|(_, flag)| *flag)
            .collect();
        sort_flags(&mut flags);
        flags
    }

    /// Returns every flag with the place it tracks, in declaration order.
    pub(crate) fn entries(&self) -> Vec<(LocalId, &FlagKey)> {
        let mut entries: Vec<(LocalId, &FlagKey)> = self
            .by_path
            .iter()
            .map(|(key, flag)| (*flag, key))
            .collect();
        entries.sort_by_key(|(flag, _)| flag.index());
        entries
    }
}

fn same_root(left: &FlagKey, right: &FlagKey) -> bool {
    left.local == right.local
}

/// Returns `true` when `prefix` is a prefix of `path`, including equality.
fn prefix_of(path: &[PathElem], prefix: &[PathElem]) -> bool {
    path.len() >= prefix.len() && path.starts_with(prefix)
}

fn sort_flags(flags: &mut Vec<LocalId>) {
    flags.sort_by_key(|flag| flag.index());
    flags.dedup();
}
