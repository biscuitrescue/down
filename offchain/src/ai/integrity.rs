//! AI Integrity Verification
//!
//! Ensures AI scoring reproducibility and auditability:
//! - Model hash verification (weights + config)
//! - Input data snapshot with CID references
//! - Output verification and consistency checks
//! - Complete audit trail for compliance

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Metadata about an AI model for reproducibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiModelMetadata {
    /// Model name/identifier
    pub name: String,
    /// Semantic version
    pub version: String,
    /// SHA256 hash of model weights + config
    pub hash: String,
    /// Framework used (pytorch, tensorflow, etc.)
    pub framework: String,
    /// Training date
    pub training_date: String,
    /// IPFS CID of input schema definition
    pub input_schema_cid: Option<String>,
    /// IPFS CID of model weights (if stored on IPFS)
    pub weights_cid: Option<String>,
}

impl AiModelMetadata {
    pub fn new(name: &str, version: &str, framework: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            hash: String::new(),
            framework: framework.to_string(),
            training_date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
            input_schema_cid: None,
            weights_cid: None,
        }
    }

    pub fn with_hash(mut self, hash: &str) -> Self {
        self.hash = hash.to_string();
        self
    }

    pub fn with_weights_cid(mut self, cid: &str) -> Self {
        self.weights_cid = Some(cid.to_string());
        self
    }

    pub fn with_input_schema_cid(mut self, cid: &str) -> Self {
        self.input_schema_cid = Some(cid.to_string());
        self
    }

    /// Generate a unique identifier for this model version
    pub fn model_id(&self) -> String {
        format!("{}:{}:{}", self.name, self.version, &self.hash[..8.min(self.hash.len())])
    }

    /// Verify the model hash against provided weights and config
    pub fn verify_hash(&self, weights: &[u8], config: &[u8]) -> bool {
        let computed = compute_model_hash(weights, config);
        computed == self.hash
    }
}

/// Input data for AI scoring with immutable references
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiScoringInput {
    /// Unique input batch ID
    pub input_id: String,
    /// References to input data CIDs
    pub data_cids: HashMap<String, String>,
    /// Hash of all input data combined
    pub combined_hash: String,
    /// Timestamp when inputs were collected
    pub collected_at: String,
    /// Data snapshot for reproducibility (small data only)
    pub snapshot: Option<serde_json::Value>,
    /// Schema version used for input validation
    pub schema_version: String,
}

impl AiScoringInput {
    pub fn new(schema_version: &str) -> Self {
        Self {
            input_id: uuid::Uuid::new_v4().to_string(),
            data_cids: HashMap::new(),
            combined_hash: String::new(),
            collected_at: chrono::Utc::now().to_rfc3339(),
            snapshot: None,
            schema_version: schema_version.to_string(),
        }
    }

    /// Add a data reference
    pub fn add_data_ref(mut self, name: &str, cid: &str) -> Self {
        self.data_cids.insert(name.to_string(), cid.to_string());
        self
    }

    /// Add multiple data references
    pub fn with_data_refs(mut self, refs: HashMap<String, String>) -> Self {
        self.data_cids.extend(refs);
        self
    }

    /// Set a snapshot of the input data
    pub fn with_snapshot(mut self, snapshot: serde_json::Value) -> Self {
        self.snapshot = Some(snapshot);
        self
    }

    /// Compute and set the combined hash of all inputs
    pub fn finalize(mut self) -> Self {
        let mut hasher = Sha256::new();

        // Sort keys for deterministic hashing
        let mut keys: Vec<_> = self.data_cids.keys().collect();
        keys.sort();

        for key in keys {
            if let Some(cid) = self.data_cids.get(key) {
                hasher.update(key.as_bytes());
                hasher.update(cid.as_bytes());
            }
        }

        // Include snapshot in hash if present
        if let Some(ref snapshot) = self.snapshot {
            if let Ok(json) = serde_json::to_string(snapshot) {
                hasher.update(json.as_bytes());
            }
        }

        self.combined_hash = format!("0x{}", hex::encode(hasher.finalize()));
        self
    }

    /// Verify the input hash is correct
    pub fn verify(&self) -> bool {
        let mut hasher = Sha256::new();

        let mut keys: Vec<_> = self.data_cids.keys().collect();
        keys.sort();

        for key in keys {
            if let Some(cid) = self.data_cids.get(key) {
                hasher.update(key.as_bytes());
                hasher.update(cid.as_bytes());
            }
        }

        if let Some(ref snapshot) = self.snapshot {
            if let Ok(json) = serde_json::to_string(snapshot) {
                hasher.update(json.as_bytes());
            }
        }

        let computed = format!("0x{}", hex::encode(hasher.finalize()));
        computed == self.combined_hash
    }
}

/// Output from AI scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiScoringOutput {
    /// Unique output ID
    pub output_id: String,
    /// Reference to input used
    pub input_id: String,
    /// The computed scores
    pub scores: HashMap<String, f64>,
    /// Confidence levels for each score
    pub confidence: HashMap<String, f64>,
    /// Explainability factors
    pub factors: Vec<ScoringFactor>,
    /// Hash of the output for verification
    pub output_hash: String,
    /// Timestamp of scoring
    pub scored_at: String,
    /// Processing duration in milliseconds
    pub processing_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringFactor {
    pub name: String,
    pub weight: f64,
    pub value: f64,
    pub contribution: f64,
    pub description: String,
}

impl AiScoringOutput {
    pub fn new(input_id: &str) -> Self {
        Self {
            output_id: uuid::Uuid::new_v4().to_string(),
            input_id: input_id.to_string(),
            scores: HashMap::new(),
            confidence: HashMap::new(),
            factors: Vec::new(),
            output_hash: String::new(),
            scored_at: chrono::Utc::now().to_rfc3339(),
            processing_time_ms: 0,
        }
    }

    pub fn add_score(mut self, name: &str, score: f64, confidence: f64) -> Self {
        self.scores.insert(name.to_string(), score);
        self.confidence.insert(name.to_string(), confidence);
        self
    }

    pub fn add_factor(mut self, factor: ScoringFactor) -> Self {
        self.factors.push(factor);
        self
    }

    pub fn with_processing_time(mut self, ms: u64) -> Self {
        self.processing_time_ms = ms;
        self
    }

    /// Compute and set the output hash
    pub fn finalize(mut self) -> Self {
        let mut hasher = Sha256::new();

        hasher.update(self.input_id.as_bytes());

        // Sort and hash scores
        let mut score_keys: Vec<_> = self.scores.keys().collect();
        score_keys.sort();

        for key in score_keys {
            if let Some(score) = self.scores.get(key) {
                hasher.update(key.as_bytes());
                hasher.update(score.to_le_bytes());
            }
        }

        // Hash factors
        for factor in &self.factors {
            hasher.update(factor.name.as_bytes());
            hasher.update(factor.value.to_le_bytes());
        }

        self.output_hash = format!("0x{}", hex::encode(hasher.finalize()));
        self
    }

    /// Get the primary score (first one or "quality_score" if present)
    pub fn primary_score(&self) -> Option<f64> {
        self.scores
            .get("quality_score")
            .or_else(|| self.scores.values().next())
            .copied()
    }
}

/// Complete AI scoring record for audit trail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiScoringRecord {
    /// Unique record ID
    pub record_id: String,
    /// Model metadata
    pub model: AiModelMetadata,
    /// Input data references
    pub input: AiScoringInput,
    /// Output scores and factors
    pub output: AiScoringOutput,
    /// IPFS CID where this record is stored
    pub record_cid: Option<String>,
    /// Entity being scored (e.g., batch_id, sku_id)
    pub entity_id: String,
    /// Entity type
    pub entity_type: String,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
    /// Created timestamp
    pub created_at: String,
}

impl AiScoringRecord {
    pub fn new(
        model: AiModelMetadata,
        input: AiScoringInput,
        output: AiScoringOutput,
        entity_id: &str,
        entity_type: &str,
    ) -> Self {
        Self {
            record_id: uuid::Uuid::new_v4().to_string(),
            model,
            input,
            output,
            record_cid: None,
            entity_id: entity_id.to_string(),
            entity_type: entity_type.to_string(),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn with_cid(mut self, cid: &str) -> Self {
        self.record_cid = Some(cid.to_string());
        self
    }

    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }

    /// Verify the integrity of this record
    pub fn verify_integrity(&self) -> IntegrityCheckResult {
        let mut issues = Vec::new();

        // Check input hash
        if !self.input.verify() {
            issues.push("Input hash verification failed".to_string());
        }

        // Check output references input
        if self.output.input_id != self.input.input_id {
            issues.push("Output input_id doesn't match input".to_string());
        }

        // Check model hash is present
        if self.model.hash.is_empty() {
            issues.push("Model hash is empty".to_string());
        }

        IntegrityCheckResult {
            is_valid: issues.is_empty(),
            issues,
            checked_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Result of an integrity check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityCheckResult {
    pub is_valid: bool,
    pub issues: Vec<String>,
    pub checked_at: String,
}

/// Report on reproducibility of AI scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReproducibilityReport {
    /// Original scoring record
    pub original_record_id: String,
    /// Whether the score is reproducible
    pub is_reproducible: bool,
    /// Original score
    pub original_score: f64,
    /// Reproduced score (if attempted)
    pub reproduced_score: Option<f64>,
    /// Score difference (absolute)
    pub score_difference: Option<f64>,
    /// Whether difference is within acceptable tolerance
    pub within_tolerance: bool,
    /// Tolerance used for comparison
    pub tolerance: f64,
    /// Issues found during reproduction
    pub issues: Vec<String>,
    /// Timestamp of report
    pub generated_at: String,
}

impl ReproducibilityReport {
    pub fn new(record_id: &str, original_score: f64, tolerance: f64) -> Self {
        Self {
            original_record_id: record_id.to_string(),
            is_reproducible: false,
            original_score,
            reproduced_score: None,
            score_difference: None,
            within_tolerance: false,
            tolerance,
            issues: Vec::new(),
            generated_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn with_reproduced_score(mut self, score: f64) -> Self {
        self.reproduced_score = Some(score);
        let diff = (score - self.original_score).abs();
        self.score_difference = Some(diff);
        self.within_tolerance = diff <= self.tolerance;
        self.is_reproducible = self.within_tolerance;
        self
    }

    pub fn add_issue(mut self, issue: &str) -> Self {
        self.issues.push(issue.to_string());
        self.is_reproducible = false;
        self
    }
}

/// Registry for AI models
pub struct ModelRegistry {
    models: Arc<RwLock<HashMap<String, AiModelMetadata>>>,
    active_versions: Arc<RwLock<HashMap<String, String>>>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self {
            models: Arc::new(RwLock::new(HashMap::new())),
            active_versions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a model
    pub async fn register(&self, model: AiModelMetadata) -> Result<(), String> {
        let model_id = model.model_id();
        let name = model.name.clone();
        let version = model.version.clone();

        let mut models = self.models.write().await;
        if models.contains_key(&model_id) {
            return Err(format!("Model {} already registered", model_id));
        }

        models.insert(model_id.clone(), model);
        drop(models);

        // Set as active if no active version exists
        let mut active = self.active_versions.write().await;
        active.entry(name.clone()).or_insert(version);

        info!(model_id = %model_id, "Registered AI model");
        Ok(())
    }

    /// Get a model by name (returns active version)
    pub async fn get(&self, name: &str) -> Option<AiModelMetadata> {
        let active = self.active_versions.read().await;
        let version = active.get(name)?;

        let models = self.models.read().await;
        for model in models.values() {
            if model.name == name && model.version == *version {
                return Some(model.clone());
            }
        }
        None
    }

    /// Get a specific model version
    pub async fn get_version(&self, name: &str, version: &str) -> Option<AiModelMetadata> {
        let models = self.models.read().await;
        for model in models.values() {
            if model.name == name && model.version == version {
                return Some(model.clone());
            }
        }
        None
    }

    /// Set the active version for a model
    pub async fn set_active_version(&self, name: &str, version: &str) -> Result<(), String> {
        // Verify version exists
        let exists = self.get_version(name, version).await.is_some();
        if !exists {
            return Err(format!("Model {}:{} not found", name, version));
        }

        let mut active = self.active_versions.write().await;
        active.insert(name.to_string(), version.to_string());
        info!(model = %name, version = %version, "Set active model version");
        Ok(())
    }

    /// List all registered models
    pub async fn list(&self) -> Vec<AiModelMetadata> {
        let models = self.models.read().await;
        models.values().cloned().collect()
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Verifier for AI scoring integrity
pub struct IntegrityVerifier {
    model_registry: Arc<ModelRegistry>,
    tolerance: f64,
}

impl IntegrityVerifier {
    pub fn new(model_registry: Arc<ModelRegistry>) -> Self {
        Self {
            model_registry,
            tolerance: 0.001, // Default tolerance of 0.1%
        }
    }

    pub fn with_tolerance(mut self, tolerance: f64) -> Self {
        self.tolerance = tolerance;
        self
    }

    /// Verify a scoring record's integrity
    pub async fn verify_record(&self, record: &AiScoringRecord) -> IntegrityCheckResult {
        let mut issues = Vec::new();

        // Verify input hash
        if !record.input.verify() {
            issues.push("Input data hash verification failed".to_string());
        }

        // Verify model exists in registry
        let model = self
            .model_registry
            .get_version(&record.model.name, &record.model.version)
            .await;

        match model {
            Some(registered) => {
                if registered.hash != record.model.hash {
                    issues.push(format!(
                        "Model hash mismatch: record has {}, registry has {}",
                        &record.model.hash[..16.min(record.model.hash.len())],
                        &registered.hash[..16.min(registered.hash.len())]
                    ));
                }
            }
            None => {
                issues.push(format!(
                    "Model {}:{} not found in registry",
                    record.model.name, record.model.version
                ));
            }
        }

        // Verify output references correct input
        if record.output.input_id != record.input.input_id {
            issues.push("Output references different input than record contains".to_string());
        }

        // Check for required data references
        if record.input.data_cids.is_empty() && record.input.snapshot.is_none() {
            issues.push("No input data references or snapshot provided".to_string());
        }

        IntegrityCheckResult {
            is_valid: issues.is_empty(),
            issues,
            checked_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Generate a reproducibility report (without actually re-running the model)
    pub fn check_reproducibility_prerequisites(
        &self,
        record: &AiScoringRecord,
    ) -> ReproducibilityReport {
        let primary_score = record.output.primary_score().unwrap_or(0.0);
        let mut report = ReproducibilityReport::new(&record.record_id, primary_score, self.tolerance);

        // Check if we have all necessary data for reproduction
        if record.model.weights_cid.is_none() {
            report = report.add_issue("Model weights CID not available for reproduction");
        }

        if record.input.data_cids.is_empty() && record.input.snapshot.is_none() {
            report = report.add_issue("No input data available for reproduction");
        }

        if record.model.hash.is_empty() {
            report = report.add_issue("Model hash not available for verification");
        }

        // If no issues, mark as reproducible (prerequisites met)
        if report.issues.is_empty() {
            report.is_reproducible = true;
        }

        report
    }
}

/// Compute SHA256 hash of model weights and config
pub fn compute_model_hash(weights: &[u8], config: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(weights);
    hasher.update(config);
    format!("0x{}", hex::encode(hasher.finalize()))
}

/// Compute hash of arbitrary data
pub fn compute_data_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("0x{}", hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_metadata() {
        let model = AiModelMetadata::new("quality_model", "1.0.0", "pytorch")
            .with_hash("0x123456789abcdef");

        assert_eq!(model.name, "quality_model");
        assert_eq!(model.version, "1.0.0");
        assert!(model.model_id().contains("quality_model"));
    }

    #[test]
    fn test_model_hash_verification() {
        let weights = b"model weights data";
        let config = b"model config data";
        let hash = compute_model_hash(weights, config);

        let model = AiModelMetadata::new("test", "1.0", "test").with_hash(&hash);

        assert!(model.verify_hash(weights, config));
        assert!(!model.verify_hash(b"wrong", config));
    }

    #[test]
    fn test_scoring_input() {
        let input = AiScoringInput::new("1.0.0")
            .add_data_ref("farmer_data", "QmFarmerCid123")
            .add_data_ref("warehouse_data", "QmWarehouseCid456")
            .finalize();

        assert!(!input.combined_hash.is_empty());
        assert!(input.verify());
    }

    #[test]
    fn test_scoring_output() {
        let output = AiScoringOutput::new("input-123")
            .add_score("quality_score", 85.5, 0.95)
            .add_score("trust_score", 92.0, 0.88)
            .add_factor(ScoringFactor {
                name: "storage".to_string(),
                weight: 0.3,
                value: 90.0,
                contribution: 27.0,
                description: "Good storage conditions".to_string(),
            })
            .with_processing_time(150)
            .finalize();

        assert_eq!(output.primary_score(), Some(85.5));
        assert!(!output.output_hash.is_empty());
    }

    #[test]
    fn test_scoring_record_integrity() {
        let model = AiModelMetadata::new("test", "1.0", "pytorch").with_hash("0xabc123");

        let input = AiScoringInput::new("1.0.0")
            .add_data_ref("data", "QmTestCid")
            .finalize();

        let output = AiScoringOutput::new(&input.input_id)
            .add_score("quality_score", 80.0, 0.9)
            .finalize();

        let record = AiScoringRecord::new(model, input, output, "BATCH-001", "batch");

        let result = record.verify_integrity();
        assert!(result.is_valid);
    }

    #[test]
    fn test_reproducibility_report() {
        let report = ReproducibilityReport::new("record-123", 85.0, 0.5)
            .with_reproduced_score(85.3);

        assert!(report.is_reproducible);
        assert!(report.within_tolerance);
        assert_eq!(report.score_difference, Some(0.3));
    }

    #[test]
    fn test_reproducibility_out_of_tolerance() {
        let report = ReproducibilityReport::new("record-123", 85.0, 0.1)
            .with_reproduced_score(86.0);

        assert!(!report.is_reproducible);
        assert!(!report.within_tolerance);
    }

    #[tokio::test]
    async fn test_model_registry() {
        let registry = ModelRegistry::new();

        let model = AiModelMetadata::new("quality_model", "1.0.0", "pytorch")
            .with_hash("0xhash123");

        registry.register(model.clone()).await.unwrap();

        let retrieved = registry.get("quality_model").await.unwrap();
        assert_eq!(retrieved.version, "1.0.0");
    }

    #[tokio::test]
    async fn test_model_registry_versions() {
        let registry = ModelRegistry::new();

        let v1 = AiModelMetadata::new("model", "1.0.0", "pytorch").with_hash("0xhash1");
        let v2 = AiModelMetadata::new("model", "2.0.0", "pytorch").with_hash("0xhash2");

        registry.register(v1).await.unwrap();
        registry.register(v2).await.unwrap();

        // Default should be v1 (first registered)
        let active = registry.get("model").await.unwrap();
        assert_eq!(active.version, "1.0.0");

        // Set v2 as active
        registry.set_active_version("model", "2.0.0").await.unwrap();

        let active = registry.get("model").await.unwrap();
        assert_eq!(active.version, "2.0.0");
    }

    #[tokio::test]
    async fn test_integrity_verifier() {
        let registry = Arc::new(ModelRegistry::new());
        let verifier = IntegrityVerifier::new(registry.clone());

        let model = AiModelMetadata::new("test", "1.0", "pytorch").with_hash("0xhash");
        registry.register(model.clone()).await.unwrap();

        let input = AiScoringInput::new("1.0.0")
            .add_data_ref("data", "QmCid")
            .finalize();

        let output = AiScoringOutput::new(&input.input_id)
            .add_score("score", 90.0, 0.95)
            .finalize();

        let record = AiScoringRecord::new(model, input, output, "entity-1", "batch");

        let result = verifier.verify_record(&record).await;
        assert!(result.is_valid);
    }

    #[test]
    fn test_compute_hashes() {
        let hash1 = compute_data_hash(b"test data");
        let hash2 = compute_data_hash(b"test data");
        let hash3 = compute_data_hash(b"different data");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert!(hash1.starts_with("0x"));
    }
}
