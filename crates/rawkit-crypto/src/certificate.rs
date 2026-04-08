use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::sign::{sign, verify};

/// A certificate grants write permission to a specific identity for a specific path.
///
/// - Issuer (the data owner) signs a certificate
/// - Certificate specifies WHO (grantee public key) can write WHERE (path pattern)
/// - Optional expiry for time-limited access
/// - Certificates are themselves graph nodes, verifiable by any peer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Certificate {
    /// Who issued this certificate (hex-encoded Ed25519 public key).
    pub issuer: String,
    /// Who is granted permission (hex-encoded Ed25519 public key).
    pub grantee: String,
    /// Path pattern the grantee can write to (e.g., "users/alice/posts/*").
    pub path: String,
    /// What operations are allowed.
    pub permissions: Permissions,
    /// Unix timestamp (ms) when this certificate was created.
    pub created_at: f64,
    /// Optional expiry timestamp (ms). None = never expires.
    pub expires_at: Option<f64>,
    /// Detached Ed25519 signature from the issuer over the certificate body.
    #[serde(with = "base64_bytes")]
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permissions {
    pub read: bool,
    pub write: bool,
    pub delete: bool,
}

impl Permissions {
    pub fn read_write() -> Self {
        Permissions {
            read: true,
            write: true,
            delete: false,
        }
    }

    pub fn full() -> Self {
        Permissions {
            read: true,
            write: true,
            delete: true,
        }
    }

    pub fn read_only() -> Self {
        Permissions {
            read: true,
            write: false,
            delete: false,
        }
    }
}

impl Certificate {
    /// Create and sign a new certificate.
    pub fn create(
        issuer_key: &SigningKey,
        grantee_pub_hex: &str,
        path: &str,
        permissions: Permissions,
        expires_at: Option<f64>,
    ) -> Self {
        let issuer = hex::encode(issuer_key.verifying_key().as_bytes());
        let created_at = crate::identity::now_ms();

        let mut cert = Certificate {
            issuer: issuer.clone(),
            grantee: grantee_pub_hex.to_string(),
            path: path.to_string(),
            permissions,
            created_at,
            expires_at,
            signature: Vec::new(),
        };

        let body = cert.canonical_body();
        cert.signature = sign(body.as_bytes(), issuer_key);
        cert
    }

    /// Verify the certificate's signature and check expiry.
    pub fn verify(&self, current_time: f64) -> Result<(), CertificateError> {
        // Check expiry
        if let Some(expires) = self.expires_at {
            if current_time > expires {
                return Err(CertificateError::Expired);
            }
        }

        // Verify signature
        let issuer_bytes =
            hex::decode(&self.issuer).map_err(|_| CertificateError::InvalidIssuer)?;
        let mut key_bytes = [0u8; 32];
        if issuer_bytes.len() != 32 {
            return Err(CertificateError::InvalidIssuer);
        }
        key_bytes.copy_from_slice(&issuer_bytes);

        let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&key_bytes)
            .map_err(|_| CertificateError::InvalidIssuer)?;

        let body = self.canonical_body();
        verify(body.as_bytes(), &self.signature, &verifying_key)
            .map_err(|_| CertificateError::InvalidSignature)
    }

    /// Check if this certificate grants access to the given path.
    pub fn allows(&self, path: &str, operation: Operation) -> bool {
        let perm_ok = match operation {
            Operation::Read => self.permissions.read,
            Operation::Write => self.permissions.write,
            Operation::Delete => self.permissions.delete,
        };

        perm_ok && self.path_matches(path)
    }

    fn path_matches(&self, path: &str) -> bool {
        if self.path.ends_with('*') {
            let prefix = &self.path[..self.path.len() - 1];
            path.starts_with(prefix)
        } else {
            self.path == path
        }
    }

    fn canonical_body(&self) -> String {
        format!(
            "{}\n{}\n{}\n{:?}\n{}\n{:?}",
            self.issuer, self.grantee, self.path, self.permissions, self.created_at, self.expires_at
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Operation {
    Read,
    Write,
    Delete,
}

#[derive(Debug, thiserror::Error)]
pub enum CertificateError {
    #[error("certificate has expired")]
    Expired,
    #[error("invalid issuer key")]
    InvalidIssuer,
    #[error("invalid signature")]
    InvalidSignature,
}

mod base64_bytes {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        STANDARD.decode(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_verify_certificate() {
        let issuer = SigningKey::generate(&mut rand::thread_rng());
        let grantee = SigningKey::generate(&mut rand::thread_rng());
        let grantee_pub = hex::encode(grantee.verifying_key().as_bytes());

        let cert = Certificate::create(
            &issuer,
            &grantee_pub,
            "users/alice/posts/*",
            Permissions::read_write(),
            None,
        );

        assert!(cert.verify(crate::identity::now_ms()).is_ok());
    }

    #[test]
    fn test_expired_certificate() {
        let issuer = SigningKey::generate(&mut rand::thread_rng());
        let cert = Certificate::create(
            &issuer,
            "deadbeef",
            "test/*",
            Permissions::read_only(),
            Some(1000.0), // expired long ago
        );

        assert!(matches!(
            cert.verify(crate::identity::now_ms()),
            Err(CertificateError::Expired)
        ));
    }

    #[test]
    fn test_path_matching() {
        let issuer = SigningKey::generate(&mut rand::thread_rng());
        let cert = Certificate::create(
            &issuer,
            "grantee",
            "users/alice/*",
            Permissions::full(),
            None,
        );

        assert!(cert.allows("users/alice/posts", Operation::Write));
        assert!(cert.allows("users/alice/profile", Operation::Read));
        assert!(!cert.allows("users/bob/posts", Operation::Write));
    }

    #[test]
    fn test_exact_path_matching() {
        let issuer = SigningKey::generate(&mut rand::thread_rng());
        let cert = Certificate::create(
            &issuer,
            "grantee",
            "users/alice/name",
            Permissions::read_write(),
            None,
        );

        assert!(cert.allows("users/alice/name", Operation::Write));
        assert!(!cert.allows("users/alice/age", Operation::Write));
    }
}
