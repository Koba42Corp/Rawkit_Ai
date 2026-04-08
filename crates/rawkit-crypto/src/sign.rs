use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

#[derive(Debug, thiserror::Error)]
pub enum SignError {
    #[error("signing failed: {0}")]
    SigningFailed(String),
    #[error("verification failed")]
    VerificationFailed,
    #[error("invalid signature format")]
    InvalidFormat,
}

/// Sign data with an Ed25519 signing key.
/// Returns a 64-byte detached signature.
pub fn sign(data: &[u8], signing_key: &SigningKey) -> Vec<u8> {
    let signature: Signature = signing_key.sign(data);
    signature.to_bytes().to_vec()
}

/// Verify a detached Ed25519 signature.
pub fn verify(data: &[u8], signature_bytes: &[u8], verifying_key: &VerifyingKey) -> Result<(), SignError> {
    let signature =
        Signature::from_slice(signature_bytes).map_err(|_| SignError::InvalidFormat)?;
    verifying_key
        .verify(data, &signature)
        .map_err(|_| SignError::VerificationFailed)
}

/// Sign a graph update (soul + key + value + state) for authenticated writes.
/// This creates a canonical representation of the update and signs it.
pub fn sign_update(
    soul: &str,
    key: &str,
    value_json: &str,
    state: f64,
    signing_key: &SigningKey,
) -> Vec<u8> {
    let canonical = format!("{soul}\n{key}\n{value_json}\n{state}");
    sign(canonical.as_bytes(), signing_key)
}

/// Verify a signed graph update.
pub fn verify_update(
    soul: &str,
    key: &str,
    value_json: &str,
    state: f64,
    signature_bytes: &[u8],
    verifying_key: &VerifyingKey,
) -> Result<(), SignError> {
    let canonical = format!("{soul}\n{key}\n{value_json}\n{state}");
    verify(canonical.as_bytes(), signature_bytes, verifying_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_verify_roundtrip() {
        let signing_key = SigningKey::generate(&mut rand::thread_rng());
        let verifying_key = signing_key.verifying_key();

        let data = b"Hello, Rawkit!";
        let sig = sign(data, &signing_key);

        assert!(verify(data, &sig, &verifying_key).is_ok());
    }

    #[test]
    fn test_wrong_key_verification_fails() {
        let key1 = SigningKey::generate(&mut rand::thread_rng());
        let key2 = SigningKey::generate(&mut rand::thread_rng());

        let data = b"test data";
        let sig = sign(data, &key1);

        assert!(verify(data, &sig, &key2.verifying_key()).is_err());
    }

    #[test]
    fn test_tampered_data_fails() {
        let key = SigningKey::generate(&mut rand::thread_rng());

        let sig = sign(b"original", &key);
        assert!(verify(b"tampered", &sig, &key.verifying_key()).is_err());
    }

    #[test]
    fn test_sign_verify_update() {
        let key = SigningKey::generate(&mut rand::thread_rng());

        let sig = sign_update("users/alice", "name", "\"Alice\"", 1000.0, &key);
        assert!(
            verify_update("users/alice", "name", "\"Alice\"", 1000.0, &sig, &key.verifying_key())
                .is_ok()
        );

        // Tampered soul
        assert!(
            verify_update("users/bob", "name", "\"Alice\"", 1000.0, &sig, &key.verifying_key())
                .is_err()
        );
    }
}
