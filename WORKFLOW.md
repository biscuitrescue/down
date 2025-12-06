# Supply Chain Workflow Integration Guide

This document explains how all the components of the Oilseed Value Chain system work together, from farmer registration to retail distribution.

## 🔄 Complete Workflow Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         OILSEED SUPPLY CHAIN                            │
│                     End-to-End Blockchain Tracking                      │
└─────────────────────────────────────────────────────────────────────────┘

    ┌──────────────┐
    │   FARMER     │  Stage 1: Registration
    │  Registration│  ─────────────────────
    └──────┬───────┘  • Farmer registers with DID
           │          • Land documents → IPFS
           │          • Crop ID hash → Blockchain
           ▼          • Role: ROLE_FARMER (2)
    ┌──────────────┐
    │     FPO      │  Stage 2: Purchase & Verification
    │   Purchase   │  ──────────────────────────────
    └──────┬───────┘  • FPO buys from farmer
           │          • Quality reports → IPFS
           │          • Ownership transfer → Blockchain
           │          • Role: ROLE_FPO (4)
           ▼
    ┌──────────────┐
    │  WAREHOUSE   │  Stage 3: Storage & Monitoring
    │   Storage    │  ──────────────────────────────
    └──────┬───────┘  • IoT sensors (temp, humidity)
           │          • Continuous data → IPFS
           │          • State hash → Blockchain
           │          • Role: ROLE_WAREHOUSE (8)
           ▼
    ┌──────────────┐
    │  LOGISTICS   │  Stage 4: Transportation
    │   Tracking   │  ─────────────────────
    └──────┬───────┘  • GPS checkpoints
           │          • Location data → IPFS
           │          • Milestone hashes → Blockchain
           │          • Role: ROLE_LOGISTICS (16)
           ▼
    ┌──────────────┐
    │  PROCESSING  │  Stage 5: Transformation
    │  & Refining  │  ───────────────────────
    └──────┬───────┘  • Raw crops → Oil products
           │          • Lab results → IPFS
           │          • Batch transformation → Blockchain
           │          • Role: ROLE_PROCESSOR (32)
           ▼
    ┌──────────────┐
    │  PACKAGING   │  Stage 6: SKU Creation
    │  & Labeling  │  ─────────────────────
    └──────┬───────┘  • Create retail packages
           │          • Unit IDs + Merkle tree
           │          • Package metadata → IPFS
           │          • SKU registration → Blockchain
           │          • Role: ROLE_PACKAGER (64)
           ▼
    ┌──────────────┐
    │    RETAIL    │  Stage 7: Distribution & Fraud Detection
    │ Distribution │  ────────────────────────────────────
    └──────┬───────┘  • Consumer verification
           │          • QR code scanning
           │          • Fraud reporting (if needed)
           │          • Evidence → IPFS
           │          • Role: PUBLIC (anyone)
           ▼
    ┌──────────────┐
    │ AI SCORING   │  Stage 8: Quality Assessment
    │   (Oracle)   │  ──────────────────────────
    └──────────────┘  • Commit-reveal pattern
                      • AI model outputs → IPFS
                      • Score hashes → Blockchain
                      • Role: ROLE_AI_ORACLE (128)
```

## 📂 File Integration Map

### Core Modules and Their Roles

```
┌─────────────────────────────────────────────────────────────────────┐
│                            BACKEND STRUCTURE                        │
└─────────────────────────────────────────────────────────────────────┘

main.rs
  │
  ├─► config.rs ──────► Load ENV variables (RPC_URL, IPFS, etc.)
  │
  ├─► state.rs ────────┬─► chain.rs (ChainClient)
  │                    │     │
  │                    │     ├─► Connect to blockchain RPC
  │                    │     ├─► Sign transactions
  │                    │     └─► Call smart contract methods
  │                    │
  │                    └─► ipfs.rs (IpfsClient)
  │                          │
  │                          ├─► Connect to IPFS API
  │                          ├─► Upload JSON/bytes
  │                          └─► Return CIDs
  │
  ├─► routes.rs ───────┬─► supply_chain_handlers.rs
  │                    │     │
  │                    │     ├─► register_farmer()
  │                    │     ├─► fpo_purchase()
  │                    │     ├─► update_warehouse_state()
  │                    │     ├─► record_logistics()
  │                    │     ├─► process_batch()
  │                    │     ├─► create_sku()
  │                    │     ├─► report_fraud()
  │                    │     └─► commit/reveal_ai_score()
  │                    │
  │                    └─► workflows.rs
  │                          │
  │                          ├─► SupplyChainWorkflow::execute_full_workflow()
  │                          ├─► verify_sku_traceability()
  │                          └─► verify_farmer()
  │
  └─► Run HTTP server with all routes
```

### Data Flow: Single Operation

```
HTTP Request → Route Handler → State (ChainClient + IpfsClient)
                                    │
                    ┌───────────────┴────────────────┐
                    │                                │
                    ▼                                ▼
              1. Upload to IPFS              2. Send to Blockchain
                    │                                │
                    ├─► JSON → Bytes                 ├─► Sign transaction
                    ├─► POST /api/v0/add             ├─► Estimate gas
                    └─► Return CID                   └─► Wait for receipt
                                                           │
                                                           ▼
                    ┌──────────────────────────────────────┘
                    │
                    ▼
              HTTP Response
                    │
                    └─► { tx_hash, cid, ipfs_url, ... }
```

## 🎯 Integration Points

### 1. State Initialization (`state.rs`)

**Purpose**: Central hub connecting blockchain and IPFS

**File**: `offchain/src/state.rs`

```rust
pub struct AppState {
    pub chain_client: Arc<ChainClient>,  // Blockchain operations
    pub ipfs_client: Arc<IpfsClient>,    // IPFS operations
}

impl AppState {
    pub async fn from_env() -> Result<Self> {
        // Initialize both clients from environment
        let chain_client = ChainClient::from_env().await?;
        let ipfs_client = IpfsClient::from_env()?;
        Ok(Self { ... })
    }
}
```

**Integration**: 
- Used by all handlers via `State(state): State<AppState>`
- Provides access to both blockchain and IPFS
- Initialized once in `main.rs`, shared across all requests

### 2. Chain Client (`chain.rs`)

**Purpose**: Blockchain interaction layer

**Key Functions**:
```rust
impl ChainClient {
    // Role management
    pub async fn grant_role(address, role) -> Result<TransactionReceipt>
    pub async fn has_role(address, role) -> Result<bool>
    
    // Stage 1: Farmer
    pub async fn register_farmer(farmer_did, crop_id_hash) -> Result<TransactionReceipt>
    pub async fn verify_farmer(farmer_did) -> Result<(bool, bytes32, u64)>
    
    // Stage 2: FPO
    pub async fn fpo_purchase(batch_hash, farmer_did) -> Result<TransactionReceipt>
    
    // Stage 3: Warehouse
    pub async fn update_warehouse_state(warehouse_id, state_hash) -> Result<TransactionReceipt>
    pub async fn batch_update_warehouse(ids, hashes) -> Result<TransactionReceipt>
    
    // Stage 4: Logistics
    pub async fn record_logistics(shipment_id, location_hash, delivered) -> Result<TransactionReceipt>
    
    // Stage 5: Processing
    pub async fn process_batch(input, transform, outputs) -> Result<TransactionReceipt>
    
    // Stage 6: Packaging
    pub async fn create_sku(sku_id, parent_batch, merkle_root) -> Result<TransactionReceipt>
    pub async fn verify_package_origin(sku_id) -> Result<(bytes32, bytes32, u64)>
    
    // Stage 7: Fraud
    pub async fn report_fraud(sku_id, evidence_hash) -> Result<TransactionReceipt>
    
    // Stage 8: AI
    pub async fn commit_ai_score(batch_hash, commit_hash) -> Result<TransactionReceipt>
    pub async fn reveal_ai_score(batch_hash, reveal_hash, nonce) -> Result<TransactionReceipt>
}

// Helper functions
pub fn hash_string(data: &str) -> FixedBytes<32>
pub fn hash_bytes(data: &[u8]) -> FixedBytes<32>
pub fn generate_commit_hash(reveal_hash, nonce) -> FixedBytes<32>
```

**Integration**:
- Called by supply chain handlers for all blockchain operations
- Each function maps to a smart contract method
- Returns transaction receipts with hashes

### 3. IPFS Client (`ipfs.rs`)

**Purpose**: Decentralized storage for metadata

**Key Functions**:
```rust
impl IpfsClient {
    // Upload JSON data
    pub async fn upload_json<T: Serialize>(value: &T) -> Result<String>
    
    // Upload raw bytes
    pub async fn upload_bytes(bytes: Vec<u8>) -> Result<String>
    
    // Get configuration
    pub fn api_url(&self) -> &str
    pub fn has_auth(&self) -> bool
}

// HTTP Handler
pub async fn upload_to_ipfs(
    State(state): State<AppState>,
    Json(payload): Json<UploadRequest>
) -> Result<Json<UploadResponse>>
```

**Integration**:
- Used by all handlers to store large metadata
- Returns CID (Content Identifier) 
- CIDs stored off-chain, referenced in events

### 4. Supply Chain Handlers (`supply_chain_handlers.rs`)

**Purpose**: Individual stage operations

**Pattern** (same for all stages):
```rust
pub async fn stage_handler(
    State(state): State<AppState>,
    Json(payload): Json<StageRequest>
) -> Result<Json<StageResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 1. Upload metadata to IPFS
    let metadata_cid = state.ipfs_client.upload_json(&metadata).await?;
    
    // 2. Hash identifiers
    let id_hash = hash_string(&payload.id);
    
    // 3. Record on blockchain
    let receipt = state.chain_client.stage_method(id_hash, ...).await?;
    
    // 4. Return response
    Ok(Json(StageResponse {
        tx_hash: format!("{:?}", receipt.transaction_hash),
        metadata_cid,
        ipfs_url: format!("https://ipfs.io/ipfs/{}", metadata_cid),
        ...
    }))
}
```

**Available Handlers**:
- `register_farmer` - Stage 1
- `fpo_purchase` - Stage 2
- `update_warehouse_state` - Stage 3
- `batch_update_warehouse` - Stage 3 (bulk)
- `record_logistics` - Stage 4
- `process_batch` - Stage 5
- `create_sku` - Stage 6
- `verify_sku` - Stage 6 (verification)
- `report_fraud` - Stage 7
- `commit_ai_score` - Stage 8a
- `reveal_ai_score` - Stage 8b
- `verify_farmer` - Verification

### 5. Workflow Orchestration (`workflows.rs`)

**Purpose**: End-to-end automation

**Key Type**:
```rust
pub struct SupplyChainWorkflow {
    state: AppState,
}

impl SupplyChainWorkflow {
    // Execute all stages in sequence
    pub async fn execute_full_workflow(
        data: CompleteWorkflowData
    ) -> Result<WorkflowResult>
    
    // Verification
    pub async fn verify_sku_traceability(sku_id) -> Result<SkuTraceability>
    pub async fn verify_farmer(farmer_did) -> Result<FarmerVerification>
}
```

**Workflow Execution**:
```rust
let workflow = SupplyChainWorkflow::new(app_state);
let result = workflow.execute_full_workflow(data).await?;

// Result contains:
// - All transaction hashes
// - All IPFS CIDs
// - Generated SKU IDs
// - Summary statistics
```

**Integration**:
- Reuses all chain_client and ipfs_client methods
- Orchestrates multiple operations
- Provides complete traceability

### 6. Routes Configuration (`routes.rs`)

**Purpose**: HTTP endpoint mapping

**Structure**:
```rust
pub fn configure_routes(state: AppState) -> Router {
    Router::new()
        // Workflow routes (orchestrated)
        .route("/api/workflow/execute", post(workflows::http_handlers::execute_workflow))
        .route("/api/workflow/verify-sku", post(workflows::http_handlers::verify_sku_handler))
        .route("/api/workflow/verify-farmer", post(workflows::http_handlers::verify_farmer_handler))
        
        // Individual stage routes
        .route("/api/farmer/register", post(supply_chain_handlers::register_farmer))
        .route("/api/fpo/purchase", post(supply_chain_handlers::fpo_purchase))
        // ... all other stages ...
        
        // Direct IPFS upload
        .route("/api/ipfs/upload", post(ipfs::upload_to_ipfs))
        
        .with_state(state)
}
```

## 🔗 Usage Examples

### Example 1: Individual Stage Operation

```bash
# Stage 1: Register a farmer
curl -X POST http://localhost:3000/api/farmer/register \
  -H "Content-Type: application/json" \
  -d '{
    "farmer_did": "0x0000000000000000000000000000000000000000000000000000000000000001",
    "crop_id": "MUSTARD-2024-001",
    "metadata": {
      "name": "Rajesh Kumar",
      "location": "Punjab, India",
      "land_area": "10 acres"
    }
  }'

# Response:
{
  "tx_hash": "0xabc...",
  "farmer_did": "0x00...01",
  "crop_id_hash": "0xdef...",
  "metadata_cid": "QmXxx...",
  "ipfs_url": "https://ipfs.io/ipfs/QmXxx..."
}
```

**What happens internally**:
1. HTTP request → `supply_chain_handlers::register_farmer()`
2. Handler extracts `AppState` (has chain + IPFS clients)
3. Upload metadata to IPFS → get CID
4. Hash farmer DID and crop ID
5. Call `chain_client.register_farmer(farmer_did, crop_id_hash)`
6. Wait for blockchain transaction receipt
7. Return response with both tx_hash and CID

### Example 2: Complete Workflow

```bash
# Execute entire supply chain workflow
curl -X POST http://localhost:3000/api/workflow/execute \
  -H "Content-Type: application/json" \
  -d @workflow_data.json

# workflow_data.json contains all stages data
```

**What happens internally**:
1. HTTP request → `workflows::http_handlers::execute_workflow()`
2. Create `SupplyChainWorkflow` with app_state
3. Execute stages sequentially:
   - Call `register_farmer()` internally
   - Call `record_fpo_purchase()` internally
   - Call `record_warehouse_storage()` internally
   - Call `record_logistics_journey()` internally (multiple checkpoints)
   - Call `process_batch()` internally
   - Call `create_retail_packages()` internally (multiple SKUs)
   - Call `execute_ai_scoring()` internally (optional)
4. Track all transactions and CIDs
5. Return comprehensive result with summary

### Example 3: Verification Flow

```bash
# Consumer scans QR code on product
# QR contains SKU ID: "MUSTARDOIL-2024-0001"

curl -X POST http://localhost:3000/api/workflow/verify-sku \
  -H "Content-Type: application/json" \
  -d '{"sku_id": "MUSTARDOIL-2024-0001"}'

# Response shows:
{
  "sku_id": "MUSTARDOIL-2024-0001",
  "parent_batch_hash": "0x...",
  "merkle_root": "0x...",
  "packaged_at": 1234567890,
  "verified": true,
  "trace_summary": "SKU MUSTARDOIL-2024-0001 → Batch 0x... → Packaged at ..."
}
```

**What happens internally**:
1. Hash SKU ID
2. Query blockchain: `chain_client.verify_package_origin(sku_hash)`
3. Blockchain returns parent batch, merkle root, timestamp
4. Build traceability response
5. Frontend can then query IPFS for full metadata using stored CIDs

## 🔐 Role-Based Access Control Flow

```
┌─────────────────────────────────────────────────────────────┐
│                     RBAC INTEGRATION                        │
└─────────────────────────────────────────────────────────────┘

Admin Account (deployer)
  │
  ├─► grantRole(farmer_address, ROLE_FARMER)
  ├─► grantRole(fpo_address, ROLE_FPO)
  ├─► grantRole(warehouse_address, ROLE_WAREHOUSE)
  └─► grantRole(...) for all roles

Each Endpoint Checks Roles:
  │
  ├─► /api/farmer/register      → requires ROLE_FARMER (2)
  ├─► /api/fpo/purchase          → requires ROLE_FPO (4)
  ├─► /api/warehouse/update      → requires ROLE_WAREHOUSE (8)
  ├─► /api/logistics/record      → requires ROLE_LOGISTICS (16)
  ├─► /api/processing/batch      → requires ROLE_PROCESSOR (32)
  ├─► /api/packaging/sku         → requires ROLE_PACKAGER (64)
  ├─► /api/ai/commit             → requires ROLE_AI_ORACLE (128)
  └─► /api/fraud/report          → PUBLIC (no role required)

Smart Contract Modifier:
  modifier onlyRole(uint8 role) {
      if (roles[msg.sender] & role == 0) revert Unauthorized();
      _;
  }
```

## 📊 Data Flow Diagram

```
┌────────────────────────────────────────────────────────────────┐
│                    COMPLETE DATA FLOW                          │
└────────────────────────────────────────────────────────────────┘

Frontend/CLI
    │
    │ HTTP POST /api/farmer/register
    ▼
┌─────────────────────────┐
│   Axum HTTP Server      │
│   (main.rs + routes.rs) │
└────────┬────────────────┘
         │
         │ Extract State(AppState)
         ▼
┌─────────────────────────┐
│  Handler Function       │
│  (supply_chain_handlers │
│   or workflows)         │
└────┬──────────────┬─────┘
     │              │
     │              │
     ▼              ▼
┌──────────┐   ┌──────────┐
│  IPFS    │   │Blockchain│
│  Client  │   │  Client  │
└────┬─────┘   └─────┬────┘
     │               │
     │               │
     ▼               ▼
┌──────────┐   ┌──────────┐
│   IPFS   │   │  Smart   │
│  Network │   │ Contract │
└────┬─────┘   └─────┬────┘
     │               │
     │ CID           │ Tx Hash
     │               │
     └───────┬───────┘
             │
             ▼
       HTTP Response
       {
         tx_hash: "0x...",
         metadata_cid: "Qm...",
         ipfs_url: "https://..."
       }
```

## 🧪 Testing the Integration

### 1. Start the Server
```bash
cd offchain
cargo run
```

### 2. Test Individual Stage
```bash
# Test farmer registration
curl -X POST http://localhost:3000/api/farmer/register \
  -H "Content-Type: application/json" \
  -d @test_data/farmer.json
```

### 3. Test Complete Workflow
```bash
# Execute full workflow
curl -X POST http://localhost:3000/api/workflow/execute \
  -H "Content-Type: application/json" \
  -d @test_data/complete_workflow.json
```

### 4. Verify Traceability
```bash
# Verify SKU
curl -X POST http://localhost:3000/api/workflow/verify-sku \
  -H "Content-Type: application/json" \
  -d '{"sku_id": "MUSTARDOIL-2024-0001"}'
```

## 🎓 Key Integration Patterns

### Pattern 1: IPFS + Blockchain Combo
```rust
// 1. Upload large data to IPFS
let metadata_cid = state.ipfs_client.upload_json(&metadata).await?;

// 2. Hash the data
let data_hash = hash_bytes(&serde_json::to_vec(&metadata)?);

// 3. Store hash on blockchain (gas efficient)
let receipt = state.chain_client.method(id, data_hash).await?;

// 4. Return both references
(receipt.transaction_hash, metadata_cid)
```

### Pattern 2: Commit-Reveal for AI
```rust
// Commit phase (hide score)
let reveal_hash = hash_bytes(&score_data);
let nonce = generate_random_nonce();
let commit_hash = generate_commit_hash(reveal_hash, nonce);
chain_client.commit_ai_score(batch_hash, commit_hash).await?;

// Reveal phase (after time-lock)
chain_client.reveal_ai_score(batch_hash, reveal_hash, nonce).await?;
```

### Pattern 3: Batch Operations
```rust
// Multiple operations in one transaction (save gas)
let ids = vec![warehouse1, warehouse2, warehouse3];
let hashes = vec![hash1, hash2, hash3];
chain_client.batch_update_warehouse(ids, hashes).await?;
```

### Pattern 4: Merkle Tree Verification
```rust
// Create Merkle root from unit IDs
let unit_hashes: Vec<_> = unit_ids.iter().map(|id| hash_string(id)).collect();
let merkle_root = compute_merkle_root(&unit_hashes);

// Store on blockchain
chain_client.create_sku(sku_id, parent_batch, merkle_root).await?;

// Later: verify any unit is part of SKU using Merkle proof
```

## 🚀 Extending the Workflow

To add a new stage or feature:

1. **Add smart contract function** (in `contracts/nyx.sol`)
2. **Add to ChainClient** (in `src/chain.rs`)
3. **Create handler** (in `src/supply_chain_handlers.rs`)
4. **Add route** (in `src/routes.rs`)
5. **Update workflow** (in `src/workflows.rs` if needed)
6. **Document** (update this file)

## 📝 Summary

All files are integrated through:

1. **`state.rs`** - Central connection point (ChainClient + IpfsClient)
2. **`chain.rs`** - All blockchain operations
3. **`ipfs.rs`** - All IPFS operations
4. **`supply_chain_handlers.rs`** - Individual stage endpoints
5. **`workflows.rs`** - Orchestrated multi-stage operations
6. **`routes.rs`** - HTTP endpoint mapping
7. **`main.rs`** - Application bootstrap

**Data always flows**: HTTP → Handler → State → (IPFS + Blockchain) → Response

No heavy changes needed - the architecture is modular and each component has a clear responsibility!