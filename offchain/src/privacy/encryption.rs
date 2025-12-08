//! Encryption Module for PII Protection
//!
//! Provides AES-256-GCM encryption for sensitive data before IPFS storage.
//! Implements envelope encryption pattern for secure key management.
//!
//! SECURITY: All PII must be encrypted before IPFS upload.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use zeroize::Zeroizing;

/// Encryption configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionConfig {
    /// Algorithm identifier
    pub algorithm: String,
    /// Key size in bits
    pub key_size: usize,
    /// Nonce size in bytes
    pub nonce_size: usize,
    /// Whether to include integrity check
    pub include_integrity_check: bool,
}

impl Default for EncryptionConfig {
    fn default() -> Self {
        Self {
            algorithm: "AES-256-GCM".to_string(),
            key_size: 256,
            nonce_size: 12,
            include_integrity_check: true,
        }
    }
}

/// Encryption key wrapper with secure memory handling
#[derive(Clone)]
pub struct EncryptionKey {
    /// The raw key bytes (zeroized on drop)
    key: Zeroizing<Vec<u8>>,
    /// Key identifier for tracking
    key_id: String,
    /// Creation timestamp
    created_at: String,
}

impl EncryptionKey {
    /// Create a new random encryption key
    pub fn generate(key_id: &str) -> Self {
        let mut key_bytes = vec![0u8; 32]; // 256 bits
        OsRng.fill_bytes(&mut key_bytes);

        Self {
            key: Zeroizing::new(key_bytes),
            key_id: key_id.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Create from existing key bytes
    pub fn from_bytes(key_bytes: Vec<u8>, key_id: &str) -> Result<Self> {
        if key_bytes.len() != 32 {
            anyhow::bail!("Invalid key size: expected 32 bytes, got {}", key_bytes.len());
        }

        Ok(Self {
            key: Zeroizing::new(key_bytes),
            key_id: key_id.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    /// Get the key ID
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// Get the raw key bytes (for encryption operations)
    pub fn as_bytes(&self) -> &[u8] {
        &self.key
    }

    /// Export key as base64 (for secure storage)
    pub fn export_base64(&self) -> String {
        BASE64.encode(&*self.key)
    }

    /// Import key from base64
    pub fn from_base64(encoded: &str, key_id: &str) -> Result<Self> {
        let key_bytes = BASE64
            .decode(encoded)
            .context("Failed to decode base64 key")?;
        Self::from_bytes(key_bytes, key_id)
    }

    /// Get key fingerprint (hash of key for identification without exposing key)
    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(&*self.key);
        let result = hasher.finalize();
        format!("{:x}", result)[..16].to_string()
    }
}

impl fmt::Debug for EncryptionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncryptionKey")
            .field("key_id", &self.key_id)
            .field("fingerprint", &self.fingerprint())
            .field("created_at", &self.created_at)
            .finish()
    }
}

/// Encrypted data envelope containing ciphertext and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedEnvelope {
    /// Version of the envelope format
    pub version: String,
    /// Encryption algorithm used
    pub algorithm: String,
    /// Key ID used for encryption
    pub key_id: String,
    /// Base64-encoded nonce/IV
    pub nonce: String,
    /// Base64-encoded ciphertext (includes auth tag for AEAD)
    pub ciphertext: String,
    /// SHA-256 hash of the plaintext (for integrity verification)
    pub plaintext_hash: Option<String>,
    /// Timestamp of encryption
    pub encrypted_at: String,
    /// Additional authenticated data (AAD) if used
    pub aad: Option<String>,
}

impl EncryptedEnvelope {
    /// Check if this envelope can be decrypted with a given key
    pub fn matches_key(&self, key: &EncryptionKey) -> bool {
        self.key_id == key.key_id()
    }

    /// Get the size of the encrypted data
    pub fn ciphertext_size(&self) -> usize {
        BASE64.decode(&self.ciphertext).map(|b| b.len()).unwrap_or(0)
    }
}

/// Data encryptor for PII protection
#[derive(Clone)]
pub struct DataEncryptor {
    config: EncryptionConfig,
}

impl DataEncryptor {
    /// Create a new data encryptor
    pub fn new(config: EncryptionConfig) -> Self {
        Self { config }
    }

    /// Create with default configuration
    pub fn default_encryptor() -> Self {
        Self::new(EncryptionConfig::default())
    }

    /// Encrypt data using AES-256-GCM
    pub fn encrypt(&self, plaintext: &[u8], key: &EncryptionKey) -> Result<EncryptedEnvelope> {
        // Generate random nonce
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Create cipher
        let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
            .map_err(|e| anyhow::anyhow!("Failed to create cipher: {}", e))?;

        // Encrypt
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

        // Calculate plaintext hash if configured
        let plaintext_hash = if self.config.include_integrity_check {
            let mut hasher = Sha256::new();
            hasher.update(plaintext);
            Some(format!("{:x}", hasher.finalize()))
        } else {
            None
        };

        Ok(EncryptedEnvelope {
            version: "1.0".to_string(),
            algorithm: self.config.algorithm.clone(),
            key_id: key.key_id().to_string(),
            nonce: BASE64.encode(nonce_bytes),
            ciphertext: BASE64.encode(&ciphertext),
            plaintext_hash,
            encrypted_at: chrono::Utc::now().to_rfc3339(),
            aad: None,
        })
    }

    /// Encrypt with additional authenticated data (AAD)
    pub fn encrypt_with_aad(
        &self,
        plaintext: &[u8],
        key: &EncryptionKey,
        aad: &[u8],
    ) -> Result<EncryptedEnvelope> {
        // Generate random nonce
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Create cipher
        let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
            .map_err(|e| anyhow::anyhow!("Failed to create cipher: {}", e))?;

        // Encrypt with AAD
        use aes_gcm::aead::Payload;
        let payload = Payload {
            msg: plaintext,
            aad,
        };

        let ciphertext = cipher
            .encrypt(nonce, payload)
            .map_err(|e| anyhow::anyhow!("Encryption with AAD failed: {}", e))?;

        // Calculate plaintext hash
        let plaintext_hash = if self.config.include_integrity_check {
            let mut hasher = Sha256::new();
            hasher.update(plaintext);
            Some(format!("{:x}", hasher.finalize()))
        } else {
            None
        };

        Ok(EncryptedEnvelope {
            version: "1.0".to_string(),
            algorithm: self.config.algorithm.clone(),
            key_id: key.key_id().to_string(),
            nonce: BASE64.encode(nonce_bytes),
            ciphertext: BASE64.encode(&ciphertext),
            plaintext_hash,
            encrypted_at: chrono::Utc::now().to_rfc3339(),
            aad: Some(BASE64.encode(aad)),
        })
    }

    /// Decrypt data
    pub fn decrypt(&self, envelope: &EncryptedEnvelope, key: &EncryptionKey) -> Result<Vec<u8>> {
        // Verify key matches
        if !envelope.matches_key(key) {
            anyhow::bail!(
                "Key mismatch: envelope expects '{}', got '{}'",
                envelope.key_id,
                key.key_id()
            );
        }

        // Decode nonce
        let nonce_bytes = BASE64
            .decode(&envelope.nonce)
            .context("Failed to decode nonce")?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Decode ciphertext
        let ciphertext = BASE64
            .decode(&envelope.ciphertext)
            .context("Failed to decode ciphertext")?;

        // Create cipher
        let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
            .map_err(|e| anyhow::anyhow!("Failed to create cipher: {}", e))?;

        // Decrypt
        let plaintext = if let Some(ref aad_b64) = envelope.aad {
            let aad = BASE64.decode(aad_b64).context("Failed to decode AAD")?;
            use aes_gcm::aead::Payload;
            let payload = Payload {
                msg: &ciphertext,
                aad: &aad,
            };
            cipher
                .decrypt(nonce, payload)
                .map_err(|e| anyhow::anyhow!("Decryption with AAD failed: {}", e))?
        } else {
            cipher
                .decrypt(nonce, ciphertext.as_slice())
                .map_err(|e| anyhow::anyhow!("Decryption failed: {}", e))?
        };

        // Verify integrity if hash is present
        if let Some(ref expected_hash) = envelope.plaintext_hash {
            let mut hasher = Sha256::new();
            hasher.update(&plaintext);
            let actual_hash = format!("{:x}", hasher.finalize());

            if &actual_hash != expected_hash {
                anyhow::bail!(
                    "Integrity check failed: plaintext hash mismatch"
                );
            }
        }

        Ok(plaintext)
    }

    /// Encrypt JSON data
    pub fn encrypt_json<T: Serialize>(
        &self,
        data: &T,
        key: &EncryptionKey,
    ) -> Result<EncryptedEnvelope> {
        let json_bytes = serde_json::to_vec(data).context("Failed to serialize JSON")?;
        self.encrypt(&json_bytes, key)
    }

    /// Decrypt to JSON
    pub fn decrypt_json<T: for<'de> Deserialize<'de>>(
        &self,
        envelope: &EncryptedEnvelope,
        key: &EncryptionKey,
    ) -> Result<T> {
        let plaintext = self.decrypt(envelope, key)?;
        let data: T = serde_json::from_slice(&plaintext).context("Failed to deserialize JSON")?;
        Ok(data)
    }

    /// Re-encrypt data with a new key (for key rotation)
    pub fn reencrypt(
        &self,
        envelope: &EncryptedEnvelope,
        old_key: &EncryptionKey,
        new_key: &EncryptionKey,
    ) -> Result<EncryptedEnvelope> {
        let plaintext = self.decrypt(envelope, old_key)?;
        self.encrypt(&plaintext, new_key)
    }
}

impl Default for DataEncryptor {
    fn default() -> Self {
        Self::default_encryptor()
    }
}

/// Utility function to generate a secure random string
pub fn generate_random_string(length: usize) -> String {
    let mut bytes = vec![0u8; length];
    OsRng.fill_bytes(&mut bytes);
    BASE64.encode(&bytes)[..length].to_string()
}

/// Utility function to hash data for integrity checking
pub fn hash_data(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_generation() {
        let key = EncryptionKey::generate("test-key-1");
        assert_eq!(key.key_id(), "test-key-1");
        assert_eq!(key.as_bytes().len(), 32);
    }

    #[test]
    fn test_key_export_import() {
        let key1 = EncryptionKey::generate("test-key");
        let exported = key1.export_base64();

        let key2 = EncryptionKey::from_base64(&exported, "test-key").unwrap();
        assert_eq!(key1.as_bytes(), key2.as_bytes());
        assert_eq!(key1.fingerprint(), key2.fingerprint());
    }

    #[test]
    fn test_basic_encryption_decryption() {
        let encryptor = DataEncryptor::default_encryptor();
        let key = EncryptionKey::generate("test-key");

        let plaintext = b"Hello, World! This is sensitive data.";
        let envelope = encryptor.encrypt(plaintext, &key).unwrap();

        assert_eq!(envelope.algorithm, "AES-256-GCM");
        assert_eq!(envelope.key_id, "test-key");
        assert!(envelope.plaintext_hash.is_some());

        let decrypted = encryptor.decrypt(&envelope, &key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encryption_with_aad() {
        let encryptor = DataEncryptor::default_encryptor();
        let key = EncryptionKey::generate("test-key");

        let plaintext = b"Sensitive PII data";
        let aad = b"entity:farmer:123";

        let envelope = encryptor.encrypt_with_aad(plaintext, &key, aad).unwrap();
        assert!(envelope.aad.is_some());

        let decrypted = encryptor.decrypt(&envelope, &key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_wrong_key_fails() {
        let encryptor = DataEncryptor::default_encryptor();
        let key1 = EncryptionKey::generate("key-1");
        let key2 = EncryptionKey::generate("key-2");

        let plaintext = b"Secret data";
        let envelope = encryptor.encrypt(plaintext, &key1).unwrap();

        let result = encryptor.decrypt(&envelope, &key2);
        assert!(result.is_err());
    }

    #[test]
    fn test_json_encryption() {
        let encryptor = DataEncryptor::default_encryptor();
        let key = EncryptionKey::generate("test-key");

        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct TestData {
            name: String,
            phone: String,
            age: u32,
        }

        let data = TestData {
            name: "John Doe".to_string(),
            phone: "+91-9876543210".to_string(),
            age: 30,
        };

        let envelope = encryptor.encrypt_json(&data, &key).unwrap();
        let decrypted: TestData = encryptor.decrypt_json(&envelope, &key).unwrap();

        assert_eq!(data, decrypted);
    }

    #[test]
    fn test_reencryption() {
        let encryptor = DataEncryptor::default_encryptor();
        let old_key = EncryptionKey::generate("old-key");
        let new_key = EncryptionKey::generate("new-key");

        let plaintext = b"Data to rotate keys";
        let envelope1 = encryptor.encrypt(plaintext, &old_key).unwrap();

        let envelope2 = encryptor.reencrypt(&envelope1, &old_key, &new_key).unwrap();
        assert_eq!(envelope2.key_id, "new-key");

        let decrypted = encryptor.decrypt(&envelope2, &new_key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_integrity_check() {
        let encryptor = DataEncryptor::default_encryptor();
        let key = EncryptionKey::generate("test-key");

        let plaintext = b"Test data for integrity";
        let envelope = encryptor.encrypt(plaintext, &key).unwrap();

        // Verify integrity hash exists
        assert!(envelope.plaintext_hash.is_some());

        // Successful decryption verifies integrity
        let decrypted = encryptor.decrypt(&envelope, &key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_key_fingerprint() {
        let key1 = EncryptionKey::generate("key-1");
        let key2 = EncryptionKey::generate("key-2");

        // Different keys should have different fingerprints
        assert_ne!(key1.fingerprint(), key2.fingerprint());

        // Same key bytes should have same fingerprint
        let key3 = EncryptionKey::from_bytes(key1.as_bytes().to_vec(), "key-3").unwrap();
        assert_eq!(key1.fingerprint(), key3.fingerprint());
    }

    #[test]
    fn test_envelope_matches_key() {
        let encryptor = DataEncryptor::default_encryptor();
        let key1 = EncryptionKey::generate("key-1");
        let key2 = EncryptionKey::generate("key-2");

        let envelope = encryptor.encrypt(b"test", &key1).unwrap();

        assert!(envelope.matches_key(&key1));
        assert!(!envelope.matches_key(&key2));
    }

    #[test]
    fn test_random_string_generation() {
        let s1 = generate_random_string(16);
        let s2 = generate_random_string(16);

        assert_eq!(s1.len(), 16);
        assert_eq!(s2.len(), 16);
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_hash_data() {
        let data = b"test data";
        let hash1 = hash_data(data);
        let hash2 = hash_data(data);

        // Same data should produce same hash
        assert_eq!(hash1, hash2);

        // Different data should produce different hash
        let hash3 = hash_data(b"different data");
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_large_data_encryption() {
        let encryptor = DataEncryptor::default_encryptor();
        let key = EncryptionKey::generate("test-key");

        // 1MB of data
        let plaintext = vec![0xABu8; 1024 * 1024];
        let envelope = encryptor.encrypt(&plaintext, &key).unwrap();

        let decrypted = encryptor.decrypt(&envelope, &key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_config_without_integrity_check() {
        let config = EncryptionConfig {
            include_integrity_check: false,
            ..Default::default()
        };
        let encryptor = DataEncryptor::new(config);
        let key = EncryptionKey::generate("test-key");

        let envelope = encryptor.encrypt(b"test", &key).unwrap();
        assert!(envelope.plaintext_hash.is_none());
    }
}
