//! Bearer-token authentication for remote endpoints.
//!
//! Tokens are validated at construction, sent only in the `authorization`
//! metadata header, and redacted from `Debug`, errors, and worker logs. They are
//! never part of an action, command, or environment variable.

use std::fmt;

use crate::error::{RemoteError, RemoteResult};

const MAX_TOKEN_BYTES: usize = 16 * 1024;

/// A remote bearer token.
#[derive(Clone, PartialEq, Eq)]
pub struct RemoteToken(String);

impl RemoteToken {
    /// Creates a token after rejecting empty values, control characters, and
    /// non-ASCII bytes that cannot be placed in a gRPC metadata value.
    pub fn new(value: impl Into<String>) -> RemoteResult<Self> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(RemoteError::configuration(
                "the remote token is empty; unset it or provide a value",
            ));
        }
        if trimmed.len() > MAX_TOKEN_BYTES {
            return Err(RemoteError::configuration(format!(
                "the remote token exceeds the {MAX_TOKEN_BYTES}-byte limit"
            )));
        }
        if !trimmed.is_ascii()
            || trimmed
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte == b' ')
        {
            return Err(RemoteError::configuration(
                "the remote token must be printable ASCII without whitespace",
            ));
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the token for the authorization header.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Replaces every occurrence of this token in `text` with a marker.
    pub fn redact(&self, text: &str) -> String {
        if self.0.is_empty() {
            return text.to_owned();
        }
        text.replace(&self.0, "<redacted>")
    }

    /// Returns the header value.
    pub fn header_value(&self) -> String {
        format!("Bearer {}", self.0)
    }
}

impl fmt::Debug for RemoteToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RemoteToken(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::RemoteToken;

    #[test]
    fn a_token_is_redacted_from_debug_and_text() {
        let token = RemoteToken::new("secret").expect("valid");
        assert_eq!(format!("{token:?}"), "RemoteToken(<redacted>)");
        assert_eq!(token.redact("failed with secret"), "failed with <redacted>");
    }

    #[test]
    fn empty_and_control_tokens_are_rejected() {
        assert!(RemoteToken::new("").is_err());
        assert!(RemoteToken::new("a\nb").is_err());
        assert!(RemoteToken::new("a b").is_err());
        assert!(RemoteToken::new("não").is_err());
    }
}
