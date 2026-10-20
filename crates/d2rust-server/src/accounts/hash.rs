//! Password hashing: argon2id, stored as a PHC string (contracts section
//! 4). The PHC string carries its own parameters, so a hash made at one
//! cost still verifies after the server's cost changes.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

/// How expensive a new hash is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HashCost {
    /// The live cost: m = 19456 KiB, t = 2, p = 1.
    Normal,
    /// For test servers only: m = 256 KiB, t = 1, p = 1. The server logs a
    /// warning when it is set; a deployed server never sets it.
    Test,
}

impl HashCost {
    fn params(self) -> Params {
        let (m, t) = match self {
            HashCost::Normal => (19_456, 2),
            HashCost::Test => (256, 1),
        };
        Params::new(m, t, 1, None).expect("valid argon2 parameters")
    }

    fn hasher(self) -> Argon2<'static> {
        Argon2::new(Algorithm::Argon2id, Version::V0x13, self.params())
    }
}

/// Hashes `password` with a fresh random salt.
pub fn hash(cost: HashCost, password: &str) -> String {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).expect("the operating system has a random source");
    let salt = SaltString::encode_b64(&bytes).expect("16 bytes make a valid salt");
    cost.hasher()
        .hash_password(password.as_bytes(), &salt)
        .expect("argon2 hashes any password of the allowed length")
        .to_string()
}

/// Whether `password` matches the stored `phc` string. A damaged string
/// matches nothing.
pub fn verify(phc: &str, password: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_verifies_its_password_only() {
        let h = hash(HashCost::Test, "correct horse");
        assert!(h.starts_with("$argon2id$v=19$m=256,t=1,p=1$"), "{h}");
        assert!(verify(&h, "correct horse"));
        assert!(!verify(&h, "correct horsf"));
        assert!(!verify("not a hash", "correct horse"));
    }

    #[test]
    fn salts_differ() {
        assert_ne!(
            hash(HashCost::Test, "same pass"),
            hash(HashCost::Test, "same pass")
        );
    }

    #[test]
    fn the_normal_cost_is_the_contract_one() {
        let h = hash(HashCost::Normal, "correct horse");
        assert!(h.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{h}");
        assert!(verify(&h, "correct horse"));
    }
}
