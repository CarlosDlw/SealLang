//! Stable query and input identities.

use std::fmt;

use seal_hash::StableHasher;

/// The kind of input a query reads.
///
/// The kind is part of the input identity, so two inputs with the same name but
/// different kinds never alias.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InputKind {
    /// Seal source text.
    Source,
    /// A package manifest.
    Manifest,
    /// Enabled feature names.
    Features,
    /// Target and `cfg` values.
    Cfg,
    /// The selected toolchain or compiler version.
    Toolchain,
    /// A declared environment value, supplied by the caller.
    Environment,
}

impl InputKind {
    /// Returns a stable spelling for diagnostics and cache metadata.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Manifest => "manifest",
            Self::Features => "features",
            Self::Cfg => "cfg",
            Self::Toolchain => "toolchain",
            Self::Environment => "environment",
        }
    }
}

impl fmt::Display for InputKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The stable identifier assigned to one `(kind, name)` input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputId(u32);

impl InputId {
    /// Creates an identifier from its raw index.
    #[inline]
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    /// Returns the raw index.
    #[inline]
    pub const fn as_raw(self) -> u32 {
        self.0
    }

    /// Returns the index as a `usize` for table access.
    #[inline]
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for InputId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "input#{}", self.0)
    }
}

/// The kind of derived query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum QueryKind {
    /// Lexing a source file.
    Tokens,
    /// Parsing a token stream.
    Ast,
    /// Resolving the items and imports visible in a module.
    ModuleItems,
    /// Lowering a declaration to its HIR signature.
    Signature,
    /// Lowering one function or initializer body to HIR.
    BodyHir,
    /// Type-checking one HIR body.
    TypeckBody,
    /// Lowering one checked body to MIR.
    MirBody,
    /// Collecting one monomorphized instance.
    MonoInstance,
    /// Emitting one code generation unit.
    CodegenCgu,
}

impl QueryKind {
    /// Returns a stable spelling for diagnostics and statistics.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tokens => "tokens",
            Self::Ast => "ast",
            Self::ModuleItems => "module-items",
            Self::Signature => "signature",
            Self::BodyHir => "body-hir",
            Self::TypeckBody => "typeck-body",
            Self::MirBody => "mir-body",
            Self::MonoInstance => "mono-instance",
            Self::CodegenCgu => "codegen-cgu",
        }
    }

    /// Returns the stable cache tag for this kind.
    #[inline]
    pub(crate) const fn tag(self) -> u8 {
        match self {
            Self::Tokens => 1,
            Self::Ast => 2,
            Self::ModuleItems => 3,
            Self::Signature => 4,
            Self::BodyHir => 5,
            Self::TypeckBody => 6,
            Self::MirBody => 7,
            Self::MonoInstance => 8,
            Self::CodegenCgu => 9,
        }
    }
}

impl fmt::Display for QueryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The stable logical identity of one derived query.
///
/// The name is the entity identity inside the compiler (for example
/// `main::parse`, `main#42`, or `cgu0`). It must not include a timestamp, a
/// process id, or a host-specific absolute path. Content changes are handled by
/// dependencies; a changed key means a different entity, not an edited one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueryKey {
    kind: QueryKind,
    name: String,
}

impl QueryKey {
    /// Creates a key for `kind` and its stable entity `name`.
    pub fn new(kind: QueryKind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
        }
    }

    /// Returns the query kind.
    #[inline]
    pub const fn kind(&self) -> QueryKind {
        self.kind
    }

    /// Returns the entity name.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the stable digest of this key.
    ///
    /// The digest is used for persistent cache file names. It deliberately does
    /// not include the cache root or any absolute path.
    pub fn digest(&self) -> seal_hash::Fingerprint {
        let mut hasher = StableHasher::new();
        hasher.write_u8(self.kind.tag());
        hasher.write_str(&self.name);
        hasher.finish()
    }
}

impl fmt::Display for QueryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind, self.name)
    }
}
