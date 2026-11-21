//! Append-only transparency log with signed tree heads and inclusion proofs.
//!
//! The tree follows RFC 6962 hashing: a leaf is `SHA-256(0x00 || leaf)`, an
//! interior node is `SHA-256(0x01 || left || right)`, and an incomplete right
//! child is promoted unchanged. The append-only property is enforced on the
//! server by never rewriting an existing entry; clients verify that an entry is
//! included under a signed root.

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{RegistryError, RegistryResult};
use crate::tuf::{TufSignature, canonical_json};

/// One immutable log entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// The entry schema version.
    pub format: u32,
    /// The zero-based leaf index.
    pub index: u64,
    /// The RFC 3339 publication time.
    pub at: String,
    /// `publish` or `yank`.
    pub kind: String,
    /// The package name.
    pub package: String,
    /// The package version.
    pub version: String,
    /// The archive digest.
    pub checksum: String,
    /// The provenance digest, when published.
    #[serde(default)]
    pub provenance: Option<String>,
    /// The SBOM digest.
    #[serde(default)]
    pub sbom: Option<String>,
    /// The API snapshot digest.
    #[serde(default)]
    pub api: Option<String>,
    /// The publisher key id.
    #[serde(default)]
    pub publisher: Option<String>,
    /// Whether this entry marks the version yanked.
    #[serde(default)]
    pub yanked: bool,
}

/// A signed tree head.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedTreeHead {
    /// The signed payload.
    pub signed: TreeHead,
    /// Detached signatures.
    #[serde(default)]
    pub signatures: Vec<TufSignature>,
}

/// The tree-head payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeHead {
    /// The tree-head schema version.
    pub format: u32,
    /// The number of leaves.
    pub tree_size: u64,
    /// The root hash, `sha256:<hex>`.
    pub root_hash: String,
    /// The RFC 3339 signing time.
    pub timestamp: String,
}

/// The append-only tree held by the server.
#[derive(Clone, Debug, Default)]
pub struct LogTree {
    leaves: Vec<[u8; 32]>,
}

impl LogTree {
    /// Builds a tree from entries in index order.
    pub fn from_entries(entries: &[LogEntry]) -> RegistryResult<Self> {
        let mut leaves = Vec::with_capacity(entries.len());
        for (position, entry) in entries.iter().enumerate() {
            if entry.index != position as u64 {
                return Err(RegistryError::transparency(format!(
                    "the log entry at position {position} claims index {}",
                    entry.index
                )));
            }
            leaves.push(leaf_hash(entry)?);
        }
        Ok(Self { leaves })
    }

    /// Returns the number of leaves.
    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    /// Returns whether the tree is empty.
    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    /// Returns the root hash as `sha256:<hex>`.
    pub fn root(&self) -> String {
        format!("sha256:{}", hex::encode(root_hash(&self.leaves)))
    }

    /// Returns the inclusion proof for one leaf, bottom-up.
    pub fn proof(&self, index: usize) -> RegistryResult<Vec<String>> {
        if index >= self.leaves.len() {
            return Err(RegistryError::transparency(format!(
                "the log has no leaf {index}"
            )));
        }
        let mut proof = Vec::new();
        collect_proof(index, &self.leaves, &mut proof);
        Ok(proof
            .into_iter()
            .map(|hash| format!("sha256:{}", hex::encode(hash)))
            .collect())
    }
}

/// Computes the leaf hash of an entry.
pub fn leaf_hash(entry: &LogEntry) -> RegistryResult<[u8; 32]> {
    let value = serde_json::to_value(entry).map_err(|error| {
        RegistryError::transparency(format!("cannot encode a log entry: {error}"))
    })?;
    let bytes = canonical_json(&value);
    let mut hasher = Sha256::new();
    hasher.update([0u8]);
    hasher.update(bytes);
    Ok(hasher.finalize().into())
}

/// Computes the root hash of the given leaves.
pub fn root_hash(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return Sha256::digest([]).into();
    }
    if leaves.len() == 1 {
        return leaves[0];
    }
    let split = largest_power_of_two_less_than(leaves.len());
    node_hash(&root_hash(&leaves[..split]), &root_hash(&leaves[split..]))
}

/// Signs a tree head with an Ed25519 seed.
pub fn sign_tree_head(
    tree_size: u64,
    root: &str,
    timestamp: &str,
    seed: &[u8; 32],
) -> RegistryResult<SignedTreeHead> {
    let signed = TreeHead {
        format: 1,
        tree_size,
        root_hash: root.to_owned(),
        timestamp: timestamp.to_owned(),
    };
    let value = serde_json::to_value(&signed).map_err(|error| {
        RegistryError::transparency(format!("cannot encode the tree head: {error}"))
    })?;
    let signature = crate::tuf::sign_document(&value, seed)?;
    Ok(SignedTreeHead {
        signed,
        signatures: vec![signature],
    })
}

/// Verifies a signed tree head against a public key.
pub fn verify_tree_head(
    head: &SignedTreeHead,
    public: &str,
    expected_size: Option<u64>,
    expected_root: Option<&str>,
) -> RegistryResult<()> {
    if head.signed.format != 1 {
        return Err(RegistryError::transparency(format!(
            "the tree head uses format {}, but this client understands 1",
            head.signed.format
        )));
    }
    if let Some(size) = expected_size
        && head.signed.tree_size != size
    {
        return Err(RegistryError::transparency(format!(
            "the tree head covers {} leaves, but the index expects {size}",
            head.signed.tree_size
        )));
    }
    if let Some(root) = expected_root
        && head.signed.root_hash != root
    {
        return Err(RegistryError::transparency(format!(
            "the tree head root is `{}`, but the index expects `{root}`",
            head.signed.root_hash
        )));
    }
    if head.signed.tree_size > 1_000_000_000 {
        return Err(RegistryError::transparency(
            "the tree head declares an unreasonable tree size",
        ));
    }
    let public = decode_public_key(public)?;
    let key = VerifyingKey::from_bytes(&public)
        .map_err(|error| RegistryError::transparency(format!("the log key is invalid: {error}")))?;
    let keyid = crate::provenance::key_id(&key);
    let signed = canonical_json(&serde_json::to_value(&head.signed).map_err(|error| {
        RegistryError::transparency(format!("cannot encode the tree head: {error}"))
    })?);
    let mut valid = false;
    for signature in &head.signatures {
        if signature.keyid != keyid {
            continue;
        }
        let Ok(bytes) = hex::decode(&signature.sig) else {
            continue;
        };
        let Ok(bytes) = <[u8; 64]>::try_from(bytes) else {
            continue;
        };
        if key
            .verify_strict(&signed, &Signature::from_bytes(&bytes))
            .is_ok()
        {
            valid = true;
            break;
        }
    }
    if !valid {
        return Err(RegistryError::transparency(
            "the tree head has no valid signature from the trusted log key",
        ));
    }
    Ok(())
}

/// Verifies that a leaf is included under a root.
pub fn verify_inclusion(
    entry: &LogEntry,
    proof: &[String],
    root: &str,
    tree_size: u64,
) -> RegistryResult<()> {
    if tree_size == 0 || entry.index >= tree_size {
        return Err(RegistryError::transparency(
            "the inclusion proof names a leaf outside its tree",
        ));
    }
    let mut hashes = Vec::with_capacity(proof.len());
    for hash in proof {
        let Some(hex) = hash.strip_prefix("sha256:") else {
            return Err(RegistryError::transparency(format!(
                "the proof hash `{hash}` does not use `sha256:`"
            )));
        };
        let bytes = hex::decode(hex).map_err(|error| {
            RegistryError::transparency(format!("the proof hash is invalid: {error}"))
        })?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| RegistryError::transparency("a proof hash is not 32 bytes"))?;
        hashes.push(bytes);
    }
    let leaf = leaf_hash(entry)?;
    let root = root
        .strip_prefix("sha256:")
        .ok_or_else(|| RegistryError::transparency("the root hash does not use `sha256:`"))?;
    let root = hex::decode(root).map_err(|error| {
        RegistryError::transparency(format!("the root hash is invalid: {error}"))
    })?;
    let root: [u8; 32] = root
        .try_into()
        .map_err(|_| RegistryError::transparency("the root hash is not 32 bytes"))?;
    let mut position = 0usize;
    let computed = fold_proof(
        entry.index as usize,
        tree_size as usize,
        leaf,
        &hashes,
        &mut position,
    )
    .ok_or_else(|| RegistryError::transparency("the proof is incomplete"))?;
    if position != hashes.len() {
        return Err(RegistryError::transparency(
            "the proof contains unused hashes",
        ));
    }
    if computed != root {
        return Err(RegistryError::transparency(
            "the inclusion proof does not reconstruct the signed root",
        ));
    }
    Ok(())
}

fn fold_proof(
    index: usize,
    size: usize,
    leaf: [u8; 32],
    proof: &[[u8; 32]],
    position: &mut usize,
) -> Option<[u8; 32]> {
    if size == 1 {
        return Some(leaf);
    }
    let split = largest_power_of_two_less_than(size);
    if index < split {
        let left = fold_proof(index, split, leaf, proof, position)?;
        let sibling = *proof.get(*position)?;
        *position += 1;
        Some(node_hash(&left, &sibling))
    } else {
        let right = fold_proof(index - split, size - split, leaf, proof, position)?;
        let sibling = *proof.get(*position)?;
        *position += 1;
        Some(node_hash(&sibling, &right))
    }
}

fn collect_proof(index: usize, leaves: &[[u8; 32]], proof: &mut Vec<[u8; 32]>) {
    if leaves.len() <= 1 {
        return;
    }
    let split = largest_power_of_two_less_than(leaves.len());
    if index < split {
        collect_proof(index, &leaves[..split], proof);
        proof.push(root_hash(&leaves[split..]));
    } else {
        collect_proof(index - split, &leaves[split..], proof);
        proof.push(root_hash(&leaves[..split]));
    }
}

fn node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([1u8]);
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

fn largest_power_of_two_less_than(value: usize) -> usize {
    let mut power = 1usize;
    while power * 2 < value {
        power *= 2;
    }
    power
}

fn decode_public_key(public: &str) -> RegistryResult<[u8; 32]> {
    if public.len() != 64 || !public.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RegistryError::transparency(
            "the log key is not 64 hexadecimal characters",
        ));
    }
    let bytes = hex::decode(public)
        .map_err(|error| RegistryError::transparency(format!("the log key is invalid: {error}")))?;
    bytes
        .try_into()
        .map_err(|_| RegistryError::transparency("the log key is not 32 bytes"))
}

/// Bundles a tree head and one entry's inclusion proof.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogProof {
    /// The signed tree head.
    pub head: SignedTreeHead,
    /// The entry being proved.
    pub entry: LogEntry,
    /// The inclusion proof.
    pub proof: Vec<String>,
}

/// Verifies a complete [`LogProof`] against a log public key.
pub fn verify_proof(proof: &LogProof, public: &str) -> RegistryResult<()> {
    verify_tree_head(
        &proof.head,
        public,
        Some(proof.head.signed.tree_size),
        Some(&proof.head.signed.root_hash),
    )?;
    verify_inclusion(
        &proof.entry,
        &proof.proof,
        &proof.head.signed.root_hash,
        proof.head.signed.tree_size,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: u64) -> LogEntry {
        LogEntry {
            format: 1,
            index,
            at: "2026-01-01T00:00:00Z".to_owned(),
            kind: "publish".to_owned(),
            package: "demo".to_owned(),
            version: format!("1.0.{index}"),
            checksum: format!("sha256:{index:064x}"),
            provenance: None,
            sbom: None,
            api: None,
            publisher: None,
            yanked: false,
        }
    }

    #[test]
    fn every_leaf_has_a_valid_inclusion_proof() {
        for count in 1..=17u64 {
            let entries: Vec<LogEntry> = (0..count).map(entry).collect();
            let tree = LogTree::from_entries(&entries).unwrap();
            let root = tree.root();
            for index in 0..count {
                let proof = tree.proof(index as usize).unwrap();
                verify_inclusion(&entries[index as usize], &proof, &root, count).unwrap();
            }
        }
    }

    #[test]
    fn an_altered_entry_breaks_the_proof() {
        let entries: Vec<LogEntry> = (0..5).map(entry).collect();
        let tree = LogTree::from_entries(&entries).unwrap();
        let proof = tree.proof(2).unwrap();
        let mut tampered = entries[2].clone();
        tampered.version = "9.9.9".to_owned();
        let error = verify_inclusion(&tampered, &proof, &tree.root(), 5).unwrap_err();
        assert_eq!(error.code(), "E8097");
    }

    #[test]
    fn a_tree_head_signature_is_verified() {
        let seed = [5u8; 32];
        let head = sign_tree_head(
            3,
            &format!("sha256:{}", "a".repeat(64)),
            "2026-01-01T00:00:00Z",
            &seed,
        )
        .unwrap();
        verify_tree_head(&head, &crate::provenance::public_key(&seed), Some(3), None).unwrap();
        let mut tampered = head;
        tampered.signed.tree_size = 4;
        assert!(
            verify_tree_head(&tampered, &crate::provenance::public_key(&seed), None, None).is_err()
        );
    }
}
