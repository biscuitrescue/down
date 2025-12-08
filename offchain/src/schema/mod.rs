//! Schema Versioning and Validation System
//!
//! This module provides:
//! - Schema definitions stored on IPFS with version control
//! - Strict validation before any IPFS upload
//! - Forward-compatible schema evolution (new optional fields don't break consumers)
//! - Schema registry with CID references

pub mod definitions;
pub mod registry;
pub mod validator;

pub use definitions::*;
pub use registry::SchemaRegistry;
pub use validator::SchemaValidator;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Schema reference embedded in every IPFS document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaReference {
    /// Schema name (e.g., "farmer_registration", "fpo_purchase")
    pub name: String,
    /// Semantic version (e.g., "1.0.0", "1.1.0")
    pub version: String,
    /// IPFS CID of the schema definition itself
    pub cid: Option<String>,
}

impl SchemaReference {
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            cid: None,
        }
    }

    pub fn with_cid(name: &str, version: &str, cid: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            cid: Some(cid.to_string()),
        }
    }

    /// Check if this version is compatible with a required version
    /// Uses semantic versioning rules: major version must match, minor can be >=
    pub fn is_compatible_with(&self, required: &SchemaReference) -> bool {
        if self.name != required.name {
            return false;
        }

        let self_parts: Vec<u32> = self
            .version
            .split('.')
            .filter_map(|s| s.parse().ok())
            .collect();
        let required_parts: Vec<u32> = required
            .version
            .split('.')
            .filter_map(|s| s.parse().ok())
            .collect();

        if self_parts.is_empty() || required_parts.is_empty() {
            return false;
        }

        // Major version must match
        if self_parts[0] != required_parts[0] {
            return false;
        }

        // Minor version must be >= required
        let self_minor = self_parts.get(1).copied().unwrap_or(0);
        let required_minor = required_parts.get(1).copied().unwrap_or(0);

        self_minor >= required_minor
    }
}

/// Schema definition that gets stored on IPFS
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaDefinition {
    /// Schema metadata
    pub schema: SchemaReference,
    /// Human-readable description
    pub description: String,
    /// JSON Schema for validation (stored as Value for flexibility)
    pub json_schema: serde_json::Value,
    /// Required fields that must always be present
    pub required_fields: Vec<String>,
    /// Optional fields that may be added in future versions
    pub optional_fields: Vec<String>,
    /// Fields that contain PII and must be encrypted
    pub pii_fields: Vec<String>,
    /// Timestamp of schema creation
    pub created_at: String,
    /// Previous version's CID for schema lineage tracking
    pub previous_version_cid: Option<String>,
    /// Changelog from previous version
    pub changelog: Option<String>,
}

/// Validation result with detailed error information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub schema_ref: SchemaReference,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
    pub pii_detected: Vec<String>,
}

impl ValidationResult {
    pub fn success(schema_ref: SchemaReference) -> Self {
        Self {
            is_valid: true,
            schema_ref,
            errors: vec![],
            warnings: vec![],
            pii_detected: vec![],
        }
    }

    pub fn failure(schema_ref: SchemaReference, errors: Vec<ValidationError>) -> Self {
        Self {
            is_valid: false,
            schema_ref,
            errors,
            warnings: vec![],
            pii_detected: vec![],
        }
    }

    pub fn with_warnings(mut self, warnings: Vec<ValidationWarning>) -> Self {
        self.warnings = warnings;
        self
    }

    pub fn with_pii(mut self, pii_fields: Vec<String>) -> Self {
        self.pii_detected = pii_fields;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub field: String,
    pub error_type: ValidationErrorType,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationErrorType {
    MissingRequiredField,
    InvalidType,
    InvalidFormat,
    OutOfRange,
    UnknownSchema,
    IncompatibleVersion,
    SchemaNotFound,
    PlaintextPiiDetected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationWarning {
    pub field: String,
    pub warning_type: ValidationWarningType,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationWarningType {
    UnknownField,
    DeprecatedField,
    LargePayload,
    MissingOptionalField,
}

/// Supply chain stages with their schema names
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupplyChainStage {
    FarmerRegistration,
    FpoPurchase,
    WarehouseStorage,
    Logistics,
    Processing,
    Packaging,
    AiScoring,
    FraudReport,
}

impl SupplyChainStage {
    pub fn schema_name(&self) -> &'static str {
        match self {
            Self::FarmerRegistration => "farmer_registration",
            Self::FpoPurchase => "fpo_purchase",
            Self::WarehouseStorage => "warehouse_storage",
            Self::Logistics => "logistics",
            Self::Processing => "processing",
            Self::Packaging => "packaging",
            Self::AiScoring => "ai_scoring",
            Self::FraudReport => "fraud_report",
        }
    }

    pub fn current_version(&self) -> &'static str {
        // All schemas start at 1.0.0
        "1.0.0"
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "farmer_registration" => Some(Self::FarmerRegistration),
            "fpo_purchase" => Some(Self::FpoPurchase),
            "warehouse_storage" => Some(Self::WarehouseStorage),
            "logistics" => Some(Self::Logistics),
            "processing" => Some(Self::Processing),
            "packaging" => Some(Self::Packaging),
            "ai_scoring" => Some(Self::AiScoring),
            "fraud_report" => Some(Self::FraudReport),
            _ => None,
        }
    }
}

/// Document wrapper that includes schema reference and validation metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionedDocument<T> {
    /// Schema reference for this document
    pub schema: SchemaReference,
    /// Document metadata
    pub metadata: DocumentMetadata,
    /// The actual content
    pub content: T,
    /// Privacy classification
    pub privacy: PrivacyClassification,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    /// Unique document ID (UUID)
    pub document_id: String,
    /// Creation timestamp (RFC 3339)
    pub created_at: String,
    /// Creator identifier (wallet address or system ID)
    pub created_by: String,
    /// Content hash for integrity verification
    pub content_hash: String,
    /// Size in bytes
    pub size_bytes: usize,
    /// Upload source identifier
    pub upload_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyClassification {
    /// Overall classification level
    pub level: PrivacyLevel,
    /// Whether this document contains encrypted data
    pub is_encrypted: bool,
    /// Key ID used for encryption (if encrypted)
    pub encryption_key_id: Option<String>,
    /// List of field paths that are PII
    pub pii_field_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrivacyLevel {
    /// Fully public, no PII
    Public,
    /// Contains some sensitive but non-PII data
    Internal,
    /// Contains PII, must be encrypted
    Confidential,
    /// Highly sensitive, restricted access
    Restricted,
}

impl Default for PrivacyClassification {
    fn default() -> Self {
        Self {
            level: PrivacyLevel::Public,
            is_encrypted: false,
            encryption_key_id: None,
            pii_field_paths: vec![],
        }
    }
}

impl<T: Serialize> VersionedDocument<T> {
    pub fn new(
        stage: SupplyChainStage,
        content: T,
        created_by: &str,
        upload_source: &str,
    ) -> anyhow::Result<Self> {
        let content_json = serde_json::to_string(&content)?;
        let content_hash = format!("{:x}", md5::compute(content_json.as_bytes()));

        Ok(Self {
            schema: SchemaReference::new(stage.schema_name(), stage.current_version()),
            metadata: DocumentMetadata {
                document_id: uuid::Uuid::new_v4().to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                created_by: created_by.to_string(),
                content_hash,
                size_bytes: content_json.len(),
                upload_source: upload_source.to_string(),
            },
            content,
            privacy: PrivacyClassification::default(),
        })
    }

    pub fn with_privacy(mut self, privacy: PrivacyClassification) -> Self {
        self.privacy = privacy;
        self
    }

    pub fn with_schema_cid(mut self, cid: &str) -> Self {
        self.schema.cid = Some(cid.to_string());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_compatibility() {
        let v1_0 = SchemaReference::new("farmer_registration", "1.0.0");
        let v1_1 = SchemaReference::new("farmer_registration", "1.1.0");
        let v2_0 = SchemaReference::new("farmer_registration", "2.0.0");
        let other = SchemaReference::new("fpo_purchase", "1.0.0");

        // Same version is compatible
        assert!(v1_0.is_compatible_with(&v1_0));

        // Higher minor version is compatible with lower
        assert!(v1_1.is_compatible_with(&v1_0));

        // Lower minor version is not compatible with higher requirement
        assert!(!v1_0.is_compatible_with(&v1_1));

        // Different major versions are not compatible
        assert!(!v2_0.is_compatible_with(&v1_0));
        assert!(!v1_0.is_compatible_with(&v2_0));

        // Different schemas are not compatible
        assert!(!v1_0.is_compatible_with(&other));
    }

    #[test]
    fn test_supply_chain_stages() {
        assert_eq!(
            SupplyChainStage::FarmerRegistration.schema_name(),
            "farmer_registration"
        );
        assert_eq!(
            SupplyChainStage::from_name("farmer_registration"),
            Some(SupplyChainStage::FarmerRegistration)
        );
        assert_eq!(SupplyChainStage::from_name("unknown"), None);
    }
}
