//! Tokens, trivia, and literal metadata.
//!
//! A [`Token`] is a kind plus a span; everything else about the lexeme lives in
//! the kind or in the source text. Identifiers and string literals are interned,
//! so the token stays `Copy` and cheap to move.

use seal_span::Span;
use seal_symbol::Symbol;

/// The base of an integer literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericBase {
    /// `0b1010`
    Binary,
    /// `0o755`
    Octal,
    /// `42`
    Decimal,
    /// `0xFF`
    Hexadecimal,
}

impl NumericBase {
    /// Returns the radix used to parse digits written in this base.
    #[inline]
    pub const fn radix(self) -> u32 {
        match self {
            Self::Binary => 2,
            Self::Octal => 8,
            Self::Decimal => 10,
            Self::Hexadecimal => 16,
        }
    }
}

/// A numeric type suffix written on a literal.
///
/// The lexer validates the spelling; whether the value fits the suffix is
/// checked later, when the literal's type is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericSuffix {
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

impl NumericSuffix {
    /// Returns the source spelling of the suffix.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
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

    /// Parses a suffix by its source spelling.
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "i8" => Self::I8,
            "i16" => Self::I16,
            "i32" => Self::I32,
            "i64" => Self::I64,
            "i128" => Self::I128,
            "isize" => Self::Isize,
            "u8" => Self::U8,
            "u16" => Self::U16,
            "u32" => Self::U32,
            "u64" => Self::U64,
            "u128" => Self::U128,
            "usize" => Self::Usize,
            "f32" => Self::F32,
            "f64" => Self::F64,
            _ => return None,
        })
    }

    /// Returns `true` for `f32` and `f64`.
    #[inline]
    pub const fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }

    /// Returns `true` for the integer suffixes.
    #[inline]
    pub const fn is_integer(self) -> bool {
        !self.is_float()
    }
}

/// The lexical form of a token.
///
/// Keywords are separate kinds rather than identifiers with a flag, so the
/// parser cannot accidentally treat `func` as a name. Everything that needs the
/// source text (operators, punctuation, keywords) is recoverable from the span.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TokenKind {
    /// End of the file. Always the last token, with an empty span.
    Eof,

    /// An identifier, interned.
    Ident(Symbol),

    /// An integer literal and its optional suffix.
    Int {
        /// Value of the digits, ignoring separators.
        value: u128,
        /// Base written in the source.
        base: NumericBase,
        /// Type suffix, if any.
        suffix: Option<NumericSuffix>,
    },

    /// A floating-point literal and its optional suffix.
    Float {
        /// Correctly rounded value of the literal.
        value: f64,
        /// Type suffix, if any.
        suffix: Option<NumericSuffix>,
    },

    /// A string literal; the symbol holds the unescaped contents.
    Str(Symbol),

    /// A character literal.
    Char(char),

    /// `func`
    Func,
    /// `let`
    Let,
    /// `mut`
    Mut,
    /// `return`
    Return,
    /// `true`
    True,
    /// `false`
    False,

    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `[`
    LBracket,
    /// `]`
    RBracket,

    /// `,`
    Comma,
    /// `;`
    Semicolon,
    /// `:`
    Colon,
    /// `.`
    Dot,
    /// `->`
    Arrow,

    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `%`
    Percent,

    /// `++`
    PlusPlus,
    /// `--`
    MinusMinus,

    /// `=`
    Eq,
    /// `==`
    EqEq,
    /// `!=`
    NotEq,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    LtEq,
    /// `>=`
    GtEq,

    /// `&&`
    AndAnd,
    /// `||`
    OrOr,
    /// `!`
    Bang,

    /// `&`
    Amp,
    /// `|`
    Pipe,
    /// `^`
    Caret,
    /// `~`
    Tilde,
    /// `<<`
    Shl,
    /// `>>`
    Shr,

    /// `+=`
    PlusEq,
    /// `-=`
    MinusEq,
    /// `*=`
    StarEq,
    /// `/=`
    SlashEq,
    /// `%=`
    PercentEq,
    /// `&=`
    AmpEq,
    /// `|=`
    PipeEq,
    /// `^=`
    CaretEq,
    /// `<<=`
    ShlEq,
    /// `>>=`
    ShrEq,

    /// A token that produced a diagnostic.
    ///
    /// The span covers the malformed input. The parser treats this kind as a
    /// recovery point; because the driver stops after errors, it never reaches
    /// semantic analysis.
    Error,
}

/// A significant token.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Token {
    /// What the token is.
    pub kind: TokenKind,
    /// The exact source range it covers.
    pub span: Span,
    /// Whether a line break appeared between the previous token and this one,
    /// even if only inside a block comment. Useful for layout-aware parsing and
    /// diagnostics.
    pub preceded_by_newline: bool,
}

/// Comments are preserved as trivia so formatters, documentation tools, and
/// language servers do not need to re-lex the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriviaKind {
    /// `// ...`
    LineComment,
    /// `/* ... */`, possibly nested.
    BlockComment,
    /// `/// ...`
    OuterLineDoc,
    /// `//! ...`
    InnerLineDoc,
    /// `/** ... */`
    OuterBlockDoc,
    /// `/*! ... */`
    InnerBlockDoc,
}

impl TriviaKind {
    /// Returns `true` for the four documentation-comment kinds.
    #[inline]
    pub const fn is_doc(self) -> bool {
        matches!(
            self,
            Self::OuterLineDoc | Self::InnerLineDoc | Self::OuterBlockDoc | Self::InnerBlockDoc
        )
    }
}

/// A comment, kept out of the significant token stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trivia {
    /// What kind of comment it is.
    pub kind: TriviaKind,
    /// The exact source range, delimiters included.
    pub span: Span,
}

/// The result of lexing one file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lexed {
    /// Significant tokens, ending with [`TokenKind::Eof`].
    pub tokens: Vec<Token>,
    /// Comments in source order.
    pub trivia: Vec<Trivia>,
}
