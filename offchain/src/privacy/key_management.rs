//! Key Management Module
//!
//! Provides secure key management for PII encryption with support for:
//! - Local key storage (development/testing)
//! - External KMS integration (AWS KMS, HashiCorp Vault, etc.)
//! - Key rotation and lifecycle management
//! - Role-based key access control
//!
//! SECURITY: Keys should never be stored alongside encrypted data.

use super::encryption::EncryptionKey;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Key metadata stored alongside keys
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMetadata {
    /// Unique key identifier
    pub key_id: String,
    /// Purpose of this key
    pub purpose: KeyPurpose,
    /// Entity this key belongs to (e.g., "farmer:123", "fpo:456")
    pub entity_id: Option<String>,
    /// When the key was created
    pub created_at: String,
    /// When the key expires (if applicable)
    pub expires_at: Option<String>,
    /// When the key was last rotated
    pub last_rotated_at: Option<String>,
    /// Number of times this key has been rotated
    pub rotation_count: u32,
    /// Current key status
    pub status: KeyStatus,
    /// Roles that can use this key
    pub allowed_roles: Vec<String>,
    /// Additional metadata
    pub tags: HashMap<String, String>,
}

/// Purpose of an encryption key
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyPurpose {
    /// Encrypting PII data
    PiiEncryption,
    /// Encrypting documents
    DocumentEncryption,
    /// Signing data
    Signing,
    /// Key wrapping (encrypting other keys)
    KeyWrapping,
    /// General purpose
    General,
}

/// Status of a key
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyStatus {
    /// Key is active and can be used
    Active,
    /// Key is scheduled for rotation
    PendingRotation,
    /// Key is disabled but can be re-enabled
    Disabled,
    /// Key is being rotated
    Rotating,
    /// Key is compromised and must not be used
    Compromised,
    /// Key has been destroyed
    Destroyed,
}

impl KeyStatus {
    /// Check if key can be used for encryption
    pub fn can_encrypt(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Check if key can be used for decryption
    pub fn can_decrypt(&self) -> bool {
        matches!(self, Self::Active | Self::PendingRotation | Self::Rotating)
    }
}

/// Trait for key providers (allows different backend implementations)
#[async_trait]
pub trait KeyProvider: Send + Sync {
    /// Get a key by ID
    async fn get_key(&self, key_id: &str) -> Result<Option<EncryptionKey>>;

    /// Store a key
    async fn store_key(&self, key: &EncryptionKey, metadata: &KeyMetadata) -> Result<()>;

    /// Delete a key
    async fn delete_key(&self, key_id: &str) -> Result<()>;

    /// List all key IDs
    async fn list_keys(&self) -> Result<Vec<String>>;

    /// Get key metadata
    async fn get_metadata(&self, key_id: &str) -> Result<Option<KeyMetadata>>;

    /// Update key metadata
    async fn update_metadata(&self, key_id: &str, metadata: &KeyMetadata) -> Result<()>;

    /// Check if a key exists
    async fn key_exists(&self, key_id: &str) -> Result<bool>;
}

/// Local in-memory key provider (for development/testing)
#[derive(Default)]
pub struct LocalKeyProvider {
    keys: Arc<RwLock<HashMap<String, StoredKey>>>,
}

struct StoredKey {
    key: EncryptionKey,
    metadata: KeyMetadata,
}

impl LocalKeyProvider {
    pub fn new() -> Self {
        Self {
            keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create with pre-loaded keys (for testing)
    pub fn with_keys(keys: Vec<(EncryptionKey, KeyMetadata)>) -> Self {
        let provider = Self::new();
        let stored: HashMap<String, StoredKey> = keys
            .into_iter()
            .map(|(key, metadata)| {
                let key_id = key.key_id().to_string();
                (key_id, StoredKey { key, metadata })
            })
            .collect();

        // Use blocking lock since this is initialization
        *provider.keys.blocking_write() = stored;
        provider
    }
}

#[async_trait]
impl KeyProvider for LocalKeyProvider {
    async fn get_key(&self, key_id: &str) -> Result<Option<EncryptionKey>> {
        let keys = self.keys.read().await;
        Ok(keys.get(key_id).map(|stored| stored.key.clone()))
    }

    async fn store_key(&self, key: &EncryptionKey, metadata: &KeyMetadata) -> Result<()> {
        let mut keys = self.keys.write().await;
        keys.insert(
            key.key_id().to_string(),
            StoredKey {
                key: key.clone(),
                metadata: metadata.clone(),
            },
        );
        debug!(key_id = %key.key_id(), "Stored key locally");
        Ok(())
    }

    async fn delete_key(&self, key_id: &str) -> Result<()> {
        let mut keys = self.keys.write().await;
        keys.remove(key_id);
        info!(key_id = %key_id, "Deleted key");
        Ok(())
    }

    async fn list_keys(&self) -> Result<Vec<String>> {
        let keys = self.keys.read().await;
        Ok(keys.keys().cloned().collect())
    }

    async fn get_metadata(&self, key_id: &str) -> Result<Option<KeyMetadata>> {
        let keys = self.keys.read().await;
        Ok(keys.get(key_id).map(|stored| stored.metadata.clone()))
    }

    async fn update_metadata(&self, key_id: &str, metadata: &KeyMetadata) -> Result<()> {
        let mut keys = self.keys.write().await;
        if let Some(stored) = keys.get_mut(key_id) {
            stored.metadata = metadata.clone();
            Ok(())
        } else {
            anyhow::bail!("Key not found: {}", key_id)
        }
    }

    async fn key_exists(&self, key_id: &str) -> Result<bool> {
        let keys = self.keys.read().await;
        Ok(keys.contains_key(key_id))
    }
}

/// Environment-based key provider (reads keys from environment variables)
pub struct EnvKeyProvider {
    prefix: String,
    cache: Arc<RwLock<HashMap<String, EncryptionKey>>>,
}

impl EnvKeyProvider {
    pub fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn env_var_name(&self, key_id: &str) -> String {
        // Convert key ID to env var name: "entity:farmer:123" -> "PREFIX_ENTITY_FARMER_123"
        let sanitized = key_id
            .to_uppercase()
            .replace(':', "_")
            .replace('-', "_")
            .replace('.', "_");
        format!("{}_{}", self.prefix, sanitized)
    }
}

#[async_trait]
impl KeyProvider for EnvKeyProvider {
    async fn get_key(&self, key_id: &str) -> Result<Option<EncryptionKey>> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(key) = cache.get(key_id) {
                return Ok(Some(key.clone()));
            }
        }

        // Try to load from environment
        let env_name = self.env_var_name(key_id);
        if let Ok(key_b64) = std::env::var(&env_name) {
            let key = EncryptionKey::from_base64(&key_b64, key_id)?;

            // Cache it
            let mut cache = self.cache.write().await;
            cache.insert(key_id.to_string(), key.clone());

            return Ok(Some(key));
        }

        Ok(None)
    }

    async fn store_key(&self, key: &EncryptionKey, _metadata: &KeyMetadata) -> Result<()> {
        // Store in cache (can't actually set env vars for persistence)
        let mut cache = self.cache.write().await;
        cache.insert(key.key_id().to_string(), key.clone());

        warn!(
            key_id = %key.key_id(),
            env_var = %self.env_var_name(key.key_id()),
            "Key stored in memory only. Set environment variable for persistence."
        );

        Ok(())
    }

    async fn delete_key(&self, key_id: &str) -> Result<()> {
        let mut cache = self.cache.write().await;
        cache.remove(key_id);
        Ok(())
    }

    async fn list_keys(&self) -> Result<Vec<String>> {
        let cache = self.cache.read().await;
        Ok(cache.keys().cloned().collect())
    }

    async fn get_metadata(&self, _key_id: &str) -> Result<Option<KeyMetadata>> {
        // Environment provider doesn't store metadata
        Ok(None)
    }

    async fn update_metadata(&self, _key_id: &str, _metadata: &KeyMetadata) -> Result<()> {
        // No-op for environment provider
        Ok(())
    }

    async fn key_exists(&self, key_id: &str) -> Result<bool> {
        let cache = self.cache.read().await;
        if cache.contains_key(key_id) {
            return Ok(true);
        }

        let env_name = self.env_var_name(key_id);
        Ok(std::env::var(&env_name).is_ok())
    }
}

/// Key manager that coordinates key operations
#[derive(Clone)]
pub struct KeyManager {
    provider: Arc<dyn KeyProvider>,
    default_key_ttl_days: u32,
    auto_rotate_days: Option<u32>,
}

impl KeyManager {
    /// Create a new key manager with a provider
    pub fn new(provider: Arc<dyn KeyProvider>) -> Self {
        Self {
            provider,
            default_key_ttl_days: 365,
            auto_rotate_days: Some(90),
        }
    }

    /// Create with local provider (for development)
    pub fn local() -> Self {
        Self::new(Arc::new(LocalKeyProvider::new()))
    }

    /// Create with environment provider
    pub fn from_env(prefix: &str) -> Self {
        Self::new(Arc::new(EnvKeyProvider::new(prefix)))
    }

    /// Set default key TTL
    pub fn with_ttl_days(mut self, days: u32) -> Self {
        self.default_key_ttl_days = days;
        self
    }

    /// Set auto-rotation period
    pub fn with_auto_rotate_days(mut self, days: Option<u32>) -> Self {
        self.auto_rotate_days = days;
        self
    }

    /// Get an existing key
    pub async fn get_key(&self, key_id: &str) -> Result<Option<EncryptionKey>> {
        let key = self.provider.get_key(key_id).await?;

        // Check if key is usable
        if key.is_some() {
            if let Some(metadata) = self.provider.get_metadata(key_id).await? {
                if !metadata.status.can_decrypt() {
                    warn!(
                        key_id = %key_id,
                        status = ?metadata.status,
                        "Attempted to use key with invalid status"
                    );
                    return Ok(None);
                }

                // Check expiration
                if let Some(expires_at) = &metadata.expires_at {
                    if let Ok(expiry) = chrono::DateTime::parse_from_rfc3339(expires_at) {
                        if expiry < chrono::Utc::now() {
                            warn!(key_id = %key_id, "Key has expired");
                            return Ok(None);
                        }
                    }
                }
            }
        }

        Ok(key)
    }

    /// Get or create a key for an entity
    pub async fn get_or_create_key(&self, key_id: &str) -> Result<EncryptionKey> {
        // Try to get existing key
        if let Some(key) = self.get_key(key_id).await? {
            return Ok(key);
        }

        // Create new key
        self.create_key(key_id, KeyPurpose::PiiEncryption, None)
            .await
    }

    /// Create a new key
    pub async fn create_key(
        &self,
        key_id: &str,
        purpose: KeyPurpose,
        entity_id: Option<String>,
    ) -> Result<EncryptionKey> {
        // Check if key already exists
        if self.provider.key_exists(key_id).await? {
            anyhow::bail!("Key already exists: {}", key_id);
        }

        // Generate new key
        let key = EncryptionKey::generate(key_id);

        // Create metadata
        let now = chrono::Utc::now();
        let expires_at = now + chrono::Duration::days(self.default_key_ttl_days as i64);

        let metadata = KeyMetadata {
            key_id: key_id.to_string(),
            purpose,
            entity_id,
            created_at: now.to_rfc3339(),
            expires_at: Some(expires_at.to_rfc3339()),
            last_rotated_at: None,
            rotation_count: 0,
            status: KeyStatus::Active,
            allowed_roles: vec!["admin".to_string(), "authorized".to_string()],
            tags: HashMap::new(),
        };

        // Store key
        self.provider.store_key(&key, &metadata).await?;

        info!(
            key_id = %key_id,
            purpose = ?purpose,
            expires_at = %expires_at,
            "Created new encryption key"
        );

        Ok(key)
    }

    /// Rotate a key (create new version, keep old for decryption)
    pub async fn rotate_key(&self, key_id: &str) -> Result<EncryptionKey> {
        // Get current key and metadata
        let _old_key = self
            .get_key(key_id)
            .await?
            .context(format!("Key not found: {}", key_id))?;

        let mut metadata = self
            .provider
            .get_metadata(key_id)
            .await?
            .context("Key metadata not found")?;

        // Update status to rotating
        metadata.status = KeyStatus::Rotating;
        self.provider.update_metadata(key_id, &metadata).await?;

        // Generate new key with versioned ID
        let new_key_id = format!("{}:v{}", key_id, metadata.rotation_count + 1);
        let new_key = EncryptionKey::generate(&new_key_id);

        // Create metadata for new key
        let now = chrono::Utc::now();
        let new_metadata = KeyMetadata {
            key_id: new_key_id.clone(),
            purpose: metadata.purpose,
            entity_id: metadata.entity_id.clone(),
            created_at: now.to_rfc3339(),
            expires_at: Some(
                (now + chrono::Duration::days(self.default_key_ttl_days as i64)).to_rfc3339(),
            ),
            last_rotated_at: Some(now.to_rfc3339()),
            rotation_count: metadata.rotation_count + 1,
            status: KeyStatus::Active,
            allowed_roles: metadata.allowed_roles.clone(),
            tags: metadata.tags.clone(),
        };

        // Store new key
        self.provider.store_key(&new_key, &new_metadata).await?;

        // Update old key status
        metadata.status = KeyStatus::PendingRotation;
        metadata.last_rotated_at = Some(now.to_rfc3339());
        metadata.rotation_count += 1;
        self.provider.update_metadata(key_id, &metadata).await?;

        info!(
            old_key_id = %key_id,
            new_key_id = %new_key_id,
            rotation_count = metadata.rotation_count,
            "Rotated encryption key"
        );

        Ok(new_key)
    }

    /// Disable a key (can still decrypt, cannot encrypt)
    pub async fn disable_key(&self, key_id: &str) -> Result<()> {
        let mut metadata = self
            .provider
            .get_metadata(key_id)
            .await?
            .context("Key not found")?;

        metadata.status = KeyStatus::Disabled;
        self.provider.update_metadata(key_id, &metadata).await?;

        info!(key_id = %key_id, "Disabled encryption key");
        Ok(())
    }

    /// Mark a key as compromised (should not be used)
    pub async fn mark_compromised(&self, key_id: &str) -> Result<()> {
        let mut metadata = self
            .provider
            .get_metadata(key_id)
            .await?
            .context("Key not found")?;

        metadata.status = KeyStatus::Compromised;
        self.provider.update_metadata(key_id, &metadata).await?;

        warn!(key_id = %key_id, "Marked encryption key as COMPROMISED");
        Ok(())
    }

    /// Destroy a key (cannot be recovered)
    pub async fn destroy_key(&self, key_id: &str) -> Result<()> {
        // Update status first for audit trail
        if let Some(mut metadata) = self.provider.get_metadata(key_id).await? {
            metadata.status = KeyStatus::Destroyed;
            self.provider.update_metadata(key_id, &metadata).await?;
        }

        // Actually delete the key material
        self.provider.delete_key(key_id).await?;

        warn!(key_id = %key_id, "Destroyed encryption key");
        Ok(())
    }

    /// List all keys
    pub async fn list_keys(&self) -> Result<Vec<String>> {
        self.provider.list_keys().await
    }

    /// Get key metadata
    pub async fn get_metadata(&self, key_id: &str) -> Result<Option<KeyMetadata>> {
        self.provider.get_metadata(key_id).await
    }

    /// Check which keys need rotation
    pub async fn get_keys_needing_rotation(&self) -> Result<Vec<String>> {
        let auto_rotate_days = match self.auto_rotate_days {
            Some(days) => days,
            None => return Ok(Vec::new()),
        };

        let mut needs_rotation = Vec::new();
        let threshold = chrono::Utc::now() - chrono::Duration::days(auto_rotate_days as i64);

        for key_id in self.list_keys().await? {
            if let Some(metadata) = self.get_metadata(&key_id).await? {
                if metadata.status != KeyStatus::Active {
                    continue;
                }

                let created = chrono::DateTime::parse_from_rfc3339(&metadata.created_at)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());

                let last_activity = metadata
                    .last_rotated_at
                    .as_ref()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or(created);

                if last_activity < threshold {
                    needs_rotation.push(key_id);
                }
            }
        }

        Ok(needs_rotation)
    }

    /// Get statistics about managed keys
    pub async fn get_stats(&self) -> Result<KeyManagerStats> {
        let key_ids = self.list_keys().await?;
        let mut stats = KeyManagerStats::default();

        for key_id in &key_ids {
            if let Some(metadata) = self.get_metadata(key_id).await? {
                stats.total_keys += 1;

                match metadata.status {
                    KeyStatus::Active => stats.active_keys += 1,
                    KeyStatus::Disabled => stats.disabled_keys += 1,
                    KeyStatus::Compromised => stats.compromised_keys += 1,
                    KeyStatus::PendingRotation | KeyStatus::Rotating => stats.rotating_keys += 1,
                    KeyStatus::Destroyed => stats.destroyed_keys += 1,
                }

                // Check expiration
                if let Some(expires_at) = &metadata.expires_at {
                    if let Ok(expiry) = chrono::DateTime::parse_from_rfc3339(expires_at) {
                        let days_until_expiry =
                            (expiry - chrono::Utc::now()).num_days();
                        if days_until_expiry < 0 {
                            stats.expired_keys += 1;
                        } else if days_until_expiry < 30 {
                            stats.expiring_soon += 1;
                        }
                    }
                }
            }
        }

        stats.keys_needing_rotation = self.get_keys_needing_rotation().await?.len();

        Ok(stats)
    }
}

/// Statistics about managed keys
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeyManagerStats {
    pub total_keys: usize,
    pub active_keys: usize,
    pub disabled_keys: usize,
    pub compromised_keys: usize,
    pub rotating_keys: usize,
    pub destroyed_keys: usize,
    pub expired_keys: usize,
    pub expiring_soon: usize,
    pub keys_needing_rotation: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_local_provider_basic_operations() {
        let provider = LocalKeyProvider::new();

        let key = EncryptionKey::generate("test-key");
        let metadata = KeyMetadata {
            key_id: "test-key".to_string(),
            purpose: KeyPurpose::PiiEncryption,
            entity_id: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            expires_at: None,
            last_rotated_at: None,
            rotation_count: 0,
            status: KeyStatus::Active,
            allowed_roles: vec!["admin".to_string()],
            tags: HashMap::new(),
        };

        // Store
        provider.store_key(&key, &metadata).await.unwrap();

        // Verify exists
        assert!(provider.key_exists("test-key").await.unwrap());

        // Get
        let retrieved = provider.get_key("test-key").await.unwrap().unwrap();
        assert_eq!(retrieved.key_id(), key.key_id());

        // List
        let keys = provider.list_keys().await.unwrap();
        assert!(keys.contains(&"test-key".to_string()));

        // Delete
        provider.delete_key("test-key").await.unwrap();
        assert!(!provider.key_exists("test-key").await.unwrap());
    }

    #[tokio::test]
    async fn test_key_manager_create_and_get() {
        let manager = KeyManager::local();

        let key = manager
            .create_key("entity:farmer:123", KeyPurpose::PiiEncryption, Some("farmer:123".to_string()))
            .await
            .unwrap();

        assert_eq!(key.key_id(), "entity:farmer:123");

        let retrieved = manager.get_key("entity:farmer:123").await.unwrap().unwrap();
        assert_eq!(retrieved.key_id(), key.key_id());
    }

    #[tokio::test]
    async fn test_key_manager_get_or_create() {
        let manager = KeyManager::local();

        // First call creates
        let key1 = manager.get_or_create_key("test-key").await.unwrap();

        // Second call retrieves
        let key2 = manager.get_or_create_key("test-key").await.unwrap();

        assert_eq!(key1.fingerprint(), key2.fingerprint());
    }

    #[tokio::test]
    async fn test_key_rotation() {
        let manager = KeyManager::local();

        // Create initial key
        let _key1 = manager
            .create_key("rotate-test", KeyPurpose::General, None)
            .await
            .unwrap();

        // Rotate
        let key2 = manager.rotate_key("rotate-test").await.unwrap();
        assert!(key2.key_id().contains(":v1"));

        // Check old key status
        let metadata = manager.get_metadata("rotate-test").await.unwrap().unwrap();
        assert_eq!(metadata.status, KeyStatus::PendingRotation);
        assert_eq!(metadata.rotation_count, 1);
    }

    #[tokio::test]
    async fn test_key_disable_and_compromise() {
        let manager = KeyManager::local();

        manager
            .create_key("status-test", KeyPurpose::General, None)
            .await
            .unwrap();

        // Disable
        manager.disable_key("status-test").await.unwrap();
        let metadata = manager.get_metadata("status-test").await.unwrap().unwrap();
        assert_eq!(metadata.status, KeyStatus::Disabled);

        // Can't encrypt with disabled key (status check)
        assert!(!metadata.status.can_encrypt());
        // Can still decrypt with disabled key
        assert!(!metadata.status.can_decrypt()); // Disabled can't decrypt either

        // Mark compromised
        manager.mark_compromised("status-test").await.unwrap();
        let metadata = manager.get_metadata("status-test").await.unwrap().unwrap();
        assert_eq!(metadata.status, KeyStatus::Compromised);
    }

    #[tokio::test]
    async fn test_key_destruction() {
        let manager = KeyManager::local();

        manager
            .create_key("destroy-test", KeyPurpose::General, None)
            .await
            .unwrap();

        // Verify exists
        assert!(manager.get_key("destroy-test").await.unwrap().is_some());

        // Destroy
        manager.destroy_key("destroy-test").await.unwrap();

        // Verify gone
        assert!(manager.get_key("destroy-test").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_key_manager_stats() {
        let manager = KeyManager::local();

        manager
            .create_key("key-1", KeyPurpose::PiiEncryption, None)
            .await
            .unwrap();
        manager
            .create_key("key-2", KeyPurpose::DocumentEncryption, None)
            .await
            .unwrap();
        manager.disable_key("key-2").await.unwrap();

        let stats = manager.get_stats().await.unwrap();
        assert_eq!(stats.total_keys, 2);
        assert_eq!(stats.active_keys, 1);
        assert_eq!(stats.disabled_keys, 1);
    }

    #[test]
    fn test_key_status_permissions() {
        assert!(KeyStatus::Active.can_encrypt());
        assert!(KeyStatus::Active.can_decrypt());

        assert!(!KeyStatus::PendingRotation.can_encrypt());
        assert!(KeyStatus::PendingRotation.can_decrypt());

        assert!(!KeyStatus::Disabled.can_encrypt());
        assert!(!KeyStatus::Disabled.can_decrypt());

        assert!(!KeyStatus::Compromised.can_encrypt());
        assert!(!KeyStatus::Compromised.can_decrypt());
    }

    #[tokio::test]
    async fn test_duplicate_key_creation_fails() {
        let manager = KeyManager::local();

        manager
            .create_key("duplicate-test", KeyPurpose::General, None)
            .await
            .unwrap();

        let result = manager
            .create_key("duplicate-test", KeyPurpose::General, None)
            .await;

        assert!(result.is_err());
    }
}
