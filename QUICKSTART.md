# Quick Start - Integrated Workflow Guide

## 🚀 Get Started in 5 Minutes

This guide shows you how to use the **fully integrated** Oilseed Value Chain system.

## 📦 What's Integrated

All components now work together seamlessly:

```
Frontend → REST API → Workflow Orchestrator → Chain + IPFS → Smart Contract
```

## 🏁 Setup

### 1. Prerequisites
```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Or use nix-shell
nix-shell -p openssl pkg-config
```

### 2. Configure Environment
```bash
cd sih-25/offchain
cp .env.example .env
```

Edit `.env` with your credentials:
```env
# Blockchain
RPC_URL=https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY
PRIVATE_KEY=0x...
CONTRACT_ADDRESS=0x...
CHAIN_ID=11155111

# IPFS
IPFS_API_URL=https://ipfs.infura.io:5001
IPFS_PROJECT_ID=your_project_id
IPFS_PROJECT_SECRET=your_project_secret
```

### 3. Build and Run
```bash
cargo build --release
cargo run
```

Server starts on `http://localhost:3000` 🎉

## 🎯 Usage Options

### Option 1: Complete Workflow (Recommended)

Execute the entire supply chain in one request:

```bash
curl -X POST http://localhost:3000/api/workflow/execute \
  -H "Content-Type: application/json" \
  -d @test_data/complete_workflow.json
```

**What happens:**
1. ✅ Farmer registered → IPFS + Blockchain
2. ✅ FPO purchase recorded → IPFS + Blockchain
3. ✅ Warehouse IoT stored → IPFS + Blockchain
4. ✅ Logistics tracked (4 checkpoints) → IPFS + Blockchain
5. ✅ Batch processed → IPFS + Blockchain
6. ✅ SKUs created (5 packages) → IPFS + Blockchain
7. ✅ AI scoring (commit + reveal) → IPFS + Blockchain

**Response includes:**
- All transaction hashes
- All IPFS CIDs
- Generated SKU IDs
- Complete traceability summary

### Option 2: Individual Stages

Execute one stage at a time:

#### Stage 1: Register Farmer
```bash
curl -X POST http://localhost:3000/api/farmer/register \
  -H "Content-Type: application/json" \
  -d @test_data/farmer.json
```

#### Stage 2: Record FPO Purchase
```bash
curl -X POST http://localhost:3000/api/fpo/purchase \
  -H "Content-Type: application/json" \
  -d '{
    "batch_id": "BATCH-001",
    "farmer_did": "0x0000...0001",
    "metadata": {
      "quantity_kg": 5000,
      "quality_grade": "A"
    }
  }'
```

#### Stage 3: Update Warehouse
```bash
curl -X POST http://localhost:3000/api/warehouse/update \
  -H "Content-Type: application/json" \
  -d '{
    "warehouse_id": "WH-001",
    "iot_data": {
      "temperature_celsius": 22.5,
      "humidity_percent": 45
    }
  }'
```

*...and so on for all 8 stages*

### Option 3: Verification

#### Verify SKU Traceability
```bash
curl -X POST http://localhost:3000/api/workflow/verify-sku \
  -H "Content-Type: application/json" \
  -d '{"sku_id": "MUSTARDOIL-2024-0001"}'
```

Returns complete traceability chain!

#### Verify Farmer
```bash
curl -X POST http://localhost:3000/api/workflow/verify-farmer \
  -H "Content-Type: application/json" \
  -d '{"farmer_did": "0x0000...0001"}'
```

## 📊 How Integration Works

### Behind the Scenes (Automated)

When you call `/api/workflow/execute`:

```
1. workflows.rs orchestrates all stages
     ↓
2. Each stage calls supply_chain_handlers
     ↓
3. Handlers use state.rs (unified app state)
     ↓
4. state.rs provides:
   ├─→ chain.rs (blockchain operations)
   └─→ ipfs.rs (storage operations)
     ↓
5. Operations execute in parallel where possible
     ↓
6. Complete result returned with summary
```

### File Integration Map

```
main.rs
  ├─→ Initializes state.rs (ChainClient + IpfsClient)
  ├─→ Configures routes.rs (maps endpoints)
  └─→ Starts HTTP server

Your Request
  ├─→ routes.rs (finds handler)
  ├─→ workflows.rs OR supply_chain_handlers.rs
  │     ├─→ Uses state.ipfs_client
  │     └─→ Uses state.chain_client
  └─→ Response with tx_hash + CID
```

## 🎯 Complete Examples

### Example 1: Full Workflow with Custom Data

```json
// my_workflow.json
{
  "farmer": {
    "farmer_did": "0x0000000000000000000000000000000000000000000000000000000000000002",
    "crop_id": "SUNFLOWER-2024-001",
    "name": "Priya Sharma",
    "location": "Karnataka, India",
    "land_area": "15 acres",
    "crops": ["sunflower", "groundnut"],
    "contact": "+91-99999-88888"
  },
  "fpo_purchase": {
    "batch_id": "BATCH-SUNFLOWER-2024-001",
    "quantity_kg": 8000.0,
    "quality_grade": "Premium",
    "moisture_content": "6.5%",
    "purchase_price": 72000.0,
    "purchase_date": "2024-01-20"
  },
  "warehouse": {
    "warehouse_id": "WH-KARNATAKA-001",
    "temperature_celsius": 23.0,
    "humidity_percent": 42.0,
    "storage_duration_days": 45
  },
  "logistics": {
    "shipment_id": "SHIP-2024-002",
    "origin": "Bangalore, Karnataka",
    "destination": "Mumbai Processing Unit",
    "checkpoints": [
      {
        "location": "Bangalore Warehouse",
        "latitude": 12.9716,
        "longitude": 77.5946,
        "timestamp": "2024-03-01T09:00:00Z"
      },
      {
        "location": "Pune Hub",
        "latitude": 18.5204,
        "longitude": 73.8567,
        "timestamp": "2024-03-01T18:00:00Z"
      },
      {
        "location": "Mumbai Processing Unit",
        "latitude": 19.0760,
        "longitude": 72.8777,
        "timestamp": "2024-03-02T08:00:00Z"
      }
    ]
  },
  "processing": {
    "input_batch_id": "BATCH-SUNFLOWER-2024-001",
    "process_type": "Expeller Press",
    "yield_percentage": 38.5,
    "output_products": [
      {
        "product_id": "SUNFLOWER-OIL-2024-001",
        "product_type": "Refined Sunflower Oil",
        "quantity_kg": 3080.0
      },
      {
        "product_id": "SUNFLOWER-CAKE-2024-001",
        "product_type": "De-oiled Cake",
        "quantity_kg": 4920.0
      }
    ]
  },
  "packaging": {
    "sku_prefix": "SUNFLOWEROIL-2024",
    "package_type": "500ml PET Bottle",
    "units_per_package": 24,
    "total_packages": 10,
    "expiry_months": 18
  },
  "ai_scoring": {
    "quality_score": 9.2,
    "freshness_score": 9.5,
    "purity_score": 9.0,
    "model_version": "v1.3.0"
  }
}
```

Execute:
```bash
curl -X POST http://localhost:3000/api/workflow/execute \
  -H "Content-Type: application/json" \
  -d @my_workflow.json
```

### Example 2: Consumer Product Verification

A consumer scans QR code on bottle → Frontend calls:

```bash
curl -X POST http://localhost:3000/api/workflow/verify-sku \
  -H "Content-Type: application/json" \
  -d '{"sku_id": "SUNFLOWEROIL-2024-0005"}'
```

Response shows complete traceability:
```json
{
  "sku_id": "SUNFLOWEROIL-2024-0005",
  "parent_batch_hash": "0xabc123...",
  "merkle_root": "0xdef456...",
  "packaged_at": 1709366400,
  "verified": true,
  "trace_summary": "SKU SUNFLOWEROIL-2024-0005 → Batch 0xabc... → Packaged at 1709366400"
}
```

Then query IPFS for full metadata:
```bash
# Get packaging details
curl https://ipfs.io/ipfs/QmPackagingCID...

# Get processing details
curl https://ipfs.io/ipfs/QmProcessingCID...

# Get farmer details
curl https://ipfs.io/ipfs/QmFarmerCID...
```

## 🔧 Troubleshooting

### Server won't start
```bash
# Check environment variables
cat .env

# Verify RPC connection
curl -X POST $RPC_URL \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

# Verify IPFS connection
curl $IPFS_API_URL/api/v0/version
```

### Transaction fails
```bash
# Check account has role
curl -X POST http://localhost:3000/api/farmer/register ...

# If unauthorized, grant role first (using admin account):
# grantRole(farmer_address, ROLE_FARMER) on smart contract
```

### IPFS upload fails
```bash
# Check credentials
echo $IPFS_PROJECT_ID
echo $IPFS_PROJECT_SECRET

# Test direct upload
curl -X POST "$IPFS_API_URL/api/v0/add" \
  -u "$IPFS_PROJECT_ID:$IPFS_PROJECT_SECRET" \
  -F file=@test.txt
```

## 📚 API Reference

### Workflow Endpoints (Integrated)
| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/workflow/execute` | POST | Execute complete supply chain |
| `/api/workflow/verify-sku` | POST | Verify product authenticity |
| `/api/workflow/verify-farmer` | POST | Verify farmer registration |

### Individual Stage Endpoints
| Endpoint | Method | Stage |
|----------|--------|-------|
| `/api/farmer/register` | POST | Stage 1: Farmer Registration |
| `/api/fpo/purchase` | POST | Stage 2: FPO Purchase |
| `/api/warehouse/update` | POST | Stage 3: Warehouse Storage |
| `/api/warehouse/batch-update` | POST | Stage 3: Batch Warehouse Updates |
| `/api/logistics/record` | POST | Stage 4: Logistics Tracking |
| `/api/processing/batch` | POST | Stage 5: Processing |
| `/api/packaging/sku` | POST | Stage 6: Packaging |
| `/api/packaging/verify` | POST | Stage 6: SKU Verification |
| `/api/fraud/report` | POST | Stage 7: Fraud Reporting |
| `/api/ai/commit` | POST | Stage 8: AI Score Commit |
| `/api/ai/reveal` | POST | Stage 8: AI Score Reveal |

### Utility Endpoints
| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/` | GET | API info |
| `/health` | GET | Health check |
| `/api/ipfs/upload` | POST | Direct IPFS upload |

## 🎓 Learning Path

1. **Start Simple**: Use `/api/workflow/execute` with test data
2. **Understand Flow**: Check logs to see how stages execute
3. **Try Individual**: Call single stage endpoints
4. **Customize**: Modify `test_data/complete_workflow.json`
5. **Verify**: Use verification endpoints to check results
6. **Integrate**: Build frontend using the API

## 📖 More Information

- **Complete API Docs**: See `README.md`
- **Integration Details**: See `INTEGRATION.md`
- **Workflow Explanation**: See `WORKFLOW.md`
- **Change History**: See `CHANGELOG.md`

## 🎉 Success!

You now have a fully integrated blockchain supply chain system with:

- ✅ Automatic IPFS metadata storage
- ✅ Blockchain transaction recording
- ✅ Complete workflow orchestration
- ✅ End-to-end traceability
- ✅ Consumer verification
- ✅ Fraud detection
- ✅ AI quality scoring

**All working together seamlessly!**

---

Need help? Check the logs:
```bash
RUST_LOG=debug cargo run
```

Happy tracking! 🚀