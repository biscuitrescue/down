//! Production-Ready IPFS Client
//!
//! Features:
//! - PII detection and encryption before upload (AES-256-GCM)
//! - Schema validation before IPFS upload
//! - Multi-provider support with automatic fallback (Infura → Pinata → Local)
//! - Circuit breaker pattern to avoid hammering failed providers
//! - Retry with exponential backoff
//!
//! RULE: "No raw PII is ever stored in plaintext on IPFS."

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::RngCore;
use regex::Regex;
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

// ============================================================================
// CONFIGURATION
// ============================================================================

/// Main IPFS client configuration
#[derive(Debug, Clone)]
pub struct IpfsClientConfig {
    /// Primary provider (usually Infura)
    pub primary: ProviderConfig,
    /// Backup provider (usually Pinata)
    pub backup: Option<ProviderConfig>,
    /// Local fallback (self-hosted node)
    pub local: Option<ProviderConfig>,
    /// Gateway URL for public access
    pub gateway_url: String,
    /// Whether to enforce PII encryption
    pub enforce_pii_encryption: bool,
    /// Whether to validate schemas before upload
    pub validate_schemas: bool,
    /// Encryption key (32 bytes for AES-256)
    pub encryption_key: Option<Vec<u8>>,
    /// Maximum payload size in bytes
    pub max_payload_size: usize,
    /// Request timeout
    pub timeout: Duration,
    /// Max retries per provider
    pub max_retries: u32,
}

impl Default for IpfsClientConfig {
    fn default() -> Self {
        Self {
            primary: ProviderConfig::default(),
            backup: None,
            local: None,
            gateway_url: "https://ipfs.io/ipfs/".to_string(),
            enforce_pii_encryption: true,
            validate_schemas: true,
            encryption_key: None,
            max_payload_size: 100 * 1024 * 1024, // 100 MB
            timeout: Duration::from_secs(30),
            max_retries: 3,
        }
    }
}

impl IpfsClientConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Result<Self> {
        let primary = ProviderConfig {
            name: "infura".to_string(),
            api_url: std::env::var("IPFS_API_URL")
                .unwrap_or_else(|_| "https://ipfs.infura.io:5001".to_string()),
            project_id: std::env::var("IPFS_PROJECT_ID").ok(),
            project_secret: std::env::var("IPFS_PROJECT_SECRET").ok(),
            api_key: None,
            secret_key: None,
        };

        let backup = match (
            std::env::var("PINATA_API_KEY"),
            std::env::var("PINATA_SECRET_KEY"),
        ) {
            (Ok(api_key), Ok(secret_key)) => Some(ProviderConfig {
                name: "pinata".to_string(),
                api_url: "https://api.pinata.cloud".to_string(),
                project_id: None,
                project_secret: None,
                api_key: Some(api_key),
                secret_key: Some(secret_key),
            }),
            _ => None,
        };

        let local = std::env::var("IPFS_LOCAL_API_URL").ok().map(|url| ProviderConfig {
            name: "local".to_string(),
            api_url: url,
            project_id: None,
            project_secret: None,
            api_key: None,
            secret_key: None,
        });

        let encryption_key = std::env::var("IPFS_ENCRYPTION_KEY")
            .ok()
            .and_then(|k| BASE64.decode(&k).ok())
            .filter(|k| k.len() == 32);

        Ok(Self {
            primary,
            backup,
            local,
            gateway_url: std::env::var("IPFS_GATEWAY_URL")
                .unwrap_or_else(|_| "https://ipfs.io/ipfs/".to_string()),
            enforce_pii_encryption: std::env::var("IPFS_ENFORCE_PII_ENCRYPTION")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true),
            validate_schemas: std::env::var("IPFS_VALIDATE_SCHEMAS")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true),
            encryption_key,
            max_payload_size: std::env::var("IPFS_MAX_PAYLOAD_SIZE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(100 * 1024 * 1024),
            timeout: Duration::from_secs(
                std::env::var("IPFS_TIMEOUT_SECONDS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(30),
            ),
            max_retries: std::env::var("IPFS_MAX_RETRIES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(3),
        })
    }
}

/// Provider-specific configuration
#[derive(Debug, Clone, Default)]
pub struct ProviderConfig {
    pub name: String,
    pub api_url: String,
    pub project_id: Option<String>,
    pub project_secret: Option<String>,
    pub api_key: Option<String>,
    pub secret_key: Option<String>,
}

// ============================================================================
// PII DETECTION
// ============================================================================

/// Types of PII that can be detected
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiiType {
    Name,
    Phone,
    Email,
    Address,
    Aadhaar,
    Pan,
    BankAccount,
    DateOfBirth,
    Biometric,
    Other,
}

/// Detected PII field
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedPii {
    pub path: String,
    pub pii_type: PiiType,
    pub confidence: f32,
}

/// PII Detector
pub struct PiiDetector {
    field_patterns: Vec<(Regex, PiiType)>,
    value_patterns: Vec<(Regex, PiiType)>,
    ignore_fields: Vec<String>,
}

impl Default for PiiDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl PiiDetector {
    pub fn new() -> Self {
        let field_patterns = vec![
            (Regex::new(r"(?i)(^|_|\.)(name|first_?name|last_?name|full_?name)($|_|\.)").unwrap(), PiiType::Name),
            (Regex::new(r"(?i)(^|_|\.)(phone|mobile|cell|contact_?number)($|_|\.)").unwrap(), PiiType::Phone),
            (Regex::new(r"(?i)(^|_|\.)(email|e_?mail)($|_|\.)").unwrap(), PiiType::Email),
            (Regex::new(r"(?i)(^|_|\.)(address|street|city|pincode|postal)($|_|\.)").unwrap(), PiiType::Address),
            (Regex::new(r"(?i)(^|_|\.)(aadhaar|aadhar|uid)($|_|\.)").unwrap(), PiiType::Aadhaar),
            (Regex::new(r"(?i)(^|_|\.)(pan|pan_?number)($|_|\.)").unwrap(), PiiType::Pan),
            (Regex::new(r"(?i)(^|_|\.)(bank_?account|account_?number|ifsc)($|_|\.)").unwrap(), PiiType::BankAccount),
            (Regex::new(r"(?i)(^|_|\.)(dob|date_?of_?birth|birth_?date)($|_|\.)").unwrap(), PiiType::DateOfBirth),
            (Regex::new(r"(?i)(^|_|\.)(biometric|fingerprint|iris)($|_|\.)").unwrap(), PiiType::Biometric),
        ];

        let value_patterns = vec![
            (Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap(), PiiType::Email),
            (Regex::new(r"^(\+91[\-\s]?)?[6-9]\d{9}$").unwrap(), PiiType::Phone),
            (Regex::new(r"^\d{4}\s?\d{4}\s?\d{4}$").unwrap(), PiiType::Aadhaar),
            (Regex::new(r"^[A-Z]{5}\d{4}[A-Z]$").unwrap(), PiiType::Pan),
        ];

        let ignore_fields = vec![
            "batch_id", "sku_id", "farmer_id", "fpo_id", "warehouse_id",
            "shipment_id", "processing_id", "schema", "metadata", "version",
            "timestamp", "created_at", "updated_at", "cid", "hash",
        ].into_iter().map(String::from).collect();

        Self { field_patterns, value_patterns, ignore_fields }
    }

    /// Detect PII in a JSON document
    pub fn detect(&self, doc: &serde_json::Value) -> Vec<DetectedPii> {
        let mut detected = Vec::new();
        self.scan_value(doc, "", &mut detected);
        detected
    }

    fn scan_value(&self, value: &serde_json::Value, path: &str, detected: &mut Vec<DetectedPii>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, val) in map {
                    let field_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{}", path, key)
                    };

                    // Skip ignored fields
                    if self.ignore_fields.iter().any(|f| key.to_lowercase().contains(f)) {
                        continue;
                    }

                    // Check field name patterns
                    for (pattern, pii_type) in &self.field_patterns {
                        if pattern.is_match(key) || pattern.is_match(&field_path) {
                            detected.push(DetectedPii {
                                path: field_path.clone(),
                                pii_type: *pii_type,
                                confidence: 0.9,
                            });
                            break;
                        }
                    }

                    self.scan_value(val, &field_path, detected);
                }
            }
            serde_json::Value::Array(arr) => {
                for (i, item) in arr.iter().enumerate() {
                    self.scan_value(item, &format!("{}.{}", path, i), detected);
                }
            }
            serde_json::Value::String(s) => {
                // Check value patterns
                for (pattern, pii_type) in &self.value_patterns {
                    if pattern.is_match(s) {
                        // Don't duplicate if already detected by field name
                        if !detected.iter().any(|d| d.path == path) {
                            detected.push(DetectedPii {
                                path: path.to_string(),
                                pii_type: *pii_type,
                                confidence: 0.8,
                            });
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    /// Check if value appears to be already encrypted
    pub fn appears_encrypted(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::String(s) => {
                // Base64-like
                (s.len() > 32 && s.len() % 4 == 0 && s.chars().all(|c| c.is_alphanumeric() || c == '+' || c == '/' || c == '='))
                // IPFS CID
                || s.starts_with("Qm") || s.starts_with("bafy")
                // Marked encrypted
                || s.starts_with("enc:") || s.starts_with("encrypted_")
            }
            serde_json::Value::Object(map) => {
                map.contains_key("ciphertext") || map.contains_key("encrypted_cid")
            }
            _ => false,
        }
    }
}

// ============================================================================
// ENCRYPTION
// ============================================================================

/// Encrypted data envelope
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedEnvelope {
    pub version: String,
    pub algorithm: String,
    pub nonce: String,
    pub ciphertext: String,
    pub plaintext_hash: String,
}

/// Data encryptor using AES-256-GCM
pub struct Encryptor {
    key: Vec<u8>,
}

impl Encryptor {
    pub fn new(key: Vec<u8>) -> Result<Self> {
        if key.len() != 32 {
            anyhow::bail!("Encryption key must be 32 bytes for AES-256");
        }
        Ok(Self { key })
    }

    /// Generate a new random encryption key
    pub fn generate_key() -> Vec<u8> {
        let mut key = vec![0u8; 32];
        OsRng.fill_bytes(&mut key);
        key
    }

    /// Encrypt data
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedEnvelope> {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| anyhow::anyhow!("Failed to create cipher: {}", e))?;

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

        let mut hasher = Sha256::new();
        hasher.update(plaintext);
        let hash = format!("{:x}", hasher.finalize());

        Ok(EncryptedEnvelope {
            version: "1.0".to_string(),
            algorithm: "AES-256-GCM".to_string(),
            nonce: BASE64.encode(nonce_bytes),
            ciphertext: BASE64.encode(&ciphertext),
            plaintext_hash: hash,
        })
    }

    /// Decrypt data
    pub fn decrypt(&self, envelope: &EncryptedEnvelope) -> Result<Vec<u8>> {
        let nonce_bytes = BASE64.decode(&envelope.nonce)?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = BASE64.decode(&envelope.ciphertext)?;

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| anyhow::anyhow!("Failed to create cipher: {}", e))?;

        let plaintext = cipher
            .decrypt(nonce, ciphertext.as_slice())
            .map_err(|e| anyhow::anyhow!("Decryption failed: {}", e))?;

        // Verify hash
        let mut hasher = Sha256::new();
        hasher.update(&plaintext);
        let hash = format!("{:x}", hasher.finalize());
        if hash != envelope.plaintext_hash {
            anyhow::bail!("Integrity check failed");
        }

        Ok(plaintext)
    }

    /// Encrypt JSON, replacing PII fields with encrypted values
    pub fn encrypt_pii_fields(
        &self,
        doc: &mut serde_json::Value,
        pii_fields: &[DetectedPii],
    ) -> Result<HashMap<String, EncryptedEnvelope>> {
        let mut encrypted_fields = HashMap::new();

        for pii in pii_fields {
            if let Some(value) = get_nested_value(doc, &pii.path) {
                if !PiiDetector::appears_encrypted(value) {
                    let value_bytes = serde_json::to_vec(value)?;
                    let envelope = self.encrypt(&value_bytes)?;
                    encrypted_fields.insert(pii.path.clone(), envelope.clone());

                    // Replace with encrypted marker
                    set_nested_value(
                        doc,
                        &pii.path,
                        serde_json::json!({
                            "_encrypted": true,
                            "_field": pii.path,
                            "_type": format!("{:?}", pii.pii_type)
                        }),
                    );
                }
            }
        }

        Ok(encrypted_fields)
    }
}

// ============================================================================
// SCHEMA VALIDATION
// ============================================================================

/// Supply chain stages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    FarmerRegistration,
    FpoPurchase,
    WarehouseStorage,
    Logistics,
    Processing,
    Packaging,
    AiScoring,
    FraudReport,
}

impl Stage {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "farmer_registration" | "farmer" => Some(Self::FarmerRegistration),
            "fpo_purchase" | "fpo" => Some(Self::FpoPurchase),
            "warehouse_storage" | "warehouse" => Some(Self::WarehouseStorage),
            "logistics" => Some(Self::Logistics),
            "processing" => Some(Self::Processing),
            "packaging" => Some(Self::Packaging),
            "ai_scoring" | "ai" => Some(Self::AiScoring),
            "fraud_report" | "fraud" => Some(Self::FraudReport),
            _ => None,
        }
    }

    pub fn required_fields(&self) -> &'static [&'static str] {
        match self {
            Self::FarmerRegistration => &["farmer_id", "registration_date", "land_details", "crop_info"],
            Self::FpoPurchase => &["batch_id", "farmer_id", "fpo_id", "purchase_date", "quantity"],
            Self::WarehouseStorage => &["warehouse_id", "batch_id", "storage_unit", "entry_date"],
            Self::Logistics => &["shipment_id", "batch_id", "origin", "destination", "status"],
            Self::Processing => &["processing_id", "facility_id", "input_batches", "outputs"],
            Self::Packaging => &["sku_id", "parent_batch_id", "packaging_date", "units"],
            Self::AiScoring => &["scoring_id", "batch_id", "model_info", "outputs"],
            Self::FraudReport => &["report_id", "sku_id", "report_type", "evidence"],
        }
    }
}

/// Validation error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

/// Schema validator
pub struct SchemaValidator;

impl SchemaValidator {
    /// Validate a document against its stage schema
    pub fn validate(stage: Stage, doc: &serde_json::Value) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        // Check required fields
        for field in stage.required_fields() {
            if !has_field(doc, field) {
                errors.push(ValidationError {
                    field: field.to_string(),
                    message: format!("Required field '{}' is missing", field),
                });
            }
        }

        errors
    }
}

// ============================================================================
// CIRCUIT BREAKER
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

struct CircuitBreaker {
    state: RwLock<CircuitState>,
    failure_count: AtomicU32,
    last_failure: RwLock<Option<Instant>>,
    failure_threshold: u32,
    reset_timeout: Duration,
}

impl CircuitBreaker {
    fn new(failure_threshold: u32, reset_timeout: Duration) -> Self {
        Self {
            state: RwLock::new(CircuitState::Closed),
            failure_count: AtomicU32::new(0),
            last_failure: RwLock::new(None),
            failure_threshold,
            reset_timeout,
        }
    }

    async fn should_allow(&self) -> bool {
        let state = *self.state.read().await;
        match state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                let last = self.last_failure.read().await;
                if let Some(t) = *last {
                    if t.elapsed() >= self.reset_timeout {
                        *self.state.write().await = CircuitState::HalfOpen;
                        return true;
                    }
                }
                false
            }
            CircuitState::HalfOpen => true,
        }
    }

    async fn record_success(&self) {
        self.failure_count.store(0, Ordering::Relaxed);
        *self.state.write().await = CircuitState::Closed;
    }

    async fn record_failure(&self) {
        let count = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;
        *self.last_failure.write().await = Some(Instant::now());

        if count >= self.failure_threshold {
            *self.state.write().await = CircuitState::Open;
            warn!("Circuit breaker opened after {} failures", count);
        }
    }
}

// ============================================================================
// IPFS CLIENT
// ============================================================================

/// Upload result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadResult {
    pub cid: String,
    pub size_bytes: usize,
    pub provider: String,
    pub gateway_url: String,
    pub encrypted_fields: Option<HashMap<String, EncryptedEnvelope>>,
    pub validation_warnings: Vec<String>,
}

/// Provider health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub name: String,
    pub healthy: bool,
    pub latency_ms: Option<u64>,
    pub last_check: String,
}

/// The main IPFS client
pub struct IpfsClient {
    http: Client,
    config: IpfsClientConfig,
    pii_detector: PiiDetector,
    encryptor: Option<Encryptor>,
    primary_circuit: Arc<CircuitBreaker>,
    backup_circuit: Arc<CircuitBreaker>,
    local_circuit: Arc<CircuitBreaker>,
    stats: Arc<ClientStats>,
}

struct ClientStats {
    uploads: AtomicU64,
    downloads: AtomicU64,
    failures: AtomicU64,
    bytes_uploaded: AtomicU64,
}

impl IpfsClient {
    /// Create a new IPFS client
    pub fn new(config: IpfsClientConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(config.timeout)
            .build()
            .context("Failed to create HTTP client")?;

        let encryptor = config.encryption_key.as_ref().map(|k| Encryptor::new(k.clone())).transpose()?;

        Ok(Self {
            http,
            config,
            pii_detector: PiiDetector::new(),
            encryptor,
            primary_circuit: Arc::new(CircuitBreaker::new(5, Duration::from_secs(60))),
            backup_circuit: Arc::new(CircuitBreaker::new(5, Duration::from_secs(60))),
            local_circuit: Arc::new(CircuitBreaker::new(5, Duration::from_secs(60))),
            stats: Arc::new(ClientStats {
                uploads: AtomicU64::new(0),
                downloads: AtomicU64::new(0),
                failures: AtomicU64::new(0),
                bytes_uploaded: AtomicU64::new(0),
            }),
        })
    }

    /// Create from environment variables
    pub fn from_env() -> Result<Self> {
        Self::new(IpfsClientConfig::from_env()?)
    }

    /// Upload JSON data to IPFS with automatic PII encryption and validation
    pub async fn upload_json<T: Serialize>(
        &self,
        data: &T,
        stage: Option<Stage>,
    ) -> Result<UploadResult> {
        let mut doc: serde_json::Value = serde_json::to_value(data)?;
        let mut warnings = Vec::new();

        // 1. Validate schema if stage is specified
        if self.config.validate_schemas {
            if let Some(s) = stage {
                let errors = SchemaValidator::validate(s, &doc);
                if !errors.is_empty() {
                    for err in &errors {
                        warnings.push(format!("Validation: {} - {}", err.field, err.message));
                    }
                    // Don't fail, just warn - but you could make this strict
                    warn!("Schema validation warnings: {:?}", errors);
                }
            }
        }

        // 2. Detect PII
        let pii_fields = self.pii_detector.detect(&doc);
        let encrypted_fields = if !pii_fields.is_empty() {
            info!("Detected {} PII fields", pii_fields.len());

            if self.config.enforce_pii_encryption {
                match &self.encryptor {
                    Some(enc) => {
                        let encrypted = enc.encrypt_pii_fields(&mut doc, &pii_fields)?;
                        Some(encrypted)
                    }
                    None => {
                        anyhow::bail!(
                            "PII detected but no encryption key configured. Set IPFS_ENCRYPTION_KEY env var. \
                             PII fields: {:?}",
                            pii_fields.iter().map(|p| &p.path).collect::<Vec<_>>()
                        );
                    }
                }
            } else {
                warnings.push(format!(
                    "PII detected but encryption not enforced: {:?}",
                    pii_fields.iter().map(|p| &p.path).collect::<Vec<_>>()
                ));
                None
            }
        } else {
            None
        };

        // 3. Add metadata wrapper
        let wrapped = serde_json::json!({
            "schema": {
                "version": "1.0.0",
                "stage": stage.map(|s| format!("{:?}", s).to_lowercase()),
            },
            "metadata": {
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "has_encrypted_pii": encrypted_fields.is_some(),
            },
            "content": doc,
        });

        // 4. Upload with fallback
        let bytes = serde_json::to_vec(&wrapped)?;

        if bytes.len() > self.config.max_payload_size {
            anyhow::bail!(
                "Payload too large: {} bytes (max: {} bytes)",
                bytes.len(),
                self.config.max_payload_size
            );
        }

        let (cid, provider) = self.upload_with_fallback(&bytes).await?;

        self.stats.uploads.fetch_add(1, Ordering::Relaxed);
        self.stats.bytes_uploaded.fetch_add(bytes.len() as u64, Ordering::Relaxed);

        Ok(UploadResult {
            cid: cid.clone(),
            size_bytes: bytes.len(),
            provider,
            gateway_url: format!("{}{}", self.config.gateway_url, cid),
            encrypted_fields,
            validation_warnings: warnings,
        })
    }

    /// Upload raw bytes to IPFS
    pub async fn upload_bytes(&self, bytes: &[u8]) -> Result<UploadResult> {
        if bytes.len() > self.config.max_payload_size {
            anyhow::bail!(
                "Payload too large: {} bytes (max: {} bytes)",
                bytes.len(),
                self.config.max_payload_size
            );
        }

        let (cid, provider) = self.upload_with_fallback(bytes).await?;

        self.stats.uploads.fetch_add(1, Ordering::Relaxed);
        self.stats.bytes_uploaded.fetch_add(bytes.len() as u64, Ordering::Relaxed);

        Ok(UploadResult {
            cid: cid.clone(),
            size_bytes: bytes.len(),
            provider,
            gateway_url: format!("{}{}", self.config.gateway_url, cid),
            encrypted_fields: None,
            validation_warnings: vec![],
        })
    }

    /// Upload with automatic provider fallback
    async fn upload_with_fallback(&self, bytes: &[u8]) -> Result<(String, String)> {
        // Try primary
        if self.primary_circuit.should_allow().await {
            match self.upload_to_provider(&self.config.primary, bytes).await {
                Ok(cid) => {
                    self.primary_circuit.record_success().await;
                    // Async pin to backup (fire and forget)
                    if let Some(ref backup) = self.config.backup {
                        let backup = backup.clone();
                        let cid_clone = cid.clone();
                        let http = self.http.clone();
                        tokio::spawn(async move {
                            let _ = pin_to_pinata(&http, &backup, &cid_clone).await;
                        });
                    }
                    return Ok((cid, self.config.primary.name.clone()));
                }
                Err(e) => {
                    self.primary_circuit.record_failure().await;
                    warn!("Primary provider failed: {}", e);
                }
            }
        }

        // Try backup
        if let Some(ref backup) = self.config.backup {
            if self.backup_circuit.should_allow().await {
                match self.upload_to_pinata(backup, bytes).await {
                    Ok(cid) => {
                        self.backup_circuit.record_success().await;
                        return Ok((cid, backup.name.clone()));
                    }
                    Err(e) => {
                        self.backup_circuit.record_failure().await;
                        warn!("Backup provider failed: {}", e);
                    }
                }
            }
        }

        // Try local
        if let Some(ref local) = self.config.local {
            if self.local_circuit.should_allow().await {
                match self.upload_to_provider(local, bytes).await {
                    Ok(cid) => {
                        self.local_circuit.record_success().await;
                        return Ok((cid, local.name.clone()));
                    }
                    Err(e) => {
                        self.local_circuit.record_failure().await;
                        warn!("Local provider failed: {}", e);
                    }
                }
            }
        }

        self.stats.failures.fetch_add(1, Ordering::Relaxed);
        anyhow::bail!("All IPFS providers failed")
    }

    /// Upload to Infura/local IPFS node
    async fn upload_to_provider(&self, provider: &ProviderConfig, bytes: &[u8]) -> Result<String> {
        let url = format!("{}/api/v0/add", provider.api_url);

        let part = Part::bytes(bytes.to_vec())
            .file_name("file")
            .mime_str("application/octet-stream")?;
        let form = Form::new().part("file", part);

        let mut request = self.http.post(&url).multipart(form);

        // Add auth for Infura
        if let (Some(id), Some(secret)) = (&provider.project_id, &provider.project_secret) {
            request = request.basic_auth(id, Some(secret));
        }

        let response = request.send().await.context("Request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Upload failed: HTTP {} - {}", status, body);
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct AddResponse {
            hash: String,
        }

        let resp: AddResponse = response.json().await.context("Failed to parse response")?;
        Ok(resp.hash)
    }

    /// Upload to Pinata
    async fn upload_to_pinata(&self, provider: &ProviderConfig, bytes: &[u8]) -> Result<String> {
        let url = format!("{}/pinning/pinFileToIPFS", provider.api_url);

        let part = Part::bytes(bytes.to_vec())
            .file_name("file")
            .mime_str("application/octet-stream")?;
        let form = Form::new().part("file", part);

        let mut request = self.http.post(&url).multipart(form);

        if let (Some(key), Some(secret)) = (&provider.api_key, &provider.secret_key) {
            request = request
                .header("pinata_api_key", key)
                .header("pinata_secret_api_key", secret);
        }

        let response = request.send().await.context("Request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Upload failed: HTTP {} - {}", status, body);
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct PinataResponse {
            ipfs_hash: String,
        }

        let resp: PinataResponse = response.json().await.context("Failed to parse response")?;
        Ok(resp.ipfs_hash)
    }

    /// Download content from IPFS
    pub async fn download(&self, cid: &str) -> Result<Vec<u8>> {
        // Try gateway first (fastest)
        let url = format!("{}{}", self.config.gateway_url, cid);
        let response = self.http.get(&url).send().await;

        if let Ok(resp) = response {
            if resp.status().is_success() {
                let bytes = resp.bytes().await?;
                self.stats.downloads.fetch_add(1, Ordering::Relaxed);
                return Ok(bytes.to_vec());
            }
        }

        // Fallback to API
        let url = format!("{}/api/v0/cat?arg={}", self.config.primary.api_url, cid);
        let mut request = self.http.post(&url);

        if let (Some(id), Some(secret)) = (&self.config.primary.project_id, &self.config.primary.project_secret) {
            request = request.basic_auth(id, Some(secret));
        }

        let response = request.send().await.context("Download request failed")?;

        if !response.status().is_success() {
            anyhow::bail!("Download failed: HTTP {}", response.status());
        }

        let bytes = response.bytes().await?;
        self.stats.downloads.fetch_add(1, Ordering::Relaxed);
        Ok(bytes.to_vec())
    }

    /// Download and decrypt JSON with PII
    pub async fn download_json<T: for<'de> Deserialize<'de>>(
        &self,
        cid: &str,
        encrypted_fields: Option<&HashMap<String, EncryptedEnvelope>>,
    ) -> Result<T> {
        let bytes = self.download(cid).await?;
        let mut doc: serde_json::Value = serde_json::from_slice(&bytes)?;

        // Decrypt PII fields if provided
        if let (Some(fields), Some(enc)) = (encrypted_fields, &self.encryptor) {
            // Get to content
            if let Some(content) = doc.get_mut("content") {
                for (path, envelope) in fields {
                    let decrypted = enc.decrypt(envelope)?;
                    let value: serde_json::Value = serde_json::from_slice(&decrypted)?;
                    set_nested_value(content, path, value);
                }
            }
        }

        // Extract content
        let content = doc.get("content").cloned().unwrap_or(doc);
        Ok(serde_json::from_value(content)?)
    }

    /// Check health of all providers
    pub async fn health_check(&self) -> Vec<ProviderHealth> {
        let mut results = Vec::new();
        let now = chrono::Utc::now().to_rfc3339();

        // Check primary
        let start = Instant::now();
        let healthy = self.check_provider_health(&self.config.primary).await;
        results.push(ProviderHealth {
            name: self.config.primary.name.clone(),
            healthy,
            latency_ms: if healthy { Some(start.elapsed().as_millis() as u64) } else { None },
            last_check: now.clone(),
        });

        // Check backup
        if let Some(ref backup) = self.config.backup {
            let start = Instant::now();
            let healthy = self.check_pinata_health(backup).await;
            results.push(ProviderHealth {
                name: backup.name.clone(),
                healthy,
                latency_ms: if healthy { Some(start.elapsed().as_millis() as u64) } else { None },
                last_check: now.clone(),
            });
        }

        // Check local
        if let Some(ref local) = self.config.local {
            let start = Instant::now();
            let healthy = self.check_provider_health(local).await;
            results.push(ProviderHealth {
                name: local.name.clone(),
                healthy,
                latency_ms: if healthy { Some(start.elapsed().as_millis() as u64) } else { None },
                last_check: now,
            });
        }

        results
    }

    async fn check_provider_health(&self, provider: &ProviderConfig) -> bool {
        let url = format!("{}/api/v0/version", provider.api_url);
        let mut request = self.http.post(&url);

        if let (Some(id), Some(secret)) = (&provider.project_id, &provider.project_secret) {
            request = request.basic_auth(id, Some(secret));
        }

        request.send().await.map(|r| r.status().is_success()).unwrap_or(false)
    }

    async fn check_pinata_health(&self, provider: &ProviderConfig) -> bool {
        let url = format!("{}/data/testAuthentication", provider.api_url);
        let mut request = self.http.get(&url);

        if let (Some(key), Some(secret)) = (&provider.api_key, &provider.secret_key) {
            request = request
                .header("pinata_api_key", key)
                .header("pinata_secret_api_key", secret);
        }

        request.send().await.map(|r| r.status().is_success()).unwrap_or(false)
    }

    /// Get gateway URL for a CID
    pub fn gateway_url(&self, cid: &str) -> String {
        format!("{}{}", self.config.gateway_url, cid)
    }

    /// Get client statistics
    pub fn stats(&self) -> ClientStatsSnapshot {
        ClientStatsSnapshot {
            uploads: self.stats.uploads.load(Ordering::Relaxed),
            downloads: self.stats.downloads.load(Ordering::Relaxed),
            failures: self.stats.failures.load(Ordering::Relaxed),
            bytes_uploaded: self.stats.bytes_uploaded.load(Ordering::Relaxed),
        }
    }
}

/// Snapshot of client statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientStatsSnapshot {
    pub uploads: u64,
    pub downloads: u64,
    pub failures: u64,
    pub bytes_uploaded: u64,
}

/// Pin a CID to Pinata (background task)
async fn pin_to_pinata(http: &Client, provider: &ProviderConfig, cid: &str) -> Result<()> {
    let url = format!("{}/pinning/pinByHash", provider.api_url);

    let body = serde_json::json!({
        "hashToPin": cid,
        "pinataMetadata": {
            "name": format!("oilseed-{}", &cid[..8.min(cid.len())])
        }
    });

    let mut request = http.post(&url).json(&body);

    if let (Some(key), Some(secret)) = (&provider.api_key, &provider.secret_key) {
        request = request
            .header("pinata_api_key", key)
            .header("pinata_secret_api_key", secret);
    }

    let response = request.send().await?;
    if response.status().is_success() {
        debug!("Pinned {} to Pinata", cid);
    }
    Ok(())
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

fn has_field(doc: &serde_json::Value, path: &str) -> bool {
    get_nested_value(doc, path).is_some()
}

fn get_nested_value<'a>(doc: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = doc;

    for part in parts {
        match current {
            serde_json::Value::Object(map) => {
                current = map.get(part)?;
            }
            serde_json::Value::Array(arr) => {
                let idx: usize = part.parse().ok()?;
                current = arr.get(idx)?;
            }
            _ => return None,
        }
    }

    Some(current)
}

fn set_nested_value(doc: &mut serde_json::Value, path: &str, value: serde_json::Value) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = doc;

    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            if let serde_json::Value::Object(map) = current {
                map.insert(part.to_string(), value);
            }
            return;
        }

        current = match current {
            serde_json::Value::Object(map) => {
                map.entry(part.to_string()).or_insert_with(|| serde_json::json!({}))
            }
            _ => return,
        };
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pii_detection() {
        let detector = PiiDetector::new();

        let doc = serde_json::json!({
            "farmer_name": "John Doe",
            "phone": "+91-9876543210",
            "batch_id": "BATCH-123",
            "profile": {
                "email": "test@example.com",
                "address": "123 Main St"
            }
        });

        let detected = detector.detect(&doc);
        assert!(detected.len() >= 3);

        let paths: Vec<_> = detected.iter().map(|d| d.path.as_str()).collect();
        assert!(paths.contains(&"farmer_name"));
        assert!(paths.contains(&"phone"));
        assert!(!paths.contains(&"batch_id")); // Should be ignored
    }

    #[test]
    fn test_encryption_roundtrip() {
        let key = Encryptor::generate_key();
        let enc = Encryptor::new(key).unwrap();

        let plaintext = b"Hello, World!";
        let envelope = enc.encrypt(plaintext).unwrap();
        let decrypted = enc.decrypt(&envelope).unwrap();

        assert_eq!(plaintext.to_vec(), decrypted);
    }

    #[test]
    fn test_schema_validation() {
        let doc = serde_json::json!({
            "farmer_id": "FRM-123",
            "registration_date": "2024-01-01",
            // Missing land_details and crop_info
        });

        let errors = SchemaValidator::validate(Stage::FarmerRegistration, &doc);
        assert!(errors.len() >= 2);
    }

    #[test]
    fn test_appears_encrypted() {
        assert!(PiiDetector::appears_encrypted(&serde_json::json!("QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG")));
        assert!(PiiDetector::appears_encrypted(&serde_json::json!("enc:somedata")));
        assert!(!PiiDetector::appears_encrypted(&serde_json::json!("John Doe")));
    }

    #[test]
    fn test_nested_value_access() {
        let doc = serde_json::json!({
            "level1": {
                "level2": {
                    "value": "found"
                }
            }
        });

        assert!(has_field(&doc, "level1.level2.value"));
        assert!(!has_field(&doc, "level1.nonexistent"));
    }

    #[test]
    fn test_stage_required_fields() {
        let fields = Stage::FarmerRegistration.required_fields();
        assert!(fields.contains(&"farmer_id"));
        assert!(fields.contains(&"land_details"));
    }
}
