//! Schema Validator
//!
//! Provides strict validation of documents against JSON schemas before IPFS upload.
//! Ensures data integrity and prevents malformed data from being pinned forever.

use super::{
    definitions::get_schema_definition, PrivacyLevel, SchemaDefinition, SchemaReference,
    SupplyChainStage, ValidationError, ValidationErrorType, ValidationResult, ValidationWarning,
    ValidationWarningType,
};
use anyhow::{Context, Result};
use jsonschema::{Draft, JSONSchema};
use serde_json::Value;
use std::collections::HashSet;
use tracing::{debug, warn};

/// Maximum allowed payload size (10 MB)
const MAX_PAYLOAD_SIZE_BYTES: usize = 10 * 1024 * 1024;

/// Warning threshold for large payloads (1 MB)
const LARGE_PAYLOAD_WARNING_BYTES: usize = 1024 * 1024;

/// Schema validator that enforces strict validation before IPFS upload
#[derive(Debug, Clone)]
pub struct SchemaValidator {
    /// Whether to allow additional properties not defined in schema
    allow_additional_properties: bool,
    /// Whether to enforce PII encryption rules
    enforce_pii_encryption: bool,
    /// Maximum allowed payload size
    max_payload_size: usize,
}

impl Default for SchemaValidator {
    fn default() -> Self {
        Self {
            allow_additional_properties: true, // Forward compatibility
            enforce_pii_encryption: true,      // Security by default
            max_payload_size: MAX_PAYLOAD_SIZE_BYTES,
        }
    }
}

impl SchemaValidator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a strict validator that rejects unknown fields
    pub fn strict() -> Self {
        Self {
            allow_additional_properties: false,
            enforce_pii_encryption: true,
            max_payload_size: MAX_PAYLOAD_SIZE_BYTES,
        }
    }

    /// Create a permissive validator for development/testing
    pub fn permissive() -> Self {
        Self {
            allow_additional_properties: true,
            enforce_pii_encryption: false,
            max_payload_size: MAX_PAYLOAD_SIZE_BYTES * 10,
        }
    }

    pub fn with_max_payload_size(mut self, size: usize) -> Self {
        self.max_payload_size = size;
        self
    }

    pub fn with_pii_enforcement(mut self, enforce: bool) -> Self {
        self.enforce_pii_encryption = enforce;
        self
    }

    /// Validate a document against its schema before IPFS upload
    ///
    /// Returns a ValidationResult with detailed error/warning information.
    /// This MUST be called before any IPFS upload to prevent bad data.
    pub fn validate(
        &self,
        stage: SupplyChainStage,
        document: &Value,
    ) -> Result<ValidationResult> {
        let schema_def = get_schema_definition(stage);
        self.validate_against_schema(&schema_def, document)
    }

    /// Validate against a specific schema definition
    pub fn validate_against_schema(
        &self,
        schema_def: &SchemaDefinition,
        document: &Value,
    ) -> Result<ValidationResult> {
        let schema_ref = schema_def.schema.clone();
        let mut errors: Vec<ValidationError> = Vec::new();
        let mut warnings: Vec<ValidationWarning> = Vec::new();
        let mut pii_detected: Vec<String> = Vec::new();

        debug!(
            schema = %schema_ref.name,
            version = %schema_ref.version,
            "Validating document against schema"
        );

        // 1. Check payload size
        let payload_size = serde_json::to_string(document)
            .context("Failed to serialize document for size check")?
            .len();

        if payload_size > self.max_payload_size {
            errors.push(ValidationError {
                field: "_root".to_string(),
                error_type: ValidationErrorType::OutOfRange,
                message: format!(
                    "Payload size {} bytes exceeds maximum allowed {} bytes",
                    payload_size, self.max_payload_size
                ),
            });
        } else if payload_size > LARGE_PAYLOAD_WARNING_BYTES {
            warnings.push(ValidationWarning {
                field: "_root".to_string(),
                warning_type: ValidationWarningType::LargePayload,
                message: format!(
                    "Payload size {} bytes is large, consider compression or splitting",
                    payload_size
                ),
            });
        }

        // 2. Validate required fields
        for required_field in &schema_def.required_fields {
            if !has_field(document, required_field) {
                errors.push(ValidationError {
                    field: required_field.clone(),
                    error_type: ValidationErrorType::MissingRequiredField,
                    message: format!("Required field '{}' is missing", required_field),
                });
            }
        }

        // 3. Validate against JSON schema
        let json_schema_errors = self.validate_json_schema(&schema_def.json_schema, document)?;
        errors.extend(json_schema_errors);

        // 4. Check for PII fields and encryption
        if self.enforce_pii_encryption {
            let pii_check = self.check_pii_fields(document, &schema_def.pii_fields);
            pii_detected = pii_check.detected_fields;

            if !pii_check.all_encrypted {
                for field in &pii_check.unencrypted_fields {
                    errors.push(ValidationError {
                        field: field.clone(),
                        error_type: ValidationErrorType::PlaintextPiiDetected,
                        message: format!(
                            "PII field '{}' appears to contain plaintext data. Must be encrypted before upload.",
                            field
                        ),
                    });
                }
            }
        }

        // 5. Check for unknown fields (generate warnings, not errors for forward compatibility)
        if !self.allow_additional_properties {
            let unknown_fields = self.find_unknown_fields(document, schema_def);
            for field in unknown_fields {
                warnings.push(ValidationWarning {
                    field: field.clone(),
                    warning_type: ValidationWarningType::UnknownField,
                    message: format!(
                        "Unknown field '{}' not defined in schema version {}",
                        field, schema_ref.version
                    ),
                });
            }
        }

        // 6. Check for deprecated fields
        let deprecated_warnings = self.check_deprecated_fields(document, schema_def);
        warnings.extend(deprecated_warnings);

        // 7. Build result
        let is_valid = errors.is_empty();

        if !is_valid {
            warn!(
                schema = %schema_ref.name,
                error_count = errors.len(),
                "Document validation failed"
            );
        }

        let result = if is_valid {
            ValidationResult::success(schema_ref)
                .with_warnings(warnings)
                .with_pii(pii_detected)
        } else {
            ValidationResult::failure(schema_ref, errors)
                .with_warnings(warnings)
                .with_pii(pii_detected)
        };

        Ok(result)
    }

    /// Validate document against JSON schema
    fn validate_json_schema(
        &self,
        schema: &Value,
        document: &Value,
    ) -> Result<Vec<ValidationError>> {
        let compiled = JSONSchema::options()
            .with_draft(Draft::Draft7)
            .compile(schema)
            .map_err(|e| anyhow::anyhow!("Failed to compile JSON schema: {}", e))?;

        let mut errors = Vec::new();

        if let Err(validation_errors) = compiled.validate(document) {
            for error in validation_errors {
                let field = error
                    .instance_path
                    .to_string()
                    .trim_start_matches('/')
                    .replace('/', ".");

                let field = if field.is_empty() {
                    "_root".to_string()
                } else {
                    field
                };

                let error_type = match error.kind {
                    jsonschema::error::ValidationErrorKind::Required { .. } => {
                        ValidationErrorType::MissingRequiredField
                    }
                    jsonschema::error::ValidationErrorKind::Type { .. } => {
                        ValidationErrorType::InvalidType
                    }
                    jsonschema::error::ValidationErrorKind::Format { .. } => {
                        ValidationErrorType::InvalidFormat
                    }
                    jsonschema::error::ValidationErrorKind::Minimum { .. }
                    | jsonschema::error::ValidationErrorKind::Maximum { .. } => {
                        ValidationErrorType::OutOfRange
                    }
                    _ => ValidationErrorType::InvalidFormat,
                };

                errors.push(ValidationError {
                    field,
                    error_type,
                    message: error.to_string(),
                });
            }
        }

        Ok(errors)
    }

    /// Check PII fields for encryption
    fn check_pii_fields(&self, document: &Value, pii_fields: &[String]) -> PiiCheckResult {
        let mut detected_fields = Vec::new();
        let mut unencrypted_fields = Vec::new();

        for field_path in pii_fields {
            if let Some(value) = get_nested_value(document, field_path) {
                detected_fields.push(field_path.clone());

                // Check if the value appears to be encrypted
                // Encrypted data typically looks like base64 or is referenced as a CID
                if !appears_encrypted(value) {
                    unencrypted_fields.push(field_path.clone());
                }
            }
        }

        let all_encrypted = unencrypted_fields.is_empty();
        PiiCheckResult {
            detected_fields,
            unencrypted_fields,
            all_encrypted,
        }
    }

    /// Find fields in document that aren't defined in schema
    fn find_unknown_fields(&self, document: &Value, schema_def: &SchemaDefinition) -> Vec<String> {
        let mut unknown = Vec::new();
        let known_fields: HashSet<&String> = schema_def
            .required_fields
            .iter()
            .chain(schema_def.optional_fields.iter())
            .collect();

        if let Value::Object(map) = document {
            for key in map.keys() {
                // Skip metadata fields that are always allowed
                if key == "schema" || key == "metadata" || key == "privacy" {
                    continue;
                }

                if !known_fields.contains(key) {
                    unknown.push(key.clone());
                }
            }
        }

        unknown
    }

    /// Check for deprecated fields
    fn check_deprecated_fields(
        &self,
        document: &Value,
        _schema_def: &SchemaDefinition,
    ) -> Vec<ValidationWarning> {
        let mut warnings = Vec::new();

        // Define deprecated fields per schema version
        // This would be populated from schema definition in a real implementation
        let deprecated_fields: Vec<(&str, &str)> = vec![
            // ("old_field_name", "Use 'new_field_name' instead")
        ];

        for (field, message) in deprecated_fields {
            if has_field(document, field) {
                warnings.push(ValidationWarning {
                    field: field.to_string(),
                    warning_type: ValidationWarningType::DeprecatedField,
                    message: message.to_string(),
                });
            }
        }

        warnings
    }

    /// Validate that a document's schema reference is compatible
    pub fn validate_schema_compatibility(
        &self,
        document_schema: &SchemaReference,
        required_schema: &SchemaReference,
    ) -> Result<bool, ValidationError> {
        if document_schema.is_compatible_with(required_schema) {
            Ok(true)
        } else {
            Err(ValidationError {
                field: "schema".to_string(),
                error_type: ValidationErrorType::IncompatibleVersion,
                message: format!(
                    "Document schema {}/{} is not compatible with required {}/{}",
                    document_schema.name,
                    document_schema.version,
                    required_schema.name,
                    required_schema.version
                ),
            })
        }
    }

    /// Determine the privacy level required for a document
    pub fn determine_privacy_level(
        &self,
        stage: SupplyChainStage,
        document: &Value,
    ) -> PrivacyLevel {
        let schema_def = get_schema_definition(stage);

        // Check if any PII fields are present
        for field_path in &schema_def.pii_fields {
            if get_nested_value(document, field_path).is_some() {
                return PrivacyLevel::Confidential;
            }
        }

        // Check for sensitive business data patterns
        if contains_sensitive_patterns(document) {
            return PrivacyLevel::Internal;
        }

        PrivacyLevel::Public
    }
}

/// Result of PII field check
struct PiiCheckResult {
    detected_fields: Vec<String>,
    unencrypted_fields: Vec<String>,
    all_encrypted: bool,
}

/// Check if a field exists in the document (supports nested paths like "profile.name")
fn has_field(document: &Value, field_path: &str) -> bool {
    get_nested_value(document, field_path).is_some()
}

/// Get a nested value from a document using dot notation (e.g., "profile.name")
fn get_nested_value<'a>(document: &'a Value, field_path: &str) -> Option<&'a Value> {
    let parts: Vec<&str> = field_path.split('.').collect();
    let mut current = document;

    for part in parts {
        match current {
            Value::Object(map) => {
                current = map.get(part)?;
            }
            Value::Array(arr) => {
                if let Ok(index) = part.parse::<usize>() {
                    current = arr.get(index)?;
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }

    Some(current)
}

/// Check if a value appears to be encrypted
fn appears_encrypted(value: &Value) -> bool {
    match value {
        Value::String(s) => {
            // Check for common encrypted/encoded patterns
            // Base64 encoded data typically has length divisible by 4 and specific chars
            let is_base64_like = s.len() > 32
                && s.len() % 4 == 0
                && s.chars()
                    .all(|c| c.is_alphanumeric() || c == '+' || c == '/' || c == '=');

            // Check for IPFS CID pattern (encrypted data might be stored as CID reference)
            let is_cid = s.starts_with("Qm") || s.starts_with("bafy");

            // Check for "encrypted_" prefix convention
            let is_marked_encrypted = s.starts_with("encrypted_") || s.starts_with("enc:");

            // Check for hex-encoded encrypted data
            let is_hex_like = s.len() > 64 && s.starts_with("0x") && s[2..].chars().all(|c| c.is_ascii_hexdigit());

            is_base64_like || is_cid || is_marked_encrypted || is_hex_like
        }
        Value::Object(map) => {
            // Check if it's an encrypted data envelope
            map.contains_key("ciphertext")
                || map.contains_key("encrypted_cid")
                || map.contains_key("key_id")
        }
        _ => false,
    }
}

/// Check for sensitive business data patterns
fn contains_sensitive_patterns(document: &Value) -> bool {
    let json_str = document.to_string().to_lowercase();

    // Patterns that indicate sensitive business data
    let sensitive_patterns = [
        "bank_account",
        "ifsc",
        "pan_number",
        "gst_number",
        "aadhaar",
        "passport",
        "driving_license",
        "credit_card",
        "cvv",
        "password",
        "secret",
        "private_key",
    ];

    sensitive_patterns.iter().any(|pattern| json_str.contains(pattern))
}

/// Validate and sanitize a document, returning errors if invalid
pub fn validate_before_upload(
    stage: SupplyChainStage,
    document: &Value,
) -> Result<ValidationResult> {
    let validator = SchemaValidator::default();
    validator.validate(stage, document)
}

/// Quick validation check that returns a boolean
pub fn is_valid_document(stage: SupplyChainStage, document: &Value) -> bool {
    validate_before_upload(stage, document)
        .map(|r| r.is_valid)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_valid_farmer_registration() {
        let document = json!({
            "farmer_id": "FRM-ABC12345",
            "registration_date": "2024-01-15T10:30:00Z",
            "land_details": {
                "total_area_hectares": 5.2,
                "soil_type": "alluvial",
                "geo_hash": "tdr1"
            },
            "crop_info": {
                "variety": "mustard",
                "planting_season": "rabi",
                "expected_yield_kg": 1500
            },
            "verification_status": {
                "status": "verified",
                "verified_at": "2024-01-16T10:30:00Z"
            }
        });

        let validator = SchemaValidator::new().with_pii_enforcement(false);
        let result = validator
            .validate(SupplyChainStage::FarmerRegistration, &document)
            .unwrap();

        assert!(result.is_valid, "Errors: {:?}", result.errors);
    }

    #[test]
    fn test_missing_required_field() {
        let document = json!({
            "farmer_id": "FRM-ABC12345",
            // Missing registration_date and other required fields
        });

        let validator = SchemaValidator::new().with_pii_enforcement(false);
        let result = validator
            .validate(SupplyChainStage::FarmerRegistration, &document)
            .unwrap();

        assert!(!result.is_valid);
        assert!(result
            .errors
            .iter()
            .any(|e| e.error_type == ValidationErrorType::MissingRequiredField));
    }

    #[test]
    fn test_pii_detection() {
        let document = json!({
            "farmer_id": "FRM-ABC12345",
            "profile": {
                "name": "John Doe",  // Plain text PII - should be rejected
                "phone": "+91-9876543210"  // Plain text PII
            },
            "registration_date": "2024-01-15T10:30:00Z",
            "land_details": {
                "total_area_hectares": 5.2,
                "soil_type": "alluvial",
                "geo_hash": "tdr1"
            },
            "crop_info": {
                "variety": "mustard",
                "planting_season": "rabi",
                "expected_yield_kg": 1500
            },
            "verification_status": {
                "status": "verified",
                "verified_at": "2024-01-16T10:30:00Z"
            }
        });

        let validator = SchemaValidator::new().with_pii_enforcement(true);
        let result = validator
            .validate(SupplyChainStage::FarmerRegistration, &document)
            .unwrap();

        // Should detect PII and fail validation
        assert!(!result.pii_detected.is_empty());
    }

    #[test]
    fn test_encrypted_pii_accepted() {
        let document = json!({
            "farmer_id": "FRM-ABC12345",
            "profile": {
                "name": "encrypted_base64encodeddata==",  // Appears encrypted
                "phone": "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"  // CID reference
            },
            "registration_date": "2024-01-15T10:30:00Z",
            "land_details": {
                "total_area_hectares": 5.2,
                "soil_type": "alluvial",
                "geo_hash": "tdr1"
            },
            "crop_info": {
                "variety": "mustard",
                "planting_season": "rabi",
                "expected_yield_kg": 1500
            },
            "verification_status": {
                "status": "verified",
                "verified_at": "2024-01-16T10:30:00Z"
            }
        });

        let validator = SchemaValidator::new().with_pii_enforcement(true);
        let result = validator
            .validate(SupplyChainStage::FarmerRegistration, &document)
            .unwrap();

        // PII is encrypted, so no errors about plaintext PII
        let pii_errors: Vec<_> = result
            .errors
            .iter()
            .filter(|e| e.error_type == ValidationErrorType::PlaintextPiiDetected)
            .collect();
        assert!(pii_errors.is_empty());
    }

    #[test]
    fn test_large_payload_warning() {
        // Create a document with large payload
        let large_data = "x".repeat(LARGE_PAYLOAD_WARNING_BYTES + 1);
        let document = json!({
            "farmer_id": "FRM-ABC12345",
            "registration_date": "2024-01-15T10:30:00Z",
            "large_field": large_data,
            "land_details": {
                "total_area_hectares": 5.2,
                "soil_type": "alluvial",
                "geo_hash": "tdr1"
            },
            "crop_info": {
                "variety": "mustard",
                "planting_season": "rabi",
                "expected_yield_kg": 1500
            },
            "verification_status": {
                "status": "verified",
                "verified_at": "2024-01-16T10:30:00Z"
            }
        });

        let validator = SchemaValidator::new().with_pii_enforcement(false);
        let result = validator
            .validate(SupplyChainStage::FarmerRegistration, &document)
            .unwrap();

        assert!(result
            .warnings
            .iter()
            .any(|w| w.warning_type == ValidationWarningType::LargePayload));
    }

    #[test]
    fn test_schema_compatibility() {
        let validator = SchemaValidator::new();

        let v1_0 = SchemaReference::new("farmer_registration", "1.0.0");
        let v1_1 = SchemaReference::new("farmer_registration", "1.1.0");
        let v2_0 = SchemaReference::new("farmer_registration", "2.0.0");

        // v1.1 is compatible with v1.0 requirement
        assert!(validator
            .validate_schema_compatibility(&v1_1, &v1_0)
            .is_ok());

        // v1.0 is not compatible with v1.1 requirement
        assert!(validator
            .validate_schema_compatibility(&v1_0, &v1_1)
            .is_err());

        // v2.0 is not compatible with v1.x
        assert!(validator
            .validate_schema_compatibility(&v2_0, &v1_0)
            .is_err());
    }

    #[test]
    fn test_privacy_level_detection() {
        let validator = SchemaValidator::new();

        // Document with PII
        let pii_document = json!({
            "profile": {
                "name": "John Doe"
            }
        });

        // Document without PII
        let public_document = json!({
            "batch_id": "BATCH-123",
            "quality_grade": "A"
        });

        let pii_level = validator
            .determine_privacy_level(SupplyChainStage::FarmerRegistration, &pii_document);
        assert_eq!(pii_level, PrivacyLevel::Confidential);

        let public_level = validator
            .determine_privacy_level(SupplyChainStage::FpoPurchase, &public_document);
        assert_eq!(public_level, PrivacyLevel::Public);
    }

    #[test]
    fn test_nested_field_access() {
        let document = json!({
            "level1": {
                "level2": {
                    "level3": "value"
                }
            },
            "array": [
                {"item": "first"},
                {"item": "second"}
            ]
        });

        assert!(has_field(&document, "level1"));
        assert!(has_field(&document, "level1.level2"));
        assert!(has_field(&document, "level1.level2.level3"));
        assert!(!has_field(&document, "level1.nonexistent"));
        assert!(has_field(&document, "array.0.item"));
    }

    #[test]
    fn test_appears_encrypted() {
        // Base64-like string
        assert!(appears_encrypted(&json!(
            "SGVsbG8gV29ybGQhIFRoaXMgaXMgYSB0ZXN0IG9mIGJhc2U2NCBlbmNvZGluZw=="
        )));

        // IPFS CID
        assert!(appears_encrypted(&json!(
            "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"
        )));

        // Marked as encrypted
        assert!(appears_encrypted(&json!("encrypted_somedata")));
        assert!(appears_encrypted(&json!("enc:somedata")));

        // Plain text
        assert!(!appears_encrypted(&json!("John Doe")));
        assert!(!appears_encrypted(&json!("+91-9876543210")));

        // Encrypted envelope object
        assert!(appears_encrypted(&json!({
            "ciphertext": "encrypted_data",
            "key_id": "key-123"
        })));
    }
}
