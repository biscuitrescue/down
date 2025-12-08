//! Privacy and PII Protection Module
//!
//! This module provides:
//! - PII detection and classification
//! - Encryption/decryption of sensitive data before IPFS storage
//! - Key management integration (supports external KMS)
//! - Role-based access control for decryption
//! - Data separation (public vs private content)
//!
//! RULE: "No raw PII is ever stored in plaintext on IPFS."

pub mod encryption;
pub mod key_management;
pub mod pii_detector;

pub use encryption::{DataEncryptor, EncryptedEnvelope, EncryptionConfig};
pub use key_management::{KeyManager, KeyProvider, LocalKeyProvider};
pub use pii_detector::{PiiDetector, PiiField, PiiSeverity};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Privacy classification levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrivacyLevel {
    /// Fully public data, no encryption needed
    Public,
    /// Internal business data, may contain sensitive info
    Internal,
    /// Contains PII, must be encrypted
    Confidential,
    /// Highly restricted, additional access controls
    Restricted,
}

impl Default for PrivacyLevel {
    fn default() -> Self {
        Self::Public
    }
}

impl PrivacyLevel {
    /// Check if encryption is required for this level
    pub fn requires_encryption(&self) -> bool {
        matches!(self, Self::Confidential | Self::Restricted)
    }

    /// Get minimum required role for access
    pub fn minimum_access_role(&self) -> AccessRole {
        match self {
            Self::Public => AccessRole::Public,
            Self::Internal => AccessRole::Internal,
            Self::Confidential => AccessRole::Authorized,
            Self::Restricted => AccessRole::Admin,
        }
    }
}

/// Access roles for decryption authorization
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessRole {
    /// Public access - no authentication needed
    Public = 0,
    /// Internal staff
    Internal = 1,
    /// Authorized users (farmers, FPO staff, etc.)
    Authorized = 2,
    /// Regulators and auditors
    Regulator = 3,
    /// System administrators
    Admin = 4,
}

impl AccessRole {
    /// Check if this role can access a given privacy level
    pub fn can_access(&self, level: PrivacyLevel) -> bool {
        *self >= level.minimum_access_role()
    }
}

/// Result of privacy processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyProcessingResult {
    /// Public data safe for IPFS storage
    pub public_data: serde_json::Value,
    /// Encrypted private data (if any)
    pub encrypted_data: Option<EncryptedEnvelope>,
    /// IPFS CID of encrypted data (after upload)
    pub encrypted_data_cid: Option<String>,
    /// List of fields that were encrypted
    pub encrypted_fields: Vec<String>,
    /// Privacy classification
    pub classification: PrivacyClassification,
}

/// Privacy classification metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyClassification {
    /// Overall privacy level
    pub level: PrivacyLevel,
    /// Whether data is encrypted
    pub is_encrypted: bool,
    /// Key ID used for encryption
    pub encryption_key_id: Option<String>,
    /// Algorithm used
    pub encryption_algorithm: Option<String>,
    /// List of PII field paths that were detected
    pub pii_fields: Vec<String>,
    /// Roles that can access the decrypted data
    pub access_roles: Vec<AccessRole>,
}

impl Default for PrivacyClassification {
    fn default() -> Self {
        Self {
            level: PrivacyLevel::Public,
            is_encrypted: false,
            encryption_key_id: None,
            encryption_algorithm: None,
            pii_fields: Vec::new(),
            access_roles: vec![AccessRole::Public],
        }
    }
}

/// Privacy processor that handles PII detection and encryption
#[derive(Clone)]
pub struct PrivacyProcessor {
    detector: PiiDetector,
    encryptor: DataEncryptor,
    key_manager: KeyManager,
    config: PrivacyConfig,
}

/// Configuration for privacy processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyConfig {
    /// Whether to enforce encryption for PII
    pub enforce_encryption: bool,
    /// Whether to allow any PII at all (strict mode)
    pub allow_pii: bool,
    /// Default encryption algorithm
    pub default_algorithm: String,
    /// Fields that should always be encrypted (by path pattern)
    pub always_encrypt_patterns: Vec<String>,
    /// Fields that should never be encrypted (by path pattern)
    pub never_encrypt_patterns: Vec<String>,
    /// Maximum plaintext PII fields allowed (0 = none)
    pub max_plaintext_pii: usize,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            enforce_encryption: true,
            allow_pii: true,
            default_algorithm: "AES-256-GCM".to_string(),
            always_encrypt_patterns: vec![
                "*.name".to_string(),
                "*.phone".to_string(),
                "*.email".to_string(),
                "*.address".to_string(),
                "*.aadhaar*".to_string(),
                "*.pan*".to_string(),
                "*.bank*".to_string(),
                "profile.*".to_string(),
                "contact_info.*".to_string(),
            ],
            never_encrypt_patterns: vec![
                "batch_id".to_string(),
                "sku_id".to_string(),
                "farmer_id".to_string(),
                "geo_hash".to_string(),
            ],
            max_plaintext_pii: 0,
        }
    }
}

impl PrivacyProcessor {
    /// Create a new privacy processor
    pub fn new(key_manager: KeyManager) -> Self {
        Self {
            detector: PiiDetector::new(),
            encryptor: DataEncryptor::new(EncryptionConfig::default()),
            key_manager,
            config: PrivacyConfig::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(key_manager: KeyManager, config: PrivacyConfig) -> Self {
        Self {
            detector: PiiDetector::new(),
            encryptor: DataEncryptor::new(EncryptionConfig::default()),
            key_manager,
            config,
        }
    }

    /// Process a document for privacy-safe storage
    ///
    /// This is the main entry point. It:
    /// 1. Detects PII fields
    /// 2. Separates public and private data
    /// 3. Encrypts private data
    /// 4. Returns the processed result
    pub async fn process_for_storage(
        &self,
        document: &serde_json::Value,
        entity_id: &str,
    ) -> anyhow::Result<PrivacyProcessingResult> {
        // 1. Detect PII
        let pii_fields = self.detector.detect_pii(document);

        // 2. Determine privacy level
        let level = self.determine_privacy_level(&pii_fields);

        // 3. If no PII or encryption not required, return as-is
        if pii_fields.is_empty() || !level.requires_encryption() {
            return Ok(PrivacyProcessingResult {
                public_data: document.clone(),
                encrypted_data: None,
                encrypted_data_cid: None,
                encrypted_fields: Vec::new(),
                classification: PrivacyClassification {
                    level,
                    is_encrypted: false,
                    encryption_key_id: None,
                    encryption_algorithm: None,
                    pii_fields: pii_fields.iter().map(|f| f.path.clone()).collect(),
                    access_roles: vec![level.minimum_access_role()],
                },
            });
        }

        // 4. Check if encryption is enforced
        if self.config.enforce_encryption && !self.config.allow_pii {
            let pii_paths: Vec<_> = pii_fields.iter().map(|f| f.path.as_str()).collect();
            anyhow::bail!(
                "PII detected but not allowed in strict mode. Fields: {:?}",
                pii_paths
            );
        }

        // 5. Separate public and private data
        let (public_data, private_data) = self.separate_data(document, &pii_fields)?;

        // 6. Get encryption key
        let key_id = format!("entity:{}", entity_id);
        let key = self.key_manager.get_or_create_key(&key_id).await?;

        // 7. Encrypt private data
        let encrypted = self
            .encryptor
            .encrypt(&serde_json::to_vec(&private_data)?, &key)?;

        let encrypted_field_paths: Vec<String> =
            pii_fields.iter().map(|f| f.path.clone()).collect();

        Ok(PrivacyProcessingResult {
            public_data,
            encrypted_data: Some(encrypted.clone()),
            encrypted_data_cid: None, // Will be set after IPFS upload
            encrypted_fields: encrypted_field_paths.clone(),
            classification: PrivacyClassification {
                level,
                is_encrypted: true,
                encryption_key_id: Some(key_id),
                encryption_algorithm: Some(self.config.default_algorithm.clone()),
                pii_fields: encrypted_field_paths,
                access_roles: vec![AccessRole::Authorized, AccessRole::Regulator, AccessRole::Admin],
            },
        })
    }

    /// Decrypt private data for authorized access
    pub async fn decrypt_for_access(
        &self,
        encrypted: &EncryptedEnvelope,
        key_id: &str,
        accessor_role: AccessRole,
        required_level: PrivacyLevel,
    ) -> anyhow::Result<serde_json::Value> {
        // Check access authorization
        if !accessor_role.can_access(required_level) {
            anyhow::bail!(
                "Access denied: role {:?} cannot access {:?} data",
                accessor_role,
                required_level
            );
        }

        // Get decryption key
        let key = self
            .key_manager
            .get_key(key_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Decryption key not found: {}", key_id))?;

        // Decrypt
        let plaintext = self.encryptor.decrypt(encrypted, &key)?;

        // Parse JSON
        let data: serde_json::Value = serde_json::from_slice(&plaintext)?;

        Ok(data)
    }

    /// Merge public and decrypted private data back together
    pub fn merge_data(
        &self,
        public_data: &serde_json::Value,
        private_data: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let mut merged = public_data.clone();

        if let (serde_json::Value::Object(ref mut merged_map), serde_json::Value::Object(private_map)) =
            (&mut merged, private_data)
        {
            for (key, value) in private_map {
                merged_map.insert(key.clone(), value.clone());
            }
        }

        Ok(merged)
    }

    /// Determine privacy level based on detected PII
    fn determine_privacy_level(&self, pii_fields: &[PiiField]) -> PrivacyLevel {
        if pii_fields.is_empty() {
            return PrivacyLevel::Public;
        }

        // Check for high-severity PII
        let has_high_severity = pii_fields
            .iter()
            .any(|f| matches!(f.severity, PiiSeverity::High | PiiSeverity::Critical));

        if has_high_severity {
            PrivacyLevel::Restricted
        } else {
            PrivacyLevel::Confidential
        }
    }

    /// Separate document into public and private parts
    fn separate_data(
        &self,
        document: &serde_json::Value,
        pii_fields: &[PiiField],
    ) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
        let mut public = document.clone();
        let mut private = serde_json::json!({});

        // Extract PII fields from public data and move to private
        for field in pii_fields {
            if let Some(value) = extract_and_remove_field(&mut public, &field.path) {
                set_nested_field(&mut private, &field.path, value);

                // Replace with reference in public data
                let reference = serde_json::json!({
                    "_encrypted": true,
                    "_field": field.path.clone(),
                    "_type": format!("{:?}", field.pii_type)
                });
                set_nested_field(&mut public, &field.path, reference);
            }
        }

        Ok((public, private))
    }

    /// Check if a document contains any plaintext PII
    pub fn contains_plaintext_pii(&self, document: &serde_json::Value) -> bool {
        !self.detector.detect_pii(document).is_empty()
    }

    /// Redact PII fields (replace with "[REDACTED]")
    pub fn redact_pii(&self, document: &serde_json::Value) -> serde_json::Value {
        let pii_fields = self.detector.detect_pii(document);
        let mut redacted = document.clone();

        for field in pii_fields {
            set_nested_field(&mut redacted, &field.path, serde_json::json!("[REDACTED]"));
        }

        redacted
    }
}

/// Extract a field from a JSON value and remove it, returning the value
fn extract_and_remove_field(
    document: &mut serde_json::Value,
    path: &str,
) -> Option<serde_json::Value> {
    let parts: Vec<&str> = path.split('.').collect();
    extract_nested(&mut *document, &parts)
}

fn extract_nested(value: &mut serde_json::Value, parts: &[&str]) -> Option<serde_json::Value> {
    if parts.is_empty() {
        return None;
    }

    if parts.len() == 1 {
        if let serde_json::Value::Object(map) = value {
            return map.remove(parts[0]);
        }
        return None;
    }

    if let serde_json::Value::Object(map) = value {
        if let Some(child) = map.get_mut(parts[0]) {
            return extract_nested(child, &parts[1..]);
        }
    }

    None
}

/// Set a nested field in a JSON value
fn set_nested_field(document: &mut serde_json::Value, path: &str, value: serde_json::Value) {
    let parts: Vec<&str> = path.split('.').collect();
    set_nested(&mut *document, &parts, value);
}

fn set_nested(doc: &mut serde_json::Value, parts: &[&str], value: serde_json::Value) {
    if parts.is_empty() {
        return;
    }

    if parts.len() == 1 {
        if let serde_json::Value::Object(map) = doc {
            map.insert(parts[0].to_string(), value);
        }
        return;
    }

    if let serde_json::Value::Object(map) = doc {
        let child = map
            .entry(parts[0].to_string())
            .or_insert_with(|| serde_json::json!({}));
        set_nested(child, &parts[1..], value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_privacy_level_encryption_requirement() {
        assert!(!PrivacyLevel::Public.requires_encryption());
        assert!(!PrivacyLevel::Internal.requires_encryption());
        assert!(PrivacyLevel::Confidential.requires_encryption());
        assert!(PrivacyLevel::Restricted.requires_encryption());
    }

    #[test]
    fn test_access_role_authorization() {
        assert!(AccessRole::Public.can_access(PrivacyLevel::Public));
        assert!(!AccessRole::Public.can_access(PrivacyLevel::Confidential));

        assert!(AccessRole::Admin.can_access(PrivacyLevel::Public));
        assert!(AccessRole::Admin.can_access(PrivacyLevel::Restricted));

        assert!(AccessRole::Authorized.can_access(PrivacyLevel::Confidential));
        assert!(!AccessRole::Authorized.can_access(PrivacyLevel::Restricted));
    }

    #[test]
    fn test_extract_and_remove_field() {
        let mut doc = serde_json::json!({
            "profile": {
                "name": "John Doe",
                "age": 30
            },
            "id": "123"
        });

        let extracted = extract_and_remove_field(&mut doc, "profile.name");
        assert_eq!(extracted, Some(serde_json::json!("John Doe")));

        // Field should be removed
        assert!(doc.get("profile").unwrap().get("name").is_none());
        // Other fields should remain
        assert_eq!(doc.get("profile").unwrap().get("age"), Some(&serde_json::json!(30)));
    }

    #[test]
    fn test_set_nested_field() {
        let mut doc = serde_json::json!({});

        set_nested_field(&mut doc, "profile.name", serde_json::json!("Jane Doe"));

        assert_eq!(
            doc.get("profile").unwrap().get("name"),
            Some(&serde_json::json!("Jane Doe"))
        );
    }

    #[test]
    fn test_default_privacy_config() {
        let config = PrivacyConfig::default();
        assert!(config.enforce_encryption);
        assert!(config.allow_pii);
        assert_eq!(config.max_plaintext_pii, 0);
        assert!(!config.always_encrypt_patterns.is_empty());
    }
}
