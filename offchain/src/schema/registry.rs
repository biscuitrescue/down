//! Schema Registry
//!
//! Manages schema definitions stored on IPFS with version control.
//! Provides schema retrieval, caching, and lineage tracking.

use super::{SchemaDefinition, SchemaReference, SupplyChainStage};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Schema registry entry with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaRegistryEntry {
    /// The schema definition
    pub definition: SchemaDefinition,
    /// IPFS CID where this schema is stored
    pub cid: String,
    /// When this schema was registered
    pub registered_at: String,
    /// Whether this is the current active version
    pub is_active: bool,
    /// Usage statistics
    pub usage_count: u64,
}

/// Schema version history for lineage tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaVersionHistory {
    pub schema_name: String,
    pub versions: Vec<SchemaVersionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaVersionEntry {
    pub version: String,
    pub cid: String,
    pub registered_at: String,
    pub changelog: Option<String>,
    pub is_deprecated: bool,
}

/// In-memory cache for schema definitions
type SchemaCache = Arc<RwLock<HashMap<String, SchemaRegistryEntry>>>;

/// Schema Registry for managing versioned schema definitions
#[derive(Clone)]
pub struct SchemaRegistry {
    /// Cache of schema definitions (key: "schema_name:version")
    cache: SchemaCache,
    /// Active schema versions (key: schema_name, value: version)
    active_versions: Arc<RwLock<HashMap<String, String>>>,
    /// Version history for each schema
    version_history: Arc<RwLock<HashMap<String, SchemaVersionHistory>>>,
    /// Whether to auto-load built-in schemas
    auto_load_builtin: bool,
}

impl SchemaRegistry {
    /// Create a new schema registry
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            active_versions: Arc::new(RwLock::new(HashMap::new())),
            version_history: Arc::new(RwLock::new(HashMap::new())),
            auto_load_builtin: true,
        }
    }

    /// Create a registry without auto-loading built-in schemas
    pub fn empty() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            active_versions: Arc::new(RwLock::new(HashMap::new())),
            version_history: Arc::new(RwLock::new(HashMap::new())),
            auto_load_builtin: false,
        }
    }

    /// Initialize the registry with built-in schemas
    pub async fn initialize(&self) -> Result<()> {
        if self.auto_load_builtin {
            self.load_builtin_schemas().await?;
        }
        Ok(())
    }

    /// Load all built-in schema definitions
    async fn load_builtin_schemas(&self) -> Result<()> {
        use super::definitions::get_all_schemas;

        info!("Loading built-in schema definitions");

        for schema_def in get_all_schemas() {
            let cache_key = format!("{}:{}", schema_def.schema.name, schema_def.schema.version);

            // For built-in schemas, we generate a placeholder CID
            // In production, these would be uploaded to IPFS and the actual CID stored
            let placeholder_cid = format!(
                "builtin:{}:{}",
                schema_def.schema.name, schema_def.schema.version
            );

            let entry = SchemaRegistryEntry {
                definition: schema_def.clone(),
                cid: placeholder_cid.clone(),
                registered_at: chrono::Utc::now().to_rfc3339(),
                is_active: true,
                usage_count: 0,
            };

            // Add to cache
            {
                let mut cache = self.cache.write().await;
                cache.insert(cache_key, entry);
            }

            // Set as active version
            {
                let mut active = self.active_versions.write().await;
                active.insert(
                    schema_def.schema.name.clone(),
                    schema_def.schema.version.clone(),
                );
            }

            // Add to version history
            {
                let mut history = self.version_history.write().await;
                let schema_history = history
                    .entry(schema_def.schema.name.clone())
                    .or_insert_with(|| SchemaVersionHistory {
                        schema_name: schema_def.schema.name.clone(),
                        versions: Vec::new(),
                    });

                schema_history.versions.push(SchemaVersionEntry {
                    version: schema_def.schema.version.clone(),
                    cid: placeholder_cid,
                    registered_at: chrono::Utc::now().to_rfc3339(),
                    changelog: schema_def.changelog.clone(),
                    is_deprecated: false,
                });
            }

            debug!(
                schema = %schema_def.schema.name,
                version = %schema_def.schema.version,
                "Loaded built-in schema"
            );
        }

        Ok(())
    }

    /// Get a schema definition by stage (uses active version)
    pub async fn get_schema(&self, stage: SupplyChainStage) -> Result<SchemaRegistryEntry> {
        let schema_name = stage.schema_name();
        self.get_schema_by_name(schema_name).await
    }

    /// Get a schema definition by name (uses active version)
    pub async fn get_schema_by_name(&self, name: &str) -> Result<SchemaRegistryEntry> {
        let active_versions = self.active_versions.read().await;
        let version = active_versions
            .get(name)
            .context(format!("No active version found for schema '{}'", name))?;

        self.get_schema_version(name, version).await
    }

    /// Get a specific version of a schema
    pub async fn get_schema_version(&self, name: &str, version: &str) -> Result<SchemaRegistryEntry> {
        let cache_key = format!("{}:{}", name, version);

        let cache = self.cache.read().await;
        let entry = cache
            .get(&cache_key)
            .context(format!(
                "Schema '{}' version '{}' not found in registry",
                name, version
            ))?
            .clone();

        // Update usage count (in a real implementation, this would be atomic)
        drop(cache);
        {
            let mut cache = self.cache.write().await;
            if let Some(entry) = cache.get_mut(&cache_key) {
                entry.usage_count += 1;
            }
        }

        Ok(entry)
    }

    /// Register a new schema version
    pub async fn register_schema(
        &self,
        definition: SchemaDefinition,
        cid: String,
        set_active: bool,
    ) -> Result<()> {
        let cache_key = format!("{}:{}", definition.schema.name, definition.schema.version);

        info!(
            schema = %definition.schema.name,
            version = %definition.schema.version,
            cid = %cid,
            "Registering new schema version"
        );

        let entry = SchemaRegistryEntry {
            definition: definition.clone(),
            cid: cid.clone(),
            registered_at: chrono::Utc::now().to_rfc3339(),
            is_active: set_active,
            usage_count: 0,
        };

        // Add to cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(cache_key, entry);
        }

        // Update active version if requested
        if set_active {
            let mut active = self.active_versions.write().await;
            active.insert(
                definition.schema.name.clone(),
                definition.schema.version.clone(),
            );
        }

        // Add to version history
        {
            let mut history = self.version_history.write().await;
            let schema_history = history
                .entry(definition.schema.name.clone())
                .or_insert_with(|| SchemaVersionHistory {
                    schema_name: definition.schema.name.clone(),
                    versions: Vec::new(),
                });

            schema_history.versions.push(SchemaVersionEntry {
                version: definition.schema.version.clone(),
                cid,
                registered_at: chrono::Utc::now().to_rfc3339(),
                changelog: definition.changelog.clone(),
                is_deprecated: false,
            });
        }

        Ok(())
    }

    /// Set a specific version as the active version for a schema
    pub async fn set_active_version(&self, name: &str, version: &str) -> Result<()> {
        let cache_key = format!("{}:{}", name, version);

        // Verify the version exists
        {
            let cache = self.cache.read().await;
            if !cache.contains_key(&cache_key) {
                anyhow::bail!(
                    "Cannot set active version: schema '{}' version '{}' not found",
                    name,
                    version
                );
            }
        }

        // Update cache entries
        {
            let mut cache = self.cache.write().await;
            for (key, entry) in cache.iter_mut() {
                if key.starts_with(&format!("{}:", name)) {
                    entry.is_active = key == &cache_key;
                }
            }
        }

        // Update active versions map
        {
            let mut active = self.active_versions.write().await;
            active.insert(name.to_string(), version.to_string());
        }

        info!(
            schema = %name,
            version = %version,
            "Set active schema version"
        );

        Ok(())
    }

    /// Deprecate a schema version
    pub async fn deprecate_version(&self, name: &str, version: &str) -> Result<()> {
        let mut history = self.version_history.write().await;

        if let Some(schema_history) = history.get_mut(name) {
            for v in &mut schema_history.versions {
                if v.version == version {
                    v.is_deprecated = true;
                    warn!(
                        schema = %name,
                        version = %version,
                        "Deprecated schema version"
                    );
                    return Ok(());
                }
            }
        }

        anyhow::bail!(
            "Schema '{}' version '{}' not found for deprecation",
            name,
            version
        )
    }

    /// Get the version history for a schema
    pub async fn get_version_history(&self, name: &str) -> Result<SchemaVersionHistory> {
        let history = self.version_history.read().await;
        history
            .get(name)
            .cloned()
            .context(format!("No version history found for schema '{}'", name))
    }

    /// Get all registered schemas
    pub async fn list_schemas(&self) -> Vec<SchemaRegistryEntry> {
        let cache = self.cache.read().await;
        cache.values().cloned().collect()
    }

    /// Get all active schema references
    pub async fn get_active_schema_refs(&self) -> HashMap<String, SchemaReference> {
        let active = self.active_versions.read().await;
        let cache = self.cache.read().await;

        let mut refs = HashMap::new();
        for (name, version) in active.iter() {
            let cache_key = format!("{}:{}", name, version);
            if let Some(entry) = cache.get(&cache_key) {
                refs.insert(
                    name.clone(),
                    SchemaReference::with_cid(name, version, &entry.cid),
                );
            }
        }
        refs
    }

    /// Check if a schema version exists
    pub async fn schema_exists(&self, name: &str, version: &str) -> bool {
        let cache_key = format!("{}:{}", name, version);
        let cache = self.cache.read().await;
        cache.contains_key(&cache_key)
    }

    /// Get schema CID for a specific version
    pub async fn get_schema_cid(&self, name: &str, version: &str) -> Result<String> {
        let entry = self.get_schema_version(name, version).await?;
        Ok(entry.cid)
    }

    /// Resolve a schema reference to its full definition
    pub async fn resolve_schema_ref(&self, schema_ref: &SchemaReference) -> Result<SchemaRegistryEntry> {
        self.get_schema_version(&schema_ref.name, &schema_ref.version).await
    }

    /// Validate that a document's schema reference can be resolved
    pub async fn validate_schema_ref(&self, schema_ref: &SchemaReference) -> Result<bool> {
        match self.resolve_schema_ref(schema_ref).await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Get statistics about the registry
    pub async fn get_stats(&self) -> RegistryStats {
        let cache = self.cache.read().await;
        let active = self.active_versions.read().await;

        let total_schemas = active.len();
        let total_versions = cache.len();
        let total_usage: u64 = cache.values().map(|e| e.usage_count).sum();

        RegistryStats {
            total_schemas,
            total_versions,
            total_usage,
            active_versions: active.clone(),
        }
    }
}

impl Default for SchemaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about the schema registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryStats {
    pub total_schemas: usize,
    pub total_versions: usize,
    pub total_usage: u64,
    pub active_versions: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::definitions::get_schema_definition;

    #[tokio::test]
    async fn test_registry_initialization() {
        let registry = SchemaRegistry::new();
        registry.initialize().await.unwrap();

        let stats = registry.get_stats().await;
        assert!(stats.total_schemas > 0);
        assert!(stats.total_versions > 0);
    }

    #[tokio::test]
    async fn test_get_schema_by_stage() {
        let registry = SchemaRegistry::new();
        registry.initialize().await.unwrap();

        let entry = registry
            .get_schema(SupplyChainStage::FarmerRegistration)
            .await
            .unwrap();

        assert_eq!(entry.definition.schema.name, "farmer_registration");
        assert!(entry.is_active);
    }

    #[tokio::test]
    async fn test_register_new_schema() {
        let registry = SchemaRegistry::empty();

        let schema_def = get_schema_definition(SupplyChainStage::FarmerRegistration);
        let cid = "QmTestCID12345".to_string();

        registry
            .register_schema(schema_def.clone(), cid.clone(), true)
            .await
            .unwrap();

        let entry = registry
            .get_schema_by_name("farmer_registration")
            .await
            .unwrap();

        assert_eq!(entry.cid, cid);
        assert!(entry.is_active);
    }

    #[tokio::test]
    async fn test_version_history() {
        let registry = SchemaRegistry::empty();

        let mut schema_v1 = get_schema_definition(SupplyChainStage::FarmerRegistration);
        schema_v1.schema.version = "1.0.0".to_string();
        schema_v1.changelog = Some("Initial version".to_string());

        let mut schema_v1_1 = get_schema_definition(SupplyChainStage::FarmerRegistration);
        schema_v1_1.schema.version = "1.1.0".to_string();
        schema_v1_1.changelog = Some("Added optional fields".to_string());

        registry
            .register_schema(schema_v1, "QmV1CID".to_string(), true)
            .await
            .unwrap();

        registry
            .register_schema(schema_v1_1, "QmV1_1CID".to_string(), true)
            .await
            .unwrap();

        let history = registry
            .get_version_history("farmer_registration")
            .await
            .unwrap();

        assert_eq!(history.versions.len(), 2);
        assert_eq!(history.versions[0].version, "1.0.0");
        assert_eq!(history.versions[1].version, "1.1.0");
    }

    #[tokio::test]
    async fn test_set_active_version() {
        let registry = SchemaRegistry::empty();

        let mut schema_v1 = get_schema_definition(SupplyChainStage::FarmerRegistration);
        schema_v1.schema.version = "1.0.0".to_string();

        let mut schema_v1_1 = get_schema_definition(SupplyChainStage::FarmerRegistration);
        schema_v1_1.schema.version = "1.1.0".to_string();

        registry
            .register_schema(schema_v1, "QmV1CID".to_string(), true)
            .await
            .unwrap();

        registry
            .register_schema(schema_v1_1, "QmV1_1CID".to_string(), false)
            .await
            .unwrap();

        // v1.0.0 should be active
        let entry = registry
            .get_schema_by_name("farmer_registration")
            .await
            .unwrap();
        assert_eq!(entry.definition.schema.version, "1.0.0");

        // Switch to v1.1.0
        registry
            .set_active_version("farmer_registration", "1.1.0")
            .await
            .unwrap();

        let entry = registry
            .get_schema_by_name("farmer_registration")
            .await
            .unwrap();
        assert_eq!(entry.definition.schema.version, "1.1.0");
    }

    #[tokio::test]
    async fn test_deprecate_version() {
        let registry = SchemaRegistry::empty();

        let schema = get_schema_definition(SupplyChainStage::FarmerRegistration);

        registry
            .register_schema(schema, "QmTestCID".to_string(), true)
            .await
            .unwrap();

        registry
            .deprecate_version("farmer_registration", "1.0.0")
            .await
            .unwrap();

        let history = registry
            .get_version_history("farmer_registration")
            .await
            .unwrap();

        assert!(history.versions[0].is_deprecated);
    }

    #[tokio::test]
    async fn test_schema_exists() {
        let registry = SchemaRegistry::new();
        registry.initialize().await.unwrap();

        assert!(registry.schema_exists("farmer_registration", "1.0.0").await);
        assert!(!registry.schema_exists("nonexistent", "1.0.0").await);
        assert!(!registry.schema_exists("farmer_registration", "99.0.0").await);
    }

    #[tokio::test]
    async fn test_get_active_schema_refs() {
        let registry = SchemaRegistry::new();
        registry.initialize().await.unwrap();

        let refs = registry.get_active_schema_refs().await;

        assert!(refs.contains_key("farmer_registration"));
        assert!(refs.contains_key("fpo_purchase"));
        assert!(refs.contains_key("warehouse_storage"));

        let farmer_ref = refs.get("farmer_registration").unwrap();
        assert_eq!(farmer_ref.name, "farmer_registration");
        assert!(!farmer_ref.version.is_empty());
    }
}
