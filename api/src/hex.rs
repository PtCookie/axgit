//! Lowercase hex encoding for the sha256 digests axgit turns into strings:
//! `ETag`s (`handlers/mod.rs::body_etag`, `assets.rs::serve_embedded_file`)
//! and the avatar seed (`repo/commits.rs::email_hash`). `sha2` 0.11 returns a
//! plain `Array<u8, _>` with no `LowerHex` impl — 0.10's `{:x}` `Digest`
//! formatting is gone — so all three encode through here instead.

use std::fmt::Write;

/// Encodes `bytes` as lowercase hex, two characters per byte.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(out, "{byte:02x}").expect("writing to a String never fails");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_should_match_a_known_digest() {
        assert_eq!(encode(&[0u8; 32]), "0".repeat(64));
        assert_eq!(encode(&[0xab; 32]), "ab".repeat(32));
    }
}
