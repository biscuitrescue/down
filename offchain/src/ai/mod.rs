//! AI Module
//!
//! Provides AI scoring integrity and reproducibility features:
//! - Model hash verification
//! - Input data snapshotting
//! - Score reproducibility tracking
//! - Audit trail generation

pub mod integrity;

pub use integrity::{
    AiModelMetadata, AiScoringInput, AiScoringOutput, AiScoringRecord, IntegrityVerifier,
    ModelRegistry, ReproducibilityReport,
};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// AI scoring stage types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiScoringType {
    /// Quality assessment score
    QualityScore,
    /// Trust/reliability score
    TrustScore,
    /// Fraud risk score
    FraudRisk,
    /// Price prediction
    PricePrediction,
    /// Yield estimation
    YieldEstimation,
}

impl AiScoringType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::QualityScore => "quality_score",
            Self::TrustScore => "trust_score",
            Self::FraudRisk => "fraud_risk",
            Self::PricePrediction => "price_prediction",
            Self::YieldEstimation => "yield_estimation",
        }
    }
}

/// Explainability factor for AI decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainabilityFactor {
    /// Factor name (e.g., "storage_conditions", "source_verification")
    pub name: String,
    /// Weight/importance of this factor (0.0 - 1.0)
    pub weight: f64,
    /// Contribution to the final score
    pub contribution: f64,
    /// Human-readable explanation
    pub explanation: String,
    /// Raw value used for this factor
    pub raw_value: Option<serde_json::Value>,
}

/// Complete AI analysis result with explainability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiAnalysisResult {
    /// Type of scoring performed
    pub scoring_type: AiScoringType,
    /// The computed score (typically 0-100)
    pub score: f64,
    /// Confidence level (0.0 - 1.0)
    pub confidence: f64,
    /// Breakdown of factors
    pub factors: Vec<ExplainabilityFactor>,
    /// Detailed explanation
    pub explanation: String,
    /// Recommendations based on the score
    pub recommendations: Vec<String>,
    /// Warnings or concerns
    pub warnings: Vec<String>,
    /// Model metadata used for this analysis
    pub model: AiModelMetadata,
    /// Timestamp of analysis
    pub analyzed_at: String,
    /// Processing time in milliseconds
    pub processing_time_ms: u64,
}

impl AiAnalysisResult {
    /// Create a new analysis result
    pub fn new(scoring_type: AiScoringType, score: f64, model: AiModelMetadata) -> Self {
        Self {
            scoring_type,
            score,
            confidence: 1.0,
            factors: Vec::new(),
            explanation: String::new(),
            recommendations: Vec::new(),
            warnings: Vec::new(),
            model,
            analyzed_at: chrono::Utc::now().to_rfc3339(),
            processing_time_ms: 0,
        }
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    pub fn with_factor(mut self, factor: ExplainabilityFactor) -> Self {
        self.factors.push(factor);
        self
    }

    pub fn with_explanation(mut self, explanation: &str) -> Self {
        self.explanation = explanation.to_string();
        self
    }

    pub fn with_recommendation(mut self, recommendation: &str) -> Self {
        self.recommendations.push(recommendation.to_string());
        self
    }

    pub fn with_warning(mut self, warning: &str) -> Self {
        self.warnings.push(warning.to_string());
        self
    }

    /// Generate a summary string
    pub fn summary(&self) -> String {
        format!(
            "{}: {:.1} (confidence: {:.0}%)",
            self.scoring_type.as_str(),
            self.score,
            self.confidence * 100.0
        )
    }

    /// Check if the score is above a threshold
    pub fn is_above_threshold(&self, threshold: f64) -> bool {
        self.score >= threshold
    }

    /// Get the primary contributing factor
    pub fn primary_factor(&self) -> Option<&ExplainabilityFactor> {
        self.factors
            .iter()
            .max_by(|a, b| a.contribution.partial_cmp(&b.contribution).unwrap())
    }
}

/// Builder for creating AI analysis results
pub struct AiAnalysisBuilder {
    result: AiAnalysisResult,
    start_time: std::time::Instant,
}

impl AiAnalysisBuilder {
    pub fn new(scoring_type: AiScoringType, model: AiModelMetadata) -> Self {
        Self {
            result: AiAnalysisResult::new(scoring_type, 0.0, model),
            start_time: std::time::Instant::now(),
        }
    }

    pub fn score(mut self, score: f64) -> Self {
        self.result.score = score;
        self
    }

    pub fn confidence(mut self, confidence: f64) -> Self {
        self.result.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    pub fn add_factor(
        mut self,
        name: &str,
        weight: f64,
        contribution: f64,
        explanation: &str,
    ) -> Self {
        self.result.factors.push(ExplainabilityFactor {
            name: name.to_string(),
            weight,
            contribution,
            explanation: explanation.to_string(),
            raw_value: None,
        });
        self
    }

    pub fn add_factor_with_value(
        mut self,
        name: &str,
        weight: f64,
        contribution: f64,
        explanation: &str,
        raw_value: serde_json::Value,
    ) -> Self {
        self.result.factors.push(ExplainabilityFactor {
            name: name.to_string(),
            weight,
            contribution,
            explanation: explanation.to_string(),
            raw_value: Some(raw_value),
        });
        self
    }

    pub fn explanation(mut self, explanation: &str) -> Self {
        self.result.explanation = explanation.to_string();
        self
    }

    pub fn add_recommendation(mut self, recommendation: &str) -> Self {
        self.result.recommendations.push(recommendation.to_string());
        self
    }

    pub fn add_warning(mut self, warning: &str) -> Self {
        self.result.warnings.push(warning.to_string());
        self
    }

    pub fn build(mut self) -> AiAnalysisResult {
        self.result.processing_time_ms = self.start_time.elapsed().as_millis() as u64;
        self.result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scoring_type_as_str() {
        assert_eq!(AiScoringType::QualityScore.as_str(), "quality_score");
        assert_eq!(AiScoringType::TrustScore.as_str(), "trust_score");
        assert_eq!(AiScoringType::FraudRisk.as_str(), "fraud_risk");
    }

    #[test]
    fn test_analysis_result_builder() {
        let model = AiModelMetadata {
            name: "test_model".to_string(),
            version: "1.0.0".to_string(),
            hash: "abc123".to_string(),
            framework: "pytorch".to_string(),
            training_date: "2024-01-01".to_string(),
            input_schema_cid: None,
            weights_cid: None,
        };

        let result = AiAnalysisBuilder::new(AiScoringType::QualityScore, model)
            .score(85.5)
            .confidence(0.95)
            .add_factor("storage", 0.3, 25.0, "Good storage conditions")
            .add_factor("source", 0.4, 35.0, "Verified source")
            .explanation("Overall good quality based on storage and source verification")
            .add_recommendation("Continue current storage practices")
            .build();

        assert_eq!(result.score, 85.5);
        assert_eq!(result.confidence, 0.95);
        assert_eq!(result.factors.len(), 2);
        assert!(!result.explanation.is_empty());
    }

    #[test]
    fn test_analysis_result_summary() {
        let model = AiModelMetadata {
            name: "test".to_string(),
            version: "1.0".to_string(),
            hash: "hash".to_string(),
            framework: "test".to_string(),
            training_date: "2024-01-01".to_string(),
            input_schema_cid: None,
            weights_cid: None,
        };

        let result = AiAnalysisResult::new(AiScoringType::TrustScore, 92.0, model)
            .with_confidence(0.88);

        let summary = result.summary();
        assert!(summary.contains("trust_score"));
        assert!(summary.contains("92.0"));
        assert!(summary.contains("88%"));
    }

    #[test]
    fn test_threshold_check() {
        let model = AiModelMetadata {
            name: "test".to_string(),
            version: "1.0".to_string(),
            hash: "hash".to_string(),
            framework: "test".to_string(),
            training_date: "2024-01-01".to_string(),
            input_schema_cid: None,
            weights_cid: None,
        };

        let result = AiAnalysisResult::new(AiScoringType::FraudRisk, 25.0, model);

        assert!(result.is_above_threshold(20.0));
        assert!(!result.is_above_threshold(30.0));
    }

    #[test]
    fn test_primary_factor() {
        let model = AiModelMetadata {
            name: "test".to_string(),
            version: "1.0".to_string(),
            hash: "hash".to_string(),
            framework: "test".to_string(),
            training_date: "2024-01-01".to_string(),
            input_schema_cid: None,
            weights_cid: None,
        };

        let result = AiAnalysisBuilder::new(AiScoringType::QualityScore, model)
            .add_factor("low", 0.2, 10.0, "Low impact")
            .add_factor("high", 0.5, 50.0, "High impact")
            .add_factor("medium", 0.3, 25.0, "Medium impact")
            .build();

        let primary = result.primary_factor().unwrap();
        assert_eq!(primary.name, "high");
        assert_eq!(primary.contribution, 50.0);
    }
}
