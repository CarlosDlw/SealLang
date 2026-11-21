//! Content digests.

use std::fmt;
use std::io::Read;
use std::path::Path;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::CasError;

/// A blake3 digest of one content-addressed object.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    /// Hashes an in-memory byte slice.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Hashes a stream.
    pub fn of_reader(mut reader: impl Read) -> Result<Self, CasError> {
        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = reader.read(&mut buffer).map_err(|error| {
                CasError::io("<stream>", format!("cannot read content: {error}"))
            })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(Self(*hasher.finalize().as_bytes()))
    }

    /// Hashes a file.
    pub fn of_file(path: &Path) -> Result<Self, CasError> {
        let file = std::fs::File::open(path).map_err(|error| {
            CasError::io(path.display().to_string(), format!("cannot open: {error}"))
        })?;
        Self::of_reader(file)
    }

    /// Returns the raw bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns the lowercase hexadecimal form.
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    /// Parses exactly 64 hexadecimal characters.
    pub fn parse(value: &str) -> Result<Self, CasError> {
        if value.len() != 64 {
            return Err(CasError::invalid_digest(
                value,
                "a digest has exactly 64 hexadecimal characters",
            ));
        }
        let bytes = hex::decode(value).map_err(|error| {
            CasError::invalid_digest(value, format!("invalid hexadecimal: {error}"))
        })?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| CasError::invalid_digest(value, "a digest has exactly 32 bytes"))?;
        Ok(Self(bytes))
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Digest({})", self.to_hex())
    }
}

impl Serialize for Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::Digest;

    #[test]
    fn digests_round_trip_through_hex() {
        let digest = Digest::of_bytes(b"seal");
        assert_eq!(Digest::parse(&digest.to_hex()).expect("valid"), digest);
        assert_eq!(digest.to_string(), digest.to_hex());
    }

    #[test]
    fn malformed_digests_are_rejected_with_a_typed_error() {
        let error = Digest::parse("abc").expect_err("too short");
        assert_eq!(error.code(), "E8040");
        assert!(Digest::parse(&"z".repeat(64)).is_err());
    }
}
