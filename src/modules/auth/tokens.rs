//! Opaque random tokens (refresh, email verification, password reset). Only the SHA-256 hash is
//! stored, so a database leak does not expose usable tokens.

use sha2::{Digest, Sha256};

/// 256 bits of randomness, hex encoded.
pub fn generate() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_and_long() {
        let (a, b) = (generate(), generate());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    #[test]
    fn hash_is_deterministic_and_not_the_token() {
        let t = generate();
        assert_eq!(hash(&t), hash(&t));
        assert_eq!(hash(&t).len(), 32);
        assert_ne!(hash(&t), t.as_bytes());
    }
}
