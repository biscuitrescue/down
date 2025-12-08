//! PII (Personally Identifiable Information) Detection Module
//!
//! Provides automatic detection of PII fields in JSON documents using:
//! - Field name pattern matching
//! - Value pattern matching (regex-based)
//! - Contextual analysis
//!
//! This module is critical for ensuring no raw PII is stored on IPFS.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use tracing::debug;

/// Types of PII that can be detected
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiiType {
    /// Full name (first, last, middle)
    Name,
    /// Phone number (mobile, landline)
    Phone,
    /// Email address
    Email,
    /// Physical address
    Address,
    /// Aadhaar number (Indian national ID)
    Aadhaar,
    /// PAN (Permanent Account Number - Indian tax ID)
    Pan,
    /// Passport number
    Passport,
    /// Driving license number
    DrivingLicense,
    /// Bank account number
    BankAccount,
    /// IFSC code (Indian bank routing)
    Ifsc,
    /// Credit/Debit card number
    CardNumber,
    /// Date of birth
    DateOfBirth,
    /// GPS coordinates (precise location)
    GpsCoordinates,
    /// Vehicle registration number
    VehicleNumber,
    /// Generic sensitive data
    Sensitive,
    /// Biometric data reference
    Biometric,
    /// Medical/health information
    Medical,
    /// Financial information
    Financial,
}

impl PiiType {
    /// Get the default severity for this PII type
    pub fn default_severity(&self) -> PiiSeverity {
        match self {
            Self::Aadhaar | Self::Pan | Self::Passport | Self::Biometric => PiiSeverity::Critical,
            Self::BankAccount | Self::CardNumber | Self::Medical => PiiSeverity::High,
            Self::Name | Self::Phone | Self::Email | Self::Address | Self::DateOfBirth => {
                PiiSeverity::Medium
            }
            Self::DrivingLicense | Self::VehicleNumber | Self::Ifsc | Self::Financial => {
                PiiSeverity::Medium
            }
            Self::GpsCoordinates | Self::Sensitive => PiiSeverity::Low,
        }
    }
}

/// Severity level of detected PII
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PiiSeverity {
    /// Low risk - may be sensitive in context
    Low = 1,
    /// Medium risk - standard PII
    Medium = 2,
    /// High risk - sensitive PII requiring strong protection
    High = 3,
    /// Critical - government IDs, biometrics, highly sensitive
    Critical = 4,
}

/// Detected PII field information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiiField {
    /// JSON path to the field (e.g., "profile.name")
    pub path: String,
    /// Type of PII detected
    pub pii_type: PiiType,
    /// Severity level
    pub severity: PiiSeverity,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
    /// Detection method used
    pub detection_method: DetectionMethod,
    /// Sample of the value (truncated/masked for logging)
    pub masked_sample: Option<String>,
}

/// Method used to detect PII
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionMethod {
    /// Field name matched a known PII pattern
    FieldNamePattern,
    /// Value matched a regex pattern
    ValuePattern,
    /// Contextual analysis (neighboring fields)
    ContextualAnalysis,
    /// Explicit PII marker in schema
    SchemaDefinition,
    /// Machine learning model (future)
    MlModel,
}

/// Configuration for PII detection
#[derive(Debug, Clone)]
pub struct PiiDetectorConfig {
    /// Minimum confidence threshold for reporting
    pub min_confidence: f32,
    /// Whether to check field names
    pub check_field_names: bool,
    /// Whether to check field values
    pub check_field_values: bool,
    /// Whether to perform contextual analysis
    pub check_context: bool,
    /// Custom field name patterns to flag
    pub custom_field_patterns: Vec<String>,
    /// Fields to always ignore
    pub ignore_fields: HashSet<String>,
}

impl Default for PiiDetectorConfig {
    fn default() -> Self {
        Self {
            min_confidence: 0.5,
            check_field_names: true,
            check_field_values: true,
            check_context: true,
            custom_field_patterns: Vec::new(),
            ignore_fields: HashSet::from([
                "batch_id".to_string(),
                "sku_id".to_string(),
                "farmer_id".to_string(),
                "processing_id".to_string(),
                "warehouse_id".to_string(),
                "shipment_id".to_string(),
                "schema".to_string(),
                "metadata".to_string(),
                "version".to_string(),
                "timestamp".to_string(),
                "created_at".to_string(),
                "updated_at".to_string(),
            ]),
        }
    }
}

/// PII Detector for scanning JSON documents
pub struct PiiDetector {
    config: PiiDetectorConfig,
    field_patterns: Vec<FieldPattern>,
    value_patterns: Vec<ValuePattern>,
}

struct FieldPattern {
    pattern: Regex,
    pii_type: PiiType,
    confidence: f32,
}

struct ValuePattern {
    pattern: Regex,
    pii_type: PiiType,
    confidence: f32,
}

impl PiiDetector {
    /// Create a new PII detector with default configuration
    pub fn new() -> Self {
        Self::with_config(PiiDetectorConfig::default())
    }

    /// Create with custom configuration
    pub fn with_config(config: PiiDetectorConfig) -> Self {
        let field_patterns = Self::build_field_patterns(&config);
        let value_patterns = Self::build_value_patterns();

        Self {
            config,
            field_patterns,
            value_patterns,
        }
    }

    /// Build field name patterns for PII detection
    fn build_field_patterns(config: &PiiDetectorConfig) -> Vec<FieldPattern> {
        let mut patterns = vec![
            // Name patterns
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(full_?name|first_?name|last_?name|middle_?name|sur_?name|given_?name|family_?name|name)($|_|\.)").unwrap(),
                pii_type: PiiType::Name,
                confidence: 0.9,
            },
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(person|user|customer|farmer|driver|employee)_?name($|_|\.)").unwrap(),
                pii_type: PiiType::Name,
                confidence: 0.95,
            },
            // Phone patterns
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(phone|mobile|cell|telephone|contact_?number|tel)($|_|\.)").unwrap(),
                pii_type: PiiType::Phone,
                confidence: 0.9,
            },
            // Email patterns
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(email|e_?mail|mail)($|_|\.)").unwrap(),
                pii_type: PiiType::Email,
                confidence: 0.95,
            },
            // Address patterns
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(address|street|city|state|zip|postal|pincode|locality|village|district)($|_|\.)").unwrap(),
                pii_type: PiiType::Address,
                confidence: 0.8,
            },
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(home_?address|mailing_?address|residential_?address|permanent_?address)($|_|\.)").unwrap(),
                pii_type: PiiType::Address,
                confidence: 0.95,
            },
            // Indian ID patterns
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(aadhaar|aadhar|uid|uidai)($|_|\.)").unwrap(),
                pii_type: PiiType::Aadhaar,
                confidence: 0.98,
            },
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(pan|pan_?number|pan_?card)($|_|\.)").unwrap(),
                pii_type: PiiType::Pan,
                confidence: 0.95,
            },
            // Bank patterns
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(bank_?account|account_?number|acct_?no|bank_?acct)($|_|\.)").unwrap(),
                pii_type: PiiType::BankAccount,
                confidence: 0.95,
            },
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(ifsc|ifsc_?code|bank_?code)($|_|\.)").unwrap(),
                pii_type: PiiType::Ifsc,
                confidence: 0.9,
            },
            // Other document patterns
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(passport|passport_?number|passport_?no)($|_|\.)").unwrap(),
                pii_type: PiiType::Passport,
                confidence: 0.95,
            },
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(driving_?license|dl_?number|license_?number|driver_?license)($|_|\.)").unwrap(),
                pii_type: PiiType::DrivingLicense,
                confidence: 0.9,
            },
            // Date of birth
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(dob|date_?of_?birth|birth_?date|birthday)($|_|\.)").unwrap(),
                pii_type: PiiType::DateOfBirth,
                confidence: 0.9,
            },
            // Location patterns
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(coordinates|lat_?long|latitude|longitude|gps|location)($|_|\.)").unwrap(),
                pii_type: PiiType::GpsCoordinates,
                confidence: 0.7,
            },
            // Vehicle patterns
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(vehicle_?number|registration_?number|reg_?no|license_?plate|number_?plate)($|_|\.)").unwrap(),
                pii_type: PiiType::VehicleNumber,
                confidence: 0.85,
            },
            // Card patterns
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(card_?number|credit_?card|debit_?card|cvv|card_?cvv)($|_|\.)").unwrap(),
                pii_type: PiiType::CardNumber,
                confidence: 0.95,
            },
            // Biometric patterns
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(biometric|fingerprint|iris|face_?id|face_?data)($|_|\.)").unwrap(),
                pii_type: PiiType::Biometric,
                confidence: 0.98,
            },
            // Medical patterns
            FieldPattern {
                pattern: Regex::new(r"(?i)(^|_|\.)(medical|health|diagnosis|prescription|blood_?type|allergy)($|_|\.)").unwrap(),
                pii_type: PiiType::Medical,
                confidence: 0.85,
            },
        ];

        // Add custom patterns
        for custom_pattern in &config.custom_field_patterns {
            if let Ok(regex) = Regex::new(custom_pattern) {
                patterns.push(FieldPattern {
                    pattern: regex,
                    pii_type: PiiType::Sensitive,
                    confidence: 0.8,
                });
            }
        }

        patterns
    }

    /// Build value patterns for PII detection
    fn build_value_patterns() -> Vec<ValuePattern> {
        vec![
            // Email pattern
            ValuePattern {
                pattern: Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap(),
                pii_type: PiiType::Email,
                confidence: 0.95,
            },
            // Indian phone number pattern (+91 or 10 digits starting with 6-9)
            ValuePattern {
                pattern: Regex::new(r"^(\+91[\-\s]?)?[6-9]\d{9}$").unwrap(),
                pii_type: PiiType::Phone,
                confidence: 0.9,
            },
            // International phone pattern
            ValuePattern {
                pattern: Regex::new(r"^\+\d{1,3}[\-\s]?\d{6,14}$").unwrap(),
                pii_type: PiiType::Phone,
                confidence: 0.85,
            },
            // Aadhaar pattern (12 digits, often formatted as XXXX XXXX XXXX)
            ValuePattern {
                pattern: Regex::new(r"^\d{4}\s?\d{4}\s?\d{4}$").unwrap(),
                pii_type: PiiType::Aadhaar,
                confidence: 0.7, // Lower confidence as many 12-digit numbers exist
            },
            // PAN pattern (AAAAA0000A format)
            ValuePattern {
                pattern: Regex::new(r"^[A-Z]{5}\d{4}[A-Z]$").unwrap(),
                pii_type: PiiType::Pan,
                confidence: 0.95,
            },
            // IFSC pattern (AAAA0XXXXXX format)
            ValuePattern {
                pattern: Regex::new(r"^[A-Z]{4}0[A-Z0-9]{6}$").unwrap(),
                pii_type: PiiType::Ifsc,
                confidence: 0.9,
            },
            // Indian passport pattern
            ValuePattern {
                pattern: Regex::new(r"^[A-Z]\d{7}$").unwrap(),
                pii_type: PiiType::Passport,
                confidence: 0.8,
            },
            // Indian driving license (varies by state, basic pattern)
            ValuePattern {
                pattern: Regex::new(r"^[A-Z]{2}\d{2}\s?\d{4}\s?\d{7}$").unwrap(),
                pii_type: PiiType::DrivingLicense,
                confidence: 0.75,
            },
            // Vehicle registration (Indian format)
            ValuePattern {
                pattern: Regex::new(r"^[A-Z]{2}\s?\d{1,2}\s?[A-Z]{1,3}\s?\d{4}$").unwrap(),
                pii_type: PiiType::VehicleNumber,
                confidence: 0.85,
            },
            // Credit card number (basic 16 digit check)
            ValuePattern {
                pattern: Regex::new(r"^\d{4}[\s-]?\d{4}[\s-]?\d{4}[\s-]?\d{4}$").unwrap(),
                pii_type: PiiType::CardNumber,
                confidence: 0.8,
            },
            // Date of birth patterns (various formats)
            ValuePattern {
                pattern: Regex::new(r"^\d{2}[/-]\d{2}[/-]\d{4}$").unwrap(),
                pii_type: PiiType::DateOfBirth,
                confidence: 0.6, // Low confidence as dates are common
            },
            // GPS coordinates
            ValuePattern {
                pattern: Regex::new(r"^-?\d{1,3}\.\d{4,}$").unwrap(),
                pii_type: PiiType::GpsCoordinates,
                confidence: 0.6,
            },
        ]
    }

    /// Detect PII in a JSON document
    pub fn detect_pii(&self, document: &Value) -> Vec<PiiField> {
        let mut detected = Vec::new();
        self.scan_value(document, "", &mut detected);

        // Filter by minimum confidence
        detected.retain(|f| f.confidence >= self.config.min_confidence);

        // Sort by severity (highest first) then by confidence
        detected.sort_by(|a, b| {
            b.severity
                .cmp(&a.severity)
                .then(b.confidence.partial_cmp(&a.confidence).unwrap())
        });

        debug!(
            detected_count = detected.len(),
            "PII detection completed"
        );

        detected
    }

    /// Recursively scan a JSON value for PII
    fn scan_value(&self, value: &Value, path: &str, detected: &mut Vec<PiiField>) {
        match value {
            Value::Object(map) => {
                for (key, val) in map {
                    let field_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{}", path, key)
                    };

                    // Skip ignored fields
                    if self.config.ignore_fields.contains(key) {
                        continue;
                    }

                    // Check field name
                    if self.config.check_field_names {
                        if let Some(mut pii) = self.check_field_name(&field_path, key) {
                            // Also check the value if it's a string
                            if let Value::String(s) = val {
                                pii.masked_sample = Some(mask_value(s));
                            }
                            detected.push(pii);
                        }
                    }

                    // Recursively scan the value
                    self.scan_value(val, &field_path, detected);
                }
            }
            Value::Array(arr) => {
                for (idx, item) in arr.iter().enumerate() {
                    let item_path = format!("{}.{}", path, idx);
                    self.scan_value(item, &item_path, detected);
                }
            }
            Value::String(s) => {
                // Check value patterns
                if self.config.check_field_values {
                    if let Some(mut pii) = self.check_value_pattern(path, s) {
                        pii.masked_sample = Some(mask_value(s));
                        detected.push(pii);
                    }
                }
            }
            _ => {}
        }
    }

    /// Check if a field name matches any PII pattern
    fn check_field_name(&self, path: &str, name: &str) -> Option<PiiField> {
        for pattern in &self.field_patterns {
            if pattern.pattern.is_match(name) || pattern.pattern.is_match(path) {
                return Some(PiiField {
                    path: path.to_string(),
                    pii_type: pattern.pii_type,
                    severity: pattern.pii_type.default_severity(),
                    confidence: pattern.confidence,
                    detection_method: DetectionMethod::FieldNamePattern,
                    masked_sample: None,
                });
            }
        }
        None
    }

    /// Check if a value matches any PII pattern
    fn check_value_pattern(&self, path: &str, value: &str) -> Option<PiiField> {
        // Skip very short or very long strings
        if value.len() < 3 || value.len() > 100 {
            return None;
        }

        for pattern in &self.value_patterns {
            if pattern.pattern.is_match(value) {
                return Some(PiiField {
                    path: path.to_string(),
                    pii_type: pattern.pii_type,
                    severity: pattern.pii_type.default_severity(),
                    confidence: pattern.confidence,
                    detection_method: DetectionMethod::ValuePattern,
                    masked_sample: None,
                });
            }
        }
        None
    }

    /// Check if a specific field contains PII
    pub fn is_pii_field(&self, field_name: &str, value: Option<&str>) -> bool {
        // Check field name
        for pattern in &self.field_patterns {
            if pattern.pattern.is_match(field_name) {
                return true;
            }
        }

        // Check value if provided
        if let Some(val) = value {
            for pattern in &self.value_patterns {
                if pattern.pattern.is_match(val) {
                    return true;
                }
            }
        }

        false
    }

    /// Get detection statistics
    pub fn get_stats(&self) -> PiiDetectorStats {
        PiiDetectorStats {
            field_patterns_count: self.field_patterns.len(),
            value_patterns_count: self.value_patterns.len(),
            ignored_fields_count: self.config.ignore_fields.len(),
            min_confidence: self.config.min_confidence,
        }
    }
}

impl Default for PiiDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about the PII detector configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiiDetectorStats {
    pub field_patterns_count: usize,
    pub value_patterns_count: usize,
    pub ignored_fields_count: usize,
    pub min_confidence: f32,
}

/// Mask a PII value for safe logging
fn mask_value(value: &str) -> String {
    let len = value.len();
    if len <= 4 {
        "*".repeat(len)
    } else if len <= 8 {
        format!("{}****", &value[..2])
    } else {
        format!("{}****{}", &value[..2], &value[len - 2..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_detect_name_field() {
        let detector = PiiDetector::new();
        let doc = json!({
            "farmer_name": "John Doe",
            "batch_id": "BATCH-123"
        });

        let detected = detector.detect_pii(&doc);
        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].pii_type, PiiType::Name);
        assert_eq!(detected[0].path, "farmer_name");
    }

    #[test]
    fn test_detect_nested_pii() {
        let detector = PiiDetector::new();
        let doc = json!({
            "profile": {
                "name": "Jane Doe",
                "phone": "+91-9876543210",
                "details": {
                    "email": "jane@example.com"
                }
            }
        });

        let detected = detector.detect_pii(&doc);
        assert!(detected.len() >= 3);

        let paths: Vec<_> = detected.iter().map(|d| d.path.as_str()).collect();
        assert!(paths.contains(&"profile.name"));
        assert!(paths.contains(&"profile.phone"));
        assert!(paths.contains(&"profile.details.email"));
    }

    #[test]
    fn test_detect_aadhaar_pattern() {
        let detector = PiiDetector::new();
        let doc = json!({
            "aadhaar_number": "1234 5678 9012"
        });

        let detected = detector.detect_pii(&doc);
        assert!(!detected.is_empty());
        assert!(detected.iter().any(|d| d.pii_type == PiiType::Aadhaar));
    }

    #[test]
    fn test_detect_pan_pattern() {
        let detector = PiiDetector::new();
        let doc = json!({
            "pan_number": "ABCDE1234F"
        });

        let detected = detector.detect_pii(&doc);
        assert!(!detected.is_empty());
        assert!(detected.iter().any(|d| d.pii_type == PiiType::Pan));
    }

    #[test]
    fn test_detect_email_by_value() {
        let detector = PiiDetector::new();
        let doc = json!({
            "contact": "test@example.com"
        });

        let detected = detector.detect_pii(&doc);
        assert!(!detected.is_empty());
        assert!(detected.iter().any(|d| d.pii_type == PiiType::Email));
    }

    #[test]
    fn test_detect_phone_by_value() {
        let detector = PiiDetector::new();
        let doc = json!({
            "contact_number": "+91-9876543210"
        });

        let detected = detector.detect_pii(&doc);
        assert!(!detected.is_empty());
        assert!(detected.iter().any(|d| d.pii_type == PiiType::Phone));
    }

    #[test]
    fn test_ignored_fields() {
        let detector = PiiDetector::new();
        let doc = json!({
            "farmer_id": "FRM-123456",  // Should be ignored
            "batch_id": "BATCH-789",     // Should be ignored
            "farmer_name": "Test Name"   // Should be detected
        });

        let detected = detector.detect_pii(&doc);
        let paths: Vec<_> = detected.iter().map(|d| d.path.as_str()).collect();

        assert!(!paths.contains(&"farmer_id"));
        assert!(!paths.contains(&"batch_id"));
        assert!(paths.contains(&"farmer_name"));
    }

    #[test]
    fn test_no_pii_document() {
        let detector = PiiDetector::new();
        let doc = json!({
            "batch_id": "BATCH-123",
            "quantity_kg": 1500,
            "quality_grade": "A",
            "moisture_content": 8.5
        });

        let detected = detector.detect_pii(&doc);
        assert!(detected.is_empty());
    }

    #[test]
    fn test_severity_ordering() {
        let detector = PiiDetector::new();
        let doc = json!({
            "name": "John",           // Medium severity
            "aadhaar": "1234 5678 9012"  // Critical severity
        });

        let detected = detector.detect_pii(&doc);
        assert!(detected.len() >= 2);

        // Critical should come first
        assert_eq!(detected[0].severity, PiiSeverity::Critical);
    }

    #[test]
    fn test_mask_value() {
        assert_eq!(mask_value("abc"), "***");
        assert_eq!(mask_value("abcd"), "****");
        assert_eq!(mask_value("abcdefgh"), "ab****");
        assert_eq!(mask_value("1234567890"), "12****90");
    }

    #[test]
    fn test_pii_type_severity() {
        assert_eq!(PiiType::Aadhaar.default_severity(), PiiSeverity::Critical);
        assert_eq!(PiiType::Name.default_severity(), PiiSeverity::Medium);
        assert_eq!(PiiType::GpsCoordinates.default_severity(), PiiSeverity::Low);
    }

    #[test]
    fn test_is_pii_field() {
        let detector = PiiDetector::new();

        assert!(detector.is_pii_field("phone_number", None));
        assert!(detector.is_pii_field("random_field", Some("test@example.com")));
        assert!(!detector.is_pii_field("batch_id", Some("BATCH-123")));
    }

    #[test]
    fn test_array_scanning() {
        let detector = PiiDetector::new();
        let doc = json!({
            "contacts": [
                {"name": "Person 1", "phone": "9876543210"},
                {"name": "Person 2", "phone": "8765432109"}
            ]
        });

        let detected = detector.detect_pii(&doc);
        assert!(detected.len() >= 4); // 2 names + 2 phones

        let paths: Vec<_> = detected.iter().map(|d| d.path.as_str()).collect();
        assert!(paths.iter().any(|p| p.contains("contacts.0")));
        assert!(paths.iter().any(|p| p.contains("contacts.1")));
    }

    #[test]
    fn test_custom_config() {
        let mut config = PiiDetectorConfig::default();
        config.min_confidence = 0.9;
        config.custom_field_patterns.push(r"(?i)secret".to_string());

        let detector = PiiDetector::with_config(config);
        let doc = json!({
            "secret_data": "something sensitive"
        });

        let detected = detector.detect_pii(&doc);
        assert!(!detected.is_empty());
    }

    #[test]
    fn test_detector_stats() {
        let detector = PiiDetector::new();
        let stats = detector.get_stats();

        assert!(stats.field_patterns_count > 0);
        assert!(stats.value_patterns_count > 0);
        assert!(stats.ignored_fields_count > 0);
    }
}
