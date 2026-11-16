//! Stable diagnostic codes.

use std::fmt;

/// A stable diagnostic code such as `E0001`.
///
/// Codes are assigned by `seal-error-codes`; this crate only carries them. The
/// format is enforced here so a typo cannot produce an untraceable diagnostic.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiagCode(&'static str);

impl DiagCode {
    /// Creates a code from its static text.
    ///
    /// # Panics
    ///
    /// Panics if `code` is not one upper-case letter (`E`, `W`, or `I`) followed by
    /// four ASCII digits. Codes are compile-time constants; a malformed code is a
    /// programming error, not user input.
    pub fn new(code: &'static str) -> Self {
        Self::try_new(code).unwrap_or_else(|| panic!("invalid diagnostic code `{code}`"))
    }

    /// Creates a code if its format is valid.
    pub fn try_new(code: &'static str) -> Option<Self> {
        let bytes = code.as_bytes();
        if bytes.len() != 5 {
            return None;
        }
        if !matches!(bytes[0], b'E' | b'W' | b'I') {
            return None;
        }
        if !bytes[1..].iter().all(u8::is_ascii_digit) {
            return None;
        }
        Some(Self(code))
    }

    /// Returns the code text.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for DiagCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl fmt::Debug for DiagCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::DiagCode;

    #[test]
    fn accepts_well_formed_codes() {
        assert_eq!(DiagCode::new("E0001").as_str(), "E0001");
        assert_eq!(DiagCode::new("W1234").as_str(), "W1234");
        assert_eq!(DiagCode::new("I9999").as_str(), "I9999");
    }

    #[test]
    fn rejects_malformed_codes() {
        assert!(DiagCode::try_new("X0001").is_none());
        assert!(DiagCode::try_new("E001").is_none());
        assert!(DiagCode::try_new("E00010").is_none());
        assert!(DiagCode::try_new("E00A1").is_none());
        assert!(DiagCode::try_new("").is_none());
        assert!(DiagCode::try_new("e0001").is_none());
    }

    #[test]
    #[should_panic(expected = "invalid diagnostic code")]
    fn new_panics_on_malformed_codes() {
        let _ = DiagCode::new("nope");
    }
}
