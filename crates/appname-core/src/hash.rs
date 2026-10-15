// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §0.3 (hash policy), §2.5 (`ContentHash`), §3.8 (content
//! addressing) — the BLAKE3-256 content identity.
//!
//! [NORMATIVE, ES §0.3] Hashes are BLAKE3 (256-bit) for content addressing;
//! xxHash64 is permitted for in-memory cache keys only and is never persisted
//! as an identity. [NORMATIVE, ES §3.8] Content hashes are computed over the
//! uncompressed payload bytes, and BLAKE3-256 equality **is** identity — no
//! byte-compare on hash hit.

use std::fmt;

/// 256-bit BLAKE3 content identity (ES §2.5). Derived state: recomputed on
/// load and edit, never persisted in `document.json`. When a hash does reach
/// a serialized artifact (ES §0.3), it is a 64-character lowercase hex string
/// — see [`ContentHash::to_hex`].
#[derive(Clone, Copy)]
pub struct ContentHash(pub blake3::Hash);

impl ContentHash {
    /// Hash one contiguous byte payload.
    pub fn of_bytes(bytes: &[u8]) -> ContentHash {
        ContentHash(blake3::hash(bytes))
    }

    /// Streaming construction for payloads produced in chunks (uniform-tile
    /// hashing streams a repeated pattern without allocating, ES §2.3/§3.8).
    pub fn from_stream(feed: impl FnOnce(&mut blake3::Hasher)) -> ContentHash {
        let mut hasher = blake3::Hasher::new();
        feed(&mut hasher);
        ContentHash(hasher.finalize())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }

    /// 64-character lowercase hex form (ES §0.3 serialization rule).
    pub fn to_hex(&self) -> String {
        self.0.to_hex().to_string()
    }
}

impl PartialEq for ContentHash {
    fn eq(&self, o: &ContentHash) -> bool {
        // blake3::Hash equality is constant-time over the 32 bytes.
        self.0 == o.0
    }
}

impl Eq for ContentHash {}

impl std::hash::Hash for ContentHash {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write(self.0.as_bytes());
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({})", self.to_hex())
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn equality_is_byte_identity() {
        let a = ContentHash::of_bytes(b"appname");
        let b = ContentHash::of_bytes(b"appname");
        let c = ContentHash::of_bytes(b"appnamf");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn streaming_matches_contiguous() {
        let contiguous = ContentHash::of_bytes(b"hello tile payload");
        let streamed = ContentHash::from_stream(|h| {
            h.update(b"hello ");
            h.update(b"tile ");
            h.update(b"payload");
        });
        assert_eq!(contiguous, streamed);
    }

    #[test]
    fn hex_is_64_lowercase_chars() {
        let h = ContentHash::of_bytes(&[0u8; 4]).to_hex();
        assert_eq!(h.len(), 64);
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn usable_as_hashmap_key() {
        let mut m = HashMap::new();
        m.insert(ContentHash::of_bytes(b"k"), 7u32);
        assert_eq!(m.get(&ContentHash::of_bytes(b"k")), Some(&7));
    }
}
