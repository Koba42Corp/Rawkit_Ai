use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha512};
use x25519_dalek::{StaticSecret as X25519Secret, PublicKey as X25519PublicKey};
use zeroize::Zeroize;
use serde::{Deserialize, Serialize};

/// Get current time in milliseconds since Unix epoch.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64
}

/// The chain type that produced the wallet signature used for identity derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChainType {
    Chia,
    Evm,
    Solana,
    Bitcoin,
    Standalone,
}

/// A Rawkit identity derived from a wallet signature.
///
/// The derivation process:
/// 1. Wallet signs a deterministic challenge message
/// 2. Signature is hashed with SHA-512 (64 bytes)
/// 3. First 32 bytes -> Ed25519 signing key
/// 4. Bytes 32..64 -> X25519 encryption key
///
/// This gives deterministic key derivation: same wallet always produces the same identity.
/// Compatible with TangTalk's proven derivation scheme.
#[derive(Clone)]
pub struct Identity {
    /// The wallet address this identity was derived from.
    pub address: String,
    /// Which blockchain the wallet belongs to.
    pub chain: ChainType,
    /// Ed25519 signing keypair.
    pub signing_key: SigningKey,
    /// X25519 encryption secret.
    pub encryption_secret: X25519Secret,
    /// Public encryption key (safe to share).
    pub encryption_public: X25519PublicKey,
}

impl Identity {
    /// The challenge message that wallets sign to derive identity.
    pub const CHALLENGE: &'static str =
        "Rawkit Identity Derivation - Sign to create your decentralized identity";

    /// Derive an identity from a wallet signature.
    ///
    /// The signature bytes are hashed to produce deterministic key material.
    /// This means the same wallet signature always produces the same identity.
    pub fn from_wallet_signature(
        address: impl Into<String>,
        chain: ChainType,
        signature: &[u8],
    ) -> Self {
        // SHA-512 hash of signature -> 64 bytes of key material
        let mut hasher = Sha512::new();
        hasher.update(signature);
        let hash = hasher.finalize();
        let mut key_material: [u8; 64] = hash.into();

        // First 32 bytes -> Ed25519 signing key
        let mut signing_seed = [0u8; 32];
        signing_seed.copy_from_slice(&key_material[..32]);
        let signing_key = SigningKey::from_bytes(&signing_seed);

        // Bytes 32..64 -> X25519 encryption key
        let mut encryption_seed = [0u8; 32];
        encryption_seed.copy_from_slice(&key_material[32..64]);
        let encryption_secret = X25519Secret::from(encryption_seed);
        let encryption_public = X25519PublicKey::from(&encryption_secret);

        // Zeroize sensitive material
        key_material.zeroize();
        signing_seed.zeroize();
        encryption_seed.zeroize();

        Identity {
            address: address.into(),
            chain,
            signing_key,
            encryption_secret,
            encryption_public,
        }
    }

    /// Create a standalone identity (no wallet required).
    /// Generates random keys.
    pub fn generate_standalone() -> Self {
        let mut rng = rand::thread_rng();
        let signing_key = SigningKey::generate(&mut rng);

        let encryption_seed: [u8; 32] = rand::Rng::gen(&mut rng);
        let encryption_secret = X25519Secret::from(encryption_seed);
        let encryption_public = X25519PublicKey::from(&encryption_secret);

        Identity {
            address: format!("rawkit:{}", hex::encode(signing_key.verifying_key().as_bytes())),
            chain: ChainType::Standalone,
            signing_key,
            encryption_secret,
            encryption_public,
        }
    }

    /// Get the public signing key (verifying key).
    pub fn verifying_key(&self) -> ed25519_dalek::VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Export the public identity (safe to share with peers).
    pub fn to_public(&self) -> PublicIdentity {
        PublicIdentity {
            address: self.address.clone(),
            chain: self.chain,
            signing_key: self.verifying_key().to_bytes(),
            encryption_key: self.encryption_public.to_bytes(),
        }
    }
}

/// The public portion of an identity (safe to publish to peers).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicIdentity {
    pub address: String,
    pub chain: ChainType,
    #[serde(with = "hex_bytes")]
    pub signing_key: [u8; 32],
    #[serde(with = "hex_bytes")]
    pub encryption_key: [u8; 32],
}

mod hex_bytes {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        let mut arr = [0u8; 32];
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom("expected 32 bytes"));
        }
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic_derivation() {
        let sig = b"test-wallet-signature-bytes-here-padding-to-fill";

        let id1 = Identity::from_wallet_signature("0xABCD", ChainType::Evm, sig);
        let id2 = Identity::from_wallet_signature("0xABCD", ChainType::Evm, sig);

        assert_eq!(
            id1.verifying_key().as_bytes(),
            id2.verifying_key().as_bytes()
        );
        assert_eq!(
            id1.encryption_public.as_bytes(),
            id2.encryption_public.as_bytes()
        );
    }

    #[test]
    fn test_different_sigs_different_keys() {
        let id1 = Identity::from_wallet_signature("addr1", ChainType::Chia, b"signature-one-pad");
        let id2 = Identity::from_wallet_signature("addr2", ChainType::Chia, b"signature-two-pad");

        assert_ne!(
            id1.verifying_key().as_bytes(),
            id2.verifying_key().as_bytes()
        );
    }

    #[test]
    fn test_standalone_identity() {
        let id = Identity::generate_standalone();
        assert!(id.address.starts_with("rawkit:"));
        assert_eq!(id.chain, ChainType::Standalone);
    }

    #[test]
    fn test_public_identity_serialization() {
        let id = Identity::generate_standalone();
        let public = id.to_public();

        let json = serde_json::to_string(&public).unwrap();
        let deserialized: PublicIdentity = serde_json::from_str(&json).unwrap();

        assert_eq!(public.address, deserialized.address);
        assert_eq!(public.signing_key, deserialized.signing_key);
        assert_eq!(public.encryption_key, deserialized.encryption_key);
    }
}
