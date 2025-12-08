//! Event Indexer for CID → Entity Mapping
//!
//! This module provides:
//! - Blockchain event listening and indexing
//! - CID to entity (SKU, batch, farmer) mapping
//! - Queryable database for supply chain traceability
//! - Full chain traversal from SKU back to farmer
//!
//! Without this, frontend can't efficiently query "give me all CIDs for SKU X"

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

// ============================================================================
// DATA TYPES
// ============================================================================

/// Supply chain entity types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Farmer,
    Batch,
    Warehouse,
    Shipment,
    Processing,
    Sku,
    AiScore,
    FraudReport,
}

impl EntityType {
    pub fn from_event_name(event: &str) -> Option<Self> {
        match event {
            "FarmerRegistered" => Some(Self::Farmer),
            "OwnershipTransfer" => Some(Self::Batch),
            "WarehouseStateUpdated" => Some(Self::Warehouse),
            "LogisticsMilestone" => Some(Self::Shipment),
            "BatchProcessed" => Some(Self::Processing),
            "SKUPackaged" => Some(Self::Sku),
            "AIScoreRevealed" => Some(Self::AiScore),
            "FraudDetected" => Some(Self::FraudReport),
            _ => None,
        }
    }
}

/// Indexed record linking CID to entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedRecord {
    /// Unique record ID
    pub id: String,
    /// Entity type
    pub entity_type: EntityType,
    /// Entity identifier (farmer_did, batch_hash, sku_id, etc.)
    pub entity_id: String,
    /// IPFS CID of the metadata
    pub cid: String,
    /// Block number where this was recorded
    pub block_number: u64,
    /// Transaction hash
    pub tx_hash: String,
    /// Timestamp from block
    pub timestamp: u64,
    /// Event name that created this record
    pub event_name: String,
    /// Parent entity ID (for traceability chain)
    pub parent_id: Option<String>,
    /// Additional indexed fields
    pub extra: HashMap<String, String>,
}

/// Query filters for searching records
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryFilter {
    pub entity_type: Option<EntityType>,
    pub entity_id: Option<String>,
    pub cid: Option<String>,
    pub parent_id: Option<String>,
    pub from_block: Option<u64>,
    pub to_block: Option<u64>,
    pub event_name: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Supply chain trace - full path from SKU to farmer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplyChainTrace {
    pub sku_id: String,
    pub stages: Vec<TraceStage>,
    pub farmer_did: Option<String>,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStage {
    pub stage: String,
    pub entity_id: String,
    pub cid: String,
    pub timestamp: u64,
    pub block_number: u64,
}

// ============================================================================
// IN-MEMORY INDEX (can be swapped for PostgreSQL)
// ============================================================================

/// Simple in-memory index store
/// For production, replace with PostgreSQL implementation
#[derive(Default)]
pub struct MemoryIndexStore {
    /// Records by ID
    records: HashMap<String, IndexedRecord>,
    /// Index: entity_id -> record IDs
    by_entity: HashMap<String, Vec<String>>,
    /// Index: CID -> record IDs
    by_cid: HashMap<String, Vec<String>>,
    /// Index: entity_type -> record IDs
    by_type: HashMap<EntityType, Vec<String>>,
    /// Index: parent_id -> record IDs (for traceability)
    by_parent: HashMap<String, Vec<String>>,
    /// Last indexed block
    last_block: u64,
}

impl MemoryIndexStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a new record
    pub fn insert(&mut self, record: IndexedRecord) {
        let id = record.id.clone();

        // Update indexes
        self.by_entity
            .entry(record.entity_id.clone())
            .or_default()
            .push(id.clone());

        self.by_cid
            .entry(record.cid.clone())
            .or_default()
            .push(id.clone());

        self.by_type
            .entry(record.entity_type)
            .or_default()
            .push(id.clone());

        if let Some(ref parent) = record.parent_id {
            self.by_parent
                .entry(parent.clone())
                .or_default()
                .push(id.clone());
        }

        // Update last block
        if record.block_number > self.last_block {
            self.last_block = record.block_number;
        }

        // Store record
        self.records.insert(id, record);
    }

    /// Query records with filters
    pub fn query(&self, filter: &QueryFilter) -> Vec<&IndexedRecord> {
        let mut results: Vec<&IndexedRecord> = self.records.values().collect();

        // Apply filters
        if let Some(ref entity_type) = filter.entity_type {
            results.retain(|r| r.entity_type == *entity_type);
        }

        if let Some(ref entity_id) = filter.entity_id {
            results.retain(|r| r.entity_id == *entity_id);
        }

        if let Some(ref cid) = filter.cid {
            results.retain(|r| r.cid == *cid);
        }

        if let Some(ref parent_id) = filter.parent_id {
            results.retain(|r| r.parent_id.as_ref() == Some(parent_id));
        }

        if let Some(from_block) = filter.from_block {
            results.retain(|r| r.block_number >= from_block);
        }

        if let Some(to_block) = filter.to_block {
            results.retain(|r| r.block_number <= to_block);
        }

        if let Some(ref event_name) = filter.event_name {
            results.retain(|r| r.event_name == *event_name);
        }

        // Sort by block number descending
        results.sort_by(|a, b| b.block_number.cmp(&a.block_number));

        // Apply pagination
        let offset = filter.offset.unwrap_or(0);
        let limit = filter.limit.unwrap_or(100);

        results.into_iter().skip(offset).take(limit).collect()
    }

    /// Get record by ID
    pub fn get(&self, id: &str) -> Option<&IndexedRecord> {
        self.records.get(id)
    }

    /// Get all records for an entity
    pub fn get_by_entity(&self, entity_id: &str) -> Vec<&IndexedRecord> {
        self.by_entity
            .get(entity_id)
            .map(|ids| ids.iter().filter_map(|id| self.records.get(id)).collect())
            .unwrap_or_default()
    }

    /// Get record by CID
    pub fn get_by_cid(&self, cid: &str) -> Vec<&IndexedRecord> {
        self.by_cid
            .get(cid)
            .map(|ids| ids.iter().filter_map(|id| self.records.get(id)).collect())
            .unwrap_or_default()
    }

    /// Get children of a parent entity
    pub fn get_children(&self, parent_id: &str) -> Vec<&IndexedRecord> {
        self.by_parent
            .get(parent_id)
            .map(|ids| ids.iter().filter_map(|id| self.records.get(id)).collect())
            .unwrap_or_default()
    }

    /// Get last indexed block number
    pub fn last_indexed_block(&self) -> u64 {
        self.last_block
    }

    /// Get total record count
    pub fn count(&self) -> usize {
        self.records.len()
    }

    /// Get count by entity type
    pub fn count_by_type(&self, entity_type: EntityType) -> usize {
        self.by_type.get(&entity_type).map(|v| v.len()).unwrap_or(0)
    }
}

// ============================================================================
// EVENT INDEXER
// ============================================================================

/// Event indexer service
pub struct EventIndexer {
    store: Arc<RwLock<MemoryIndexStore>>,
    contract_address: String,
}

impl EventIndexer {
    /// Create a new event indexer
    pub fn new(contract_address: &str) -> Self {
        Self {
            store: Arc::new(RwLock::new(MemoryIndexStore::new())),
            contract_address: contract_address.to_string(),
        }
    }

    /// Index a blockchain event
    pub async fn index_event(&self, event: &BlockchainEvent) -> Result<String> {
        let entity_type = EntityType::from_event_name(&event.event_name)
            .context("Unknown event type")?;

        let record = IndexedRecord {
            id: format!("{}-{}", event.tx_hash, event.log_index),
            entity_type,
            entity_id: event.entity_id.clone(),
            cid: event.cid.clone(),
            block_number: event.block_number,
            tx_hash: event.tx_hash.clone(),
            timestamp: event.timestamp,
            event_name: event.event_name.clone(),
            parent_id: event.parent_id.clone(),
            extra: event.extra.clone(),
        };

        let record_id = record.id.clone();

        {
            let mut store = self.store.write().await;
            store.insert(record);
        }

        info!(
            event = %event.event_name,
            entity_id = %event.entity_id,
            cid = %event.cid,
            block = event.block_number,
            "Indexed event"
        );

        Ok(record_id)
    }

    /// Query indexed records
    pub async fn query(&self, filter: QueryFilter) -> Vec<IndexedRecord> {
        let store = self.store.read().await;
        store.query(&filter).into_iter().cloned().collect()
    }

    /// Get all CIDs for an entity
    pub async fn get_cids_for_entity(&self, entity_id: &str) -> Vec<String> {
        let store = self.store.read().await;
        store
            .get_by_entity(entity_id)
            .into_iter()
            .map(|r| r.cid.clone())
            .collect()
    }

    /// Get entity info by CID
    pub async fn get_entity_by_cid(&self, cid: &str) -> Option<IndexedRecord> {
        let store = self.store.read().await;
        store.get_by_cid(cid).first().cloned().cloned()
    }

    /// Trace supply chain from SKU back to farmer
    pub async fn trace_sku(&self, sku_id: &str) -> Result<SupplyChainTrace> {
        let store = self.store.read().await;
        let mut stages = Vec::new();
        let mut current_id = sku_id.to_string();
        let mut farmer_did = None;

        // Get SKU record
        let sku_records = store.get_by_entity(&current_id);
        if sku_records.is_empty() {
            anyhow::bail!("SKU not found: {}", sku_id);
        }

        for record in &sku_records {
            stages.push(TraceStage {
                stage: "packaging".to_string(),
                entity_id: record.entity_id.clone(),
                cid: record.cid.clone(),
                timestamp: record.timestamp,
                block_number: record.block_number,
            });

            if let Some(ref parent) = record.parent_id {
                current_id = parent.clone();
            }
        }

        // Traverse back through the chain
        let mut visited = std::collections::HashSet::new();
        visited.insert(sku_id.to_string());

        while !current_id.is_empty() && !visited.contains(&current_id) {
            visited.insert(current_id.clone());

            let records = store.get_by_entity(&current_id);
            if records.is_empty() {
                break;
            }

            for record in &records {
                let stage_name = match record.entity_type {
                    EntityType::Farmer => "farmer_registration",
                    EntityType::Batch => "fpo_purchase",
                    EntityType::Warehouse => "warehouse_storage",
                    EntityType::Shipment => "logistics",
                    EntityType::Processing => "processing",
                    EntityType::Sku => "packaging",
                    EntityType::AiScore => "ai_scoring",
                    EntityType::FraudReport => "fraud_report",
                };

                stages.push(TraceStage {
                    stage: stage_name.to_string(),
                    entity_id: record.entity_id.clone(),
                    cid: record.cid.clone(),
                    timestamp: record.timestamp,
                    block_number: record.block_number,
                });

                // Check if we reached farmer
                if record.entity_type == EntityType::Farmer {
                    farmer_did = Some(record.entity_id.clone());
                }

                // Move to parent
                if let Some(ref parent) = record.parent_id {
                    current_id = parent.clone();
                } else {
                    current_id = String::new();
                }
            }
        }

        // Sort stages by timestamp
        stages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

        Ok(SupplyChainTrace {
            sku_id: sku_id.to_string(),
            stages,
            farmer_did: farmer_did.clone(),
            complete: farmer_did.is_some(),
        })
    }

    /// Get indexer statistics
    pub async fn stats(&self) -> IndexerStats {
        let store = self.store.read().await;

        IndexerStats {
            total_records: store.count(),
            last_indexed_block: store.last_indexed_block(),
            farmers: store.count_by_type(EntityType::Farmer),
            batches: store.count_by_type(EntityType::Batch),
            skus: store.count_by_type(EntityType::Sku),
            warehouses: store.count_by_type(EntityType::Warehouse),
            shipments: store.count_by_type(EntityType::Shipment),
            processing: store.count_by_type(EntityType::Processing),
            ai_scores: store.count_by_type(EntityType::AiScore),
            fraud_reports: store.count_by_type(EntityType::FraudReport),
        }
    }

    /// Get the underlying store (for testing/advanced use)
    pub fn store(&self) -> Arc<RwLock<MemoryIndexStore>> {
        self.store.clone()
    }
}

/// Blockchain event to index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockchainEvent {
    pub event_name: String,
    pub entity_id: String,
    pub cid: String,
    pub block_number: u64,
    pub tx_hash: String,
    pub log_index: u32,
    pub timestamp: u64,
    pub parent_id: Option<String>,
    pub extra: HashMap<String, String>,
}

/// Indexer statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexerStats {
    pub total_records: usize,
    pub last_indexed_block: u64,
    pub farmers: usize,
    pub batches: usize,
    pub skus: usize,
    pub warehouses: usize,
    pub shipments: usize,
    pub processing: usize,
    pub ai_scores: usize,
    pub fraud_reports: usize,
}

// ============================================================================
// EVENT LISTENER (simplified - real implementation would use alloy)
// ============================================================================

/// Simplified event listener
/// In production, this would use alloy to subscribe to contract events
pub struct EventListener {
    indexer: Arc<EventIndexer>,
    rpc_url: String,
    contract_address: String,
    polling_interval: std::time::Duration,
}

impl EventListener {
    pub fn new(
        indexer: Arc<EventIndexer>,
        rpc_url: &str,
        contract_address: &str,
    ) -> Self {
        Self {
            indexer,
            rpc_url: rpc_url.to_string(),
            contract_address: contract_address.to_string(),
            polling_interval: std::time::Duration::from_secs(12), // ~1 block
        }
    }

    /// Start listening for events (placeholder - implement with alloy)
    pub async fn start(&self) -> Result<()> {
        info!(
            contract = %self.contract_address,
            rpc = %self.rpc_url,
            "Starting event listener"
        );

        // In production, this would:
        // 1. Connect to RPC endpoint using alloy
        // 2. Subscribe to contract events
        // 3. Parse events and call indexer.index_event()

        // Placeholder loop
        loop {
            tokio::time::sleep(self.polling_interval).await;
            // Poll for new events here
            debug!("Polling for new events...");
        }
    }

    /// Process a raw event log (called by actual listener)
    pub async fn process_log(&self, log: RawEventLog) -> Result<()> {
        let event = self.parse_log(log)?;
        self.indexer.index_event(&event).await?;
        Ok(())
    }

    /// Parse a raw log into a BlockchainEvent
    fn parse_log(&self, log: RawEventLog) -> Result<BlockchainEvent> {
        // This would decode the event based on the topic
        // For now, just pass through
        Ok(BlockchainEvent {
            event_name: log.event_name,
            entity_id: log.entity_id,
            cid: log.cid,
            block_number: log.block_number,
            tx_hash: log.tx_hash,
            log_index: log.log_index,
            timestamp: log.timestamp,
            parent_id: log.parent_id,
            extra: log.extra,
        })
    }
}

/// Raw event log from blockchain
#[derive(Debug, Clone)]
pub struct RawEventLog {
    pub event_name: String,
    pub entity_id: String,
    pub cid: String,
    pub block_number: u64,
    pub tx_hash: String,
    pub log_index: u32,
    pub timestamp: u64,
    pub parent_id: Option<String>,
    pub extra: HashMap<String, String>,
}

// ============================================================================
// HTTP HANDLERS (for API integration)
// ============================================================================

use axum::{
    extract::{Path, Query, State},
    Json,
};

/// Query parameters for search endpoint
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub cid: Option<String>,
    pub parent_id: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// HTTP handler: Search indexed records
pub async fn search_records(
    State(indexer): State<Arc<EventIndexer>>,
    Query(query): Query<SearchQuery>,
) -> Json<Vec<IndexedRecord>> {
    let filter = QueryFilter {
        entity_type: query.entity_type.and_then(|t| {
            match t.to_lowercase().as_str() {
                "farmer" => Some(EntityType::Farmer),
                "batch" => Some(EntityType::Batch),
                "warehouse" => Some(EntityType::Warehouse),
                "shipment" => Some(EntityType::Shipment),
                "processing" => Some(EntityType::Processing),
                "sku" => Some(EntityType::Sku),
                "ai_score" => Some(EntityType::AiScore),
                "fraud_report" => Some(EntityType::FraudReport),
                _ => None,
            }
        }),
        entity_id: query.entity_id,
        cid: query.cid,
        parent_id: query.parent_id,
        limit: query.limit,
        offset: query.offset,
        ..Default::default()
    };

    let records = indexer.query(filter).await;
    Json(records)
}

/// HTTP handler: Get CIDs for entity
pub async fn get_entity_cids(
    State(indexer): State<Arc<EventIndexer>>,
    Path(entity_id): Path<String>,
) -> Json<Vec<String>> {
    let cids = indexer.get_cids_for_entity(&entity_id).await;
    Json(cids)
}

/// HTTP handler: Trace SKU
pub async fn trace_sku_handler(
    State(indexer): State<Arc<EventIndexer>>,
    Path(sku_id): Path<String>,
) -> Result<Json<SupplyChainTrace>, String> {
    indexer
        .trace_sku(&sku_id)
        .await
        .map(Json)
        .map_err(|e| e.to_string())
}

/// HTTP handler: Get indexer stats
pub async fn indexer_stats(
    State(indexer): State<Arc<EventIndexer>>,
) -> Json<IndexerStats> {
    Json(indexer.stats().await)
}

/// HTTP handler: Get entity by CID
pub async fn get_by_cid(
    State(indexer): State<Arc<EventIndexer>>,
    Path(cid): Path<String>,
) -> Json<Option<IndexedRecord>> {
    Json(indexer.get_entity_by_cid(&cid).await)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_index_and_query() {
        let indexer = EventIndexer::new("0x1234");

        // Index a farmer event
        let event = BlockchainEvent {
            event_name: "FarmerRegistered".to_string(),
            entity_id: "farmer-001".to_string(),
            cid: "QmFarmer123".to_string(),
            block_number: 100,
            tx_hash: "0xabc".to_string(),
            log_index: 0,
            timestamp: 1704067200,
            parent_id: None,
            extra: HashMap::new(),
        };

        indexer.index_event(&event).await.unwrap();

        // Query by entity
        let results = indexer.query(QueryFilter {
            entity_id: Some("farmer-001".to_string()),
            ..Default::default()
        }).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].cid, "QmFarmer123");
    }

    #[tokio::test]
    async fn test_get_cids_for_entity() {
        let indexer = EventIndexer::new("0x1234");

        // Index multiple events for same entity
        for i in 0..3 {
            let event = BlockchainEvent {
                event_name: "WarehouseStateUpdated".to_string(),
                entity_id: "warehouse-001".to_string(),
                cid: format!("QmWarehouse{}", i),
                block_number: 100 + i as u64,
                tx_hash: format!("0x{}", i),
                log_index: 0,
                timestamp: 1704067200 + i as u64 * 3600,
                parent_id: None,
                extra: HashMap::new(),
            };
            indexer.index_event(&event).await.unwrap();
        }

        let cids = indexer.get_cids_for_entity("warehouse-001").await;
        assert_eq!(cids.len(), 3);
    }

    #[tokio::test]
    async fn test_trace_sku() {
        let indexer = EventIndexer::new("0x1234");

        // Create a supply chain: Farmer -> Batch -> Processing -> SKU
        let events = vec![
            BlockchainEvent {
                event_name: "FarmerRegistered".to_string(),
                entity_id: "farmer-001".to_string(),
                cid: "QmFarmer".to_string(),
                block_number: 100,
                tx_hash: "0x1".to_string(),
                log_index: 0,
                timestamp: 1000,
                parent_id: None,
                extra: HashMap::new(),
            },
            BlockchainEvent {
                event_name: "OwnershipTransfer".to_string(),
                entity_id: "batch-001".to_string(),
                cid: "QmBatch".to_string(),
                block_number: 101,
                tx_hash: "0x2".to_string(),
                log_index: 0,
                timestamp: 2000,
                parent_id: Some("farmer-001".to_string()),
                extra: HashMap::new(),
            },
            BlockchainEvent {
                event_name: "BatchProcessed".to_string(),
                entity_id: "proc-001".to_string(),
                cid: "QmProc".to_string(),
                block_number: 102,
                tx_hash: "0x3".to_string(),
                log_index: 0,
                timestamp: 3000,
                parent_id: Some("batch-001".to_string()),
                extra: HashMap::new(),
            },
            BlockchainEvent {
                event_name: "SKUPackaged".to_string(),
                entity_id: "sku-001".to_string(),
                cid: "QmSku".to_string(),
                block_number: 103,
                tx_hash: "0x4".to_string(),
                log_index: 0,
                timestamp: 4000,
                parent_id: Some("proc-001".to_string()),
                extra: HashMap::new(),
            },
        ];

        for event in events {
            indexer.index_event(&event).await.unwrap();
        }

        let trace = indexer.trace_sku("sku-001").await.unwrap();
        assert_eq!(trace.sku_id, "sku-001");
        assert!(!trace.stages.is_empty());
    }

    #[tokio::test]
    async fn test_stats() {
        let indexer = EventIndexer::new("0x1234");

        let events = vec![
            ("FarmerRegistered", "f1"),
            ("FarmerRegistered", "f2"),
            ("SKUPackaged", "sku1"),
            ("FraudDetected", "fraud1"),
        ];

        for (i, (event_name, entity_id)) in events.iter().enumerate() {
            let event = BlockchainEvent {
                event_name: event_name.to_string(),
                entity_id: entity_id.to_string(),
                cid: format!("Qm{}", i),
                block_number: 100 + i as u64,
                tx_hash: format!("0x{}", i),
                log_index: 0,
                timestamp: 1000 + i as u64,
                parent_id: None,
                extra: HashMap::new(),
            };
            indexer.index_event(&event).await.unwrap();
        }

        let stats = indexer.stats().await;
        assert_eq!(stats.total_records, 4);
        assert_eq!(stats.farmers, 2);
        assert_eq!(stats.skus, 1);
        assert_eq!(stats.fraud_reports, 1);
    }

    #[test]
    fn test_entity_type_from_event() {
        assert_eq!(
            EntityType::from_event_name("FarmerRegistered"),
            Some(EntityType::Farmer)
        );
        assert_eq!(
            EntityType::from_event_name("SKUPackaged"),
            Some(EntityType::Sku)
        );
        assert_eq!(EntityType::from_event_name("Unknown"), None);
    }
}
