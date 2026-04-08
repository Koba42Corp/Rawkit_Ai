use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    ChaCha20Poly1305, Nonce,
};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret as X25519Secret};
use sha2::{Digest, Sha256};
use rand::RngCore;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("encryption failed")]
    EncryptionFailed,
    #[error("decryption failed")]
    DecryptionFailed,
    #[error("invalid ciphertext format")]
    InvalidFormat,
}

/// Encrypt a message for a recipient using X25519 key exchange + ChaCha20-Poly1305.
///
/// Output format: nonce (12 bytes) || ciphertext (variable + 16 byte auth tag)
///
/// Uses ECDH shared secret between sender's private key and recipient's public key,
/// hashed with SHA-256 to derive the symmetric encryption key.
pub fn encrypt(
    plaintext: &[u8],
    recipient_public: &X25519PublicKey,
    sender_secret: &X25519Secret,
) -> Result<Vec<u8>, CryptoError> {
    // ECDH key exchange
    let shared_secret = sender_secret.diffie_hellman(recipient_public);

    // Derive symmetric key from shared secret
    let mut hasher = Sha256::new();
    hasher.update(shared_secret.as_bytes());
    let key_bytes = hasher.finalize();

    let cipher =
        ChaCha20Poly1305::new_from_slice(&key_bytes).map_err(|_| CryptoError::EncryptionFailed)?;

    // Random nonce (12 bytes)
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| CryptoError::EncryptionFailed)?;

    // Prepend nonce to ciphertext
    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    Ok(output)
}

/// Decrypt a message from a sender using X25519 key exchange + ChaCha20-Poly1305.
///
/// Expects input format: nonce (12 bytes) || ciphertext (variable + 16 byte auth tag)
pub fn decrypt(
    data: &[u8],
    sender_public: &X25519PublicKey,
    recipient_secret: &X25519Secret,
) -> Result<Vec<u8>, CryptoError> {
    if data.len() < 12 + 16 {
        return Err(CryptoError::InvalidFormat);
    }

    // ECDH key exchange (same shared secret from either direction)
    let shared_secret = recipient_secret.diffie_hellman(sender_public);

    let mut hasher = Sha256::new();
    hasher.update(shared_secret.as_bytes());
    let key_bytes = hasher.finalize();

    let cipher =
        ChaCha20Poly1305::new_from_slice(&key_bytes).map_err(|_| CryptoError::DecryptionFailed)?;

    let nonce = Nonce::from_slice(&data[..12]);
    let ciphertext = &data[12..];

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| CryptoError::DecryptionFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use x25519_dalek::StaticSecret;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let alice_secret = StaticSecret::random_from_rng(OsRng);
        let alice_public = X25519PublicKey::from(&alice_secret);

        let bob_secret = StaticSecret::random_from_rng(OsRng);
        let bob_public = X25519PublicKey::from(&bob_secret);

        let message = b"Hello, Bob! This is a secret message.";

        // Alice encrypts for Bob
        let encrypted = encrypt(message, &bob_public, &alice_secret).unwrap();
        assert_ne!(&encrypted[12..], message); // ciphertext != plaintext

        // Bob decrypts from Alice
        let decrypted = decrypt(&encrypted, &alice_public, &bob_secret).unwrap();
        assert_eq!(&decrypted, message);
    }

    #[test]
    fn test_wrong_key_fails() {
        let alice_secret = StaticSecret::random_from_rng(OsRng);
        let bob_secret = StaticSecret::random_from_rng(OsRng);
        let bob_public = X25519PublicKey::from(&bob_secret);
        let eve_secret = StaticSecret::random_from_rng(OsRng);

        let encrypted = encrypt(b"secret", &bob_public, &alice_secret).unwrap();

        // Eve tries to decrypt with wrong key
        let eve_public = X25519PublicKey::from(&eve_secret);
        let result = decrypt(&encrypted, &eve_public, &bob_secret);
        assert!(result.is_err());
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let alice_secret = StaticSecret::random_from_rng(OsRng);
        let alice_public = X25519PublicKey::from(&alice_secret);
        let bob_secret = StaticSecret::random_from_rng(OsRng);
        let bob_public = X25519PublicKey::from(&bob_secret);

        let mut encrypted = encrypt(b"secret", &bob_public, &alice_secret).unwrap();
        // Tamper with the ciphertext
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0xFF;

        let result = decrypt(&encrypted, &alice_public, &bob_secret);
        assert!(result.is_err());
    }

    #[test]
    fn test_short_input_fails() {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = X25519PublicKey::from(&secret);
        let result = decrypt(&[0u8; 10], &public, &secret);
        assert!(result.is_err());
    }
}
