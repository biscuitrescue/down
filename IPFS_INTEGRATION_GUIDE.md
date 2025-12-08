# IPFS Integration Guide for Oilseed Value Chain

This comprehensive guide shows you how to properly deploy and integrate the IPFS architecture with your Rust backend and Solidity contracts for a production-ready supply chain system.

## 🎯 Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                       COMPLETE INTEGRATION FLOW                     │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐  │
│  │   Frontend      │    │   Mobile App    │    │  IoT Devices    │  │
│  │   (React/Vue)   │    │   (Flutter)     │    │  (Sensors)      │  │
│  └─────────────────┘    └─────────────────┘    └─────────────────┘  │
│           │                       │                       │         │
│           └───────────────────────┼───────────────────────┘         │
│                                   │                                 │
│  ┌─────────────────────────────────┼─────────────────────────────────┐ │
│  │              RUST BACKEND       │                                 │ │
│  │  ┌─────────────────────────────┐│┌───────────────────────────────┐│ │
│  │  │     API Gateway             │││    IPFS Integration Layer     ││ │
│  │  │  - Supply chain endpoints  │││  - Multi-provider support     ││ │
│  │  │  - Validation & auth       │││  - Automatic failover         ││ │
│  │  │  - Rate limiting           │││  - Content validation         ││ │
│  │  └─────────────────────────────┘│└───────────────────────────────┘│ │
│  └─────────────────────────────────┼─────────────────────────────────┘ │
│                                    │                                  │
│  ┌─────────────────────────────────┼─────────────────────────────────┐ │
│  │            IPFS LAYER           │                                 │ │
│  │                                 │                                 │ │
│  │  PRIMARY: Infura/Pinata        │  SECONDARY: Self-hosted Node    │ │
│  │  ┌─────────────────────────┐   │  ┌─────────────────────────────┐│ │
│  │  │  - Managed service      │   │  │  - Docker deployment       ││ │
│  │  │  - Global CDN           │   │  │  - Local cluster           ││ │
│  │  │  - Auto-pinning         │   │  │  - Backup & redundancy     ││ │
│  │  └─────────────────────────┘   │  └─────────────────────────────┘│ │
│  └─────────────────────────────────┼─────────────────────────────────┘ │
│                                    │                                  │
│  ┌─────────────────────────────────┼─────────────────────────────────┐ │
│  │         BLOCKCHAIN LAYER        │                                 │ │
│  │                                 │                                 │ │
│  │  ┌─────────────────────────────────────────────────────────────┐ │ │
│  │  │              OilseedValueChain.sol                          │ │ │
│  │  │                                                             │ │ │
│  │  │  Events with IPFS CIDs:                                    │ │ │
│  │  │  - FarmerRegistered(farmerDID, cropHash, metadataCID)      │ │ │
│  │  │  - OwnershipTransfer(batchHash, from, to, metadataCID)     │ │ │
│  │  │  - WarehouseStateUpdated(warehouseId, stateHash, CID)      │ │ │
│  │  │  - BatchProcessed(inputHash, transformHash, CID)           │ │ │
│  │  │  - SKUPackaged(skuId, parentBatch, merkleRoot, CID)        │ │ │
│  │  │  - AIScoreRevealed(batchHash, revealHash, CID)             │ │ │
│  │  └─────────────────────────────────────────────────────────────┘ │ │
│  └─────────────────────────────────┼─────────────────────────────────┘ │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

## 🚀 Quick Start Deployment

### Step 1: Environment Setup

1. **Clone and configure environment:**
```bash
git clone <repository-url>
cd sih-25

# Set up production environment
cp offchain/.env.production offchain/.env
# Edit offchain/.env with your actual credentials
```

2. **Configure IPFS services:**
```bash
# For Infura (recommended for production)
export IPFS_API_URL="https://ipfs.infura.io:5001"
export IPFS_PROJECT_ID="your_infura_project_id"
export IPFS_PROJECT_SECRET="your_infura_secret"

# For Pinata (backup service)
export PINATA_API_KEY="your_pinata_api_key"
export PINATA_SECRET_KEY="your_pinata_secret"

# Gateway URL
export IPFS_GATEWAY_URL="https://ipfs.io/ipfs/"
```

### Step 2: Deploy IPFS Infrastructure

```bash
# Deploy local IPFS infrastructure (development)
cd ipfs-infrastructure
./deploy.sh development

# Or deploy production infrastructure
./deploy.sh production
```

### Step 3: Start Rust Backend

```bash
cd offchain

# Install dependencies and build
cargo build --release

# Run migrations (if using PostgreSQL)
# DATABASE_URL="postgresql://..." cargo run --bin migrate

# Start the server
RUST_LOG=info cargo run --release
```

### Step 4: Deploy Smart Contracts

```bash
# Compile contracts
forge build

# Deploy to local network (Anvil)
forge script script/Deploy.s.sol:DeployScript --rpc-url http://localhost:8545 --broadcast

# Deploy to testnet/mainnet
forge script script/Deploy.s.sol:DeployScript --rpc-url $RPC_URL --private-key $PRIVATE_KEY --broadcast
```

### Step 5: Verify Integration

```bash
# Run comprehensive integration tests
./test_ipfs_infrastructure.sh
```

## 📝 Complete Integration Example

### 1. Farmer Registration Flow

**Frontend Request:**
```javascript
// Frontend uploads farmer data to backend
const farmerData = {
  name: "Rajesh Kumar",
  contact_info: {
    phone: "+91-9876543210",
    address: "Village Khajuraho, Chhatarpur, MP"
  },
  land_details: {
    total_area: "5.2 hectares",
    soil_type: "Alluvial",
    coordinates: [25.837, 79.919]
  },
  crop_info: {
    variety: "Mustard",
    planting_date: "2024-01-10",
    expected_yield: "1500 kg"
  },
  documents: {
    land_deed: "base64_encoded_document",
    soil_report: "base64_encoded_report"
  }
};

const response = await fetch('/api/ipfs/farmer', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(farmerData)
});

const result = await response.json();
// Returns: { cid: "Qm...", gateway_url: "https://ipfs.io/ipfs/Qm..." }
```

**Rust Backend Processing:**
```rust
// In your supply_chain_handlers.rs
pub async fn register_farmer_complete(
    State(state): State<AppState>,
    Json(farmer_data): Json<FarmerRegistrationRequest>,
) -> ApiResult<FarmerRegistrationResponse> {
    // 1. Validate farmer data
    validate_farmer_data(&farmer_data)?;
    
    // 2. Create standardized metadata
    let metadata = FarmerMetadata {
        version: "1.0".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        stage: "farmer_registration".to_string(),
        farmer_profile: farmer_data.profile,
        verification: farmer_data.verification,
    };
    
    // 3. Upload to IPFS with redundancy
    let ipfs_cid = state.ipfs_client
        .upload_json(&metadata)
        .await
        .map_err(ApiError::ipfs_upload_failed)?;
    
    // 4. Generate farmer DID and crop ID hash
    let farmer_did = generate_farmer_did(&farmer_data);
    let crop_id_hash = hash_crop_info(&farmer_data.crop_info);
    
    // 5. Call smart contract
    let tx_hash = state.blockchain_client
        .register_farmer(farmer_did, crop_id_hash, &ipfs_cid)
        .await
        .map_err(ApiError::blockchain_error)?;
    
    // 6. Store in database for indexing
    store_farmer_record(&state.db, &farmer_data, &ipfs_cid, &tx_hash).await?;
    
    Ok(Json(FarmerRegistrationResponse {
        farmer_did: format!("0x{}", hex::encode(farmer_did)),
        ipfs_cid,
        gateway_url: state.ipfs_client.gateway_url(&ipfs_cid),
        transaction_hash: format!("0x{}", hex::encode(tx_hash)),
        status: "registered".to_string(),
    }))
}
```

**Smart Contract Event:**
```solidity
// The contract emits this event
emit FarmerRegistered(
    farmerDID,           // bytes32: 0x1234...
    cropIDHash,          // bytes32: 0x5678...
    block.timestamp,     // uint64: 1704446400
    ipfsCID              // string: "Qm..."
);
```

### 2. Supply Chain Tracking Flow

**Complete workflow from farmer to consumer:**

```rust
// 1. FPO Purchase
pub async fn fpo_purchase_with_ipfs(
    State(state): State<AppState>,
    Json(purchase_data): Json<FPOPurchaseRequest>,
) -> ApiResult<PurchaseResponse> {
    // Upload purchase documents to IPFS
    let metadata = FPOPurchaseMetadata {
        transaction: purchase_data.transaction,
        documents: purchase_data.documents,
        fpo_info: purchase_data.fpo_info,
        // ... other fields
    };
    
    let ipfs_cid = state.ipfs_client.upload_json(&metadata).await?;
    
    // Record on blockchain
    let tx_hash = state.blockchain_client
        .fpo_purchase(
            purchase_data.batch_hash,
            purchase_data.farmer_did,
            &ipfs_cid
        ).await?;
    
    Ok(Json(PurchaseResponse { ipfs_cid, tx_hash, /* ... */ }))
}

// 2. Warehouse Storage with IoT Data
pub async fn warehouse_update_with_iot(
    State(state): State<AppState>,
    Json(warehouse_data): Json<WarehouseUpdateRequest>,
) -> ApiResult<WarehouseResponse> {
    // Process IoT data and create rollup
    let iot_rollup = process_iot_data(&warehouse_data.iot_readings).await?;
    
    // Upload detailed IoT data to IPFS
    let iot_cid = state.ipfs_client.upload_json(&warehouse_data.iot_readings).await?;
    
    // Create warehouse metadata with IoT rollup
    let metadata = WarehouseMetadata {
        warehouse_info: warehouse_data.warehouse_info,
        storage_details: warehouse_data.storage_details,
        iot_data: IoTData {
            rollup_period: "24h".to_string(),
            averages: iot_rollup.averages,
            alerts: iot_rollup.alerts,
            raw_data_cid: iot_cid,
        },
        // ... other fields
    };
    
    let metadata_cid = state.ipfs_client.upload_json(&metadata).await?;
    
    // Update warehouse state on blockchain
    let state_hash = hash_warehouse_state(&metadata);
    let tx_hash = state.blockchain_client
        .update_warehouse_state(
            warehouse_data.warehouse_id,
            state_hash,
            &metadata_cid
        ).await?;
    
    Ok(Json(WarehouseResponse { 
        metadata_cid, 
        iot_cid, 
        tx_hash,
        state_hash: format!("0x{}", hex::encode(state_hash))
    }))
}

// 3. Processing with Lab Results
pub async fn process_batch_with_reports(
    State(state): State<AppState>,
    Json(processing_data): Json<ProcessingRequest>,
) -> ApiResult<ProcessingResponse> {
    // Upload lab reports and processing data
    let lab_reports_cid = state.ipfs_client
        .upload_json(&processing_data.lab_reports).await?;
    
    let metadata = ProcessingMetadata {
        facility_info: processing_data.facility_info,
        process_parameters: processing_data.parameters,
        lab_results_cid: lab_reports_cid,
        yield_data: processing_data.yield_data,
        // ... other fields
    };
    
    let metadata_cid = state.ipfs_client.upload_json(&metadata).await?;
    
    // Record processing on blockchain
    let transform_hash = hash_processing_transform(&processing_data);
    let tx_hash = state.blockchain_client
        .process_batch(
            processing_data.input_batch_hash,
            transform_hash,
            &processing_data.output_batch_hashes,
            &metadata_cid
        ).await?;
    
    Ok(Json(ProcessingResponse { 
        metadata_cid, 
        lab_reports_cid, 
        tx_hash,
        output_batches: processing_data.output_batch_hashes
    }))
}
```

## 🔗 Frontend Integration Examples

### React Integration

```javascript
// IPFS Service Class
class IPFSService {
  constructor(backendUrl) {
    this.backendUrl = backendUrl;
  }

  async uploadSupplyChainData(stage, data) {
    const response = await fetch(`${this.backendUrl}/api/ipfs/upload`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ data, stage })
    });
    
    if (!response.ok) throw new Error('Upload failed');
    return await response.json();
  }

  async retrieveSupplyChainData(cid) {
    const response = await fetch(`${this.backendUrl}/api/ipfs/retrieve`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ cid })
    });
    
    if (!response.ok) throw new Error('Retrieval failed');
    return await response.json();
  }

  async registerFarmer(farmerData) {
    const response = await fetch(`${this.backendUrl}/api/ipfs/farmer`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(farmerData)
    });
    
    return await response.json();
  }
}

// React Component Example
import { useState, useEffect } from 'react';

function FarmerRegistration() {
  const [farmerData, setFarmerData] = useState({});
  const [isUploading, setIsUploading] = useState(false);
  const [result, setResult] = useState(null);
  
  const ipfsService = new IPFSService('http://localhost:3000');

  const handleSubmit = async (e) => {
    e.preventDefault();
    setIsUploading(true);
    
    try {
      const result = await ipfsService.registerFarmer(farmerData);
      setResult(result);
      
      // Optionally verify on blockchain
      const verification = await verifyOnBlockchain(result.farmer_did);
      setResult(prev => ({ ...prev, verified: verification }));
      
    } catch (error) {
      console.error('Registration failed:', error);
    } finally {
      setIsUploading(false);
    }
  };

  return (
    <div className="farmer-registration">
      <form onSubmit={handleSubmit}>
        <input 
          type="text" 
          placeholder="Farmer Name"
          onChange={(e) => setFarmerData(prev => ({ 
            ...prev, 
            name: e.target.value 
          }))}
        />
        {/* Other form fields */}
        
        <button type="submit" disabled={isUploading}>
          {isUploading ? 'Uploading...' : 'Register Farmer'}
        </button>
      </form>
      
      {result && (
        <div className="result">
          <h3>Registration Successful!</h3>
          <p><strong>Farmer DID:</strong> {result.farmer_did}</p>
          <p><strong>IPFS CID:</strong> 
            <a href={result.gateway_url} target="_blank">
              {result.ipfs_cid}
            </a>
          </p>
          <p><strong>Transaction:</strong> {result.transaction_hash}</p>
          {result.verified && <p>✅ Verified on blockchain</p>}
        </div>
      )}
    </div>
  );
}
```

### Mobile App Integration (Flutter)

```dart
// IPFS Service for Flutter
class IPFSService {
  final String backendUrl;
  final http.Client client;

  IPFSService(this.backendUrl) : client = http.Client();

  Future<Map<String, dynamic>> uploadFarmerData(Map<String, dynamic> farmerData) async {
    final response = await client.post(
      Uri.parse('$backendUrl/api/ipfs/farmer'),
      headers: {'Content-Type': 'application/json'},
      body: json.encode(farmerData),
    );

    if (response.statusCode == 200) {
      return json.decode(response.body);
    } else {
      throw Exception('Failed to upload farmer data');
    }
  }

  Future<Map<String, dynamic>> trackShipment(String shipmentId) async {
    // Implementation for shipment tracking
  }
}

// Flutter Widget Example
class FarmerRegistrationPage extends StatefulWidget {
  @override
  _FarmerRegistrationPageState createState() => _FarmerRegistrationPageState();
}

class _FarmerRegistrationPageState extends State<FarmerRegistrationPage> {
  final IPFSService ipfsService = IPFSService('http://your-backend.com');
  bool isUploading = false;
  Map<String, dynamic>? result;

  Future<void> submitFarmerData(Map<String, dynamic> data) async {
    setState(() => isUploading = true);
    
    try {
      final uploadResult = await ipfsService.uploadFarmerData(data);
      setState(() => result = uploadResult);
      
      // Show success message
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Farmer registered successfully!'))
      );
    } catch (error) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Registration failed: $error'))
      );
    } finally {
      setState(() => isUploading = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: Text('Register Farmer')),
      body: Column(
        children: [
          // Form widgets here
          ElevatedButton(
            onPressed: isUploading ? null : () => submitFarmerData(/* form data */),
            child: isUploading ? CircularProgressIndicator() : Text('Register'),
          ),
          if (result != null) ...[
            Card(
              child: Column(
                children: [
                  Text('Registration Complete!'),
                  Text('IPFS CID: ${result!['ipfs_cid']}'),
                  Text('Transaction: ${result!['transaction_hash']}'),
                ],
              ),
            ),
          ],
        ],
      ),
    );
  }
}
```

## 📊 Monitoring and Analytics

### Custom Metrics Collection

```rust
// Add to your Rust backend
use prometheus::{Counter, Histogram, register_counter, register_histogram};

lazy_static! {
    static ref IPFS_UPLOADS: Counter = register_counter!(
        "ipfs_uploads_total", 
        "Total number of IPFS uploads"
    ).unwrap();
    
    static ref IPFS_UPLOAD_DURATION: Histogram = register_histogram!(
        "ipfs_upload_duration_seconds", 
        "Time spent uploading to IPFS"
    ).unwrap();
}

pub async fn upload_with_metrics<T: Serialize>(
    client: &IpfsClient,
    data: &T
) -> Result<String> {
    let timer = IPFS_UPLOAD_DURATION.start_timer();
    
    match client.upload_json(data).await {
        Ok(cid) => {
            IPFS_UPLOADS.inc();
            timer.observe_duration();
            Ok(cid)
        }
        Err(e) => {
            timer.observe_duration();
            Err(e)
        }
    }
}
```

### Grafana Dashboard Queries

```prometheus
# IPFS Upload Rate
rate(ipfs_uploads_total[5m])

# Average Upload Time
rate(ipfs_upload_duration_seconds_sum[5m]) / rate(ipfs_upload_duration_seconds_count[5m])

# Success Rate
(rate(ipfs_uploads_total[5m]) / rate(api_requests_total{endpoint=~"/api/ipfs/.*"}[5m])) * 100
```

## 🔒 Production Security Checklist

### 1. IPFS Security
- [ ] Enable authentication for IPFS API endpoints
- [ ] Configure firewall rules (only necessary ports open)
- [ ] Use private IPFS networks for sensitive data
- [ ] Implement content encryption for PII
- [ ] Set up rate limiting on public gateways

### 2. Backend Security
- [ ] Implement JWT authentication
- [ ] Add request rate limiting
- [ ] Validate all inputs thoroughly
- [ ] Use HTTPS in production
- [ ] Implement CORS policies
- [ ] Add audit logging

### 3. Smart Contract Security
- [ ] Use role-based access control
- [ ] Implement upgrade mechanisms safely
- [ ] Add emergency pause functionality
- [ ] Conduct security audits
- [ ] Use multi-signature wallets for admin functions

### 4. Infrastructure Security
- [ ] Use secrets management (HashiCorp Vault, AWS Secrets Manager)
- [ ] Regular security updates
- [ ] Network segmentation
- [ ] Backup encryption
- [ ] Incident response procedures

## 🚀 Scaling Strategies

### Horizontal Scaling

1. **IPFS Cluster Scaling:**
```bash
# Add more IPFS cluster nodes
docker-compose -p oilseed-ipfs up -d --scale ipfs-cluster=5
```

2. **Backend Scaling:**
```bash
# Load balancer configuration (nginx.conf)
upstream backend {
    server backend1:3000;
    server backend2:3000;
    server backend3:3000;
}
```

3. **Database Scaling:**
```sql
-- Read replicas for PostgreSQL
-- Partitioning for large tables
CREATE TABLE ipfs_content_2024 PARTITION OF ipfs_content
FOR VALUES FROM ('2024-01-01') TO ('2025-01-01');
```

### Performance Optimization

1. **Caching Strategy:**
```rust
// Redis caching for frequently accessed data
pub async fn get_cached_content(
    redis: &mut redis::Connection,
    cid: &str
) -> Result<Option<Vec<u8>>> {
    let cached: Option<Vec<u8>> = redis.get(format!("ipfs:content:{}", cid))?;
    Ok(cached)
}

pub async fn cache_content(
    redis: &mut redis::Connection,
    cid: &str,
    content: &[u8],
    ttl: i64
) -> Result<()> {
    redis.setex(format!("ipfs:content:{}", cid), ttl, content)?;
    Ok(())
}
```

2. **Database Optimization:**
```sql
-- Optimized indexes for common queries
CREATE INDEX CONCURRENTLY idx_ipfs_content_stage_created 
ON ipfs_content(stage, created_at DESC);

-- Partial indexes for active records
CREATE INDEX CONCURRENTLY idx_active_batches 
ON batches(status, created_at) WHERE status IN ('active', 'processing');
```

## 📈 Cost Optimization

### IPFS Cost Management

1. **Service Comparison:**
```yaml
# Cost analysis (approximate)
Infura IPFS:
  - Storage: $0.15/GB/month
  - Bandwidth: $0.09/GB
  - API calls: $0.50/1K requests

Pinata:
  - Storage: $0.20/GB/month  
  - Bandwidth: $0.10/GB
  - API calls: $0.40/1K requests

Self-hosted:
  - Server costs: $50-200/month
  - Storage: $0.05-0.10/GB/month
  - Bandwidth: Varies by provider
```

2. **Cost Optimization Strategies:**
```rust
// Implement data lifecycle management
pub async fn archive_old_content(
    client: &IpfsClient,
    cutoff_date: DateTime<Utc>
) -> Result<()> {
    let old_content = get_content_older_than(cutoff_date).await?;
    
    for content in old_content {
        // Move to cheaper storage
        archive_to_s3(&content).await?;
        
        // Unpin from premium IPFS service
        client.unpin(&content.cid).await?;
    }
    
    Ok(())
}
```

## 🧪 Testing Strategies

### Integration Testing

```rust
#[cfg(test)]
mod integration_tests {
    use super::*;
    
    #[tokio::test]
    async fn test_complete_farmer_workflow() {
        // Setup test environment
        let test_env = setup_test_environment().await;
        
        // 1. Register farmer
        let farmer_data = create_test_farmer_data();
        let registration = test_env.register_farmer(farmer_data).await.unwrap();
        
        // 2. Verify IPFS upload
        let ipfs_content = test_env.ipfs_client
            .get_json::<FarmerMetadata>(&registration.ipfs_cid)
            .await.unwrap();
        
        assert_eq!(ipfs_content.stage, "farmer_registration");
        
        // 3. Verify blockchain event
        let events = test_env.blockchain_client
            .get_farmer_registered_events()
            .await.unwrap();
        
        assert!(events.iter().any(|e| e.metadata_cid == registration.ipfs_cid));
        
        // 4. Verify database record
        let db_record = test_env.db
            .get_farmer_by_did(&registration.farmer_did)
            .await.unwrap();
        
        assert_eq!(db_record.ipfs_cid, registration.ipfs_cid);
    }
    
    #[tokio::test]
    async fn test_supply_chain_traceability() {
        let test_env = setup_test_environment().await;
        
        // Create complete supply chain flow
        let farmer_reg = test_env.register_test_farmer().await.unwrap();
        let fpo_purchase = test_env.fpo_purchase(farmer_reg.farmer_did).await.unwrap();
        let warehouse_storage = test_env.warehouse_store(fpo_purchase.batch_id).await.unwrap();
        let processing = test_env.process_batch(warehouse_storage.batch_id).await.unwrap();
        let packaging = test_env.package_sku(processing.output_batch_id).await.unwrap();
        
        // Verify complete traceability
        let trace = test_env.trace_sku(packaging.sku_id).await.unwrap();
        
        assert_eq!(trace.farmer_did, farmer_reg.farmer_did);
        assert_eq!(trace.steps.len(), 5); // All supply chain steps
        assert!(trace.steps.iter().all(|step| step.ipfs_cid.is_some()));
    }
}
```

### Load Testing

```javascript
// k6 load test script
import http from 'k6/http';
import { check, sleep } from 'k6';

export let options = {
  stages: [
    { duration: '2m', target: 10 }, // Ramp up
    { duration: '5m', target: 50 }, // Stay at 50 users
    { duration: '2m', target: 0 },  // Ramp down
  ],
};

export default function() {
  // Test farmer registration
  let farmerData = {
    name: `Test Farmer ${Math.random()}`,
    contact_info: { phone: "+91-9876543210", address: "Test Address" },
    land_details: { total_area: "5.2 hectares", soil_type: "Alluvial" },
    crop_info: { variety: "Mustard", expected_yield: "1500 kg" }
  };
  
  let response = http.post(
    'http://localhost:3000/api/ipfs/farmer',
    JSON.stringify(farmerData),
    { headers: { 'Content-Type': 'application/json' } }
  );
  
  check(response, {
    'farmer registration successful': (r) => r.status === 200,
    'response has CID': (r) => JSON.parse(r.body).cid !== undefined,
    'response time < 5s': (r) => r.timings.duration < 5000,
  });
  
  sleep(1);
}
```

## 📋 Deployment Checklist

### Pre-Production

- [ ] All environment variables configured
- [ ] IPFS services (Infura/Pinata) set up and tested
- [ ] Database migrations run successfully
- [ ] SSL certificates installed and configured
- [ ] Firewall rules configured
- [ ] Monitoring and alerting set up
- [ ] Backup procedures tested
- [ ] Load testing completed
- [ ] Security audit performed
- [ ] Documentation updated

### Production Deployment

```bash
# 1. Deploy IPFS infrastructure
cd ipfs-infrastructure
./deploy.sh production

# 2. Build and deploy Rust backend
cd ../offchain
cargo build --release
# Deploy to your preferred platform (AWS, GCP, etc.)

# 3. Deploy smart contracts
cd ..
forge script script/Deploy.s.sol:DeployScript --rpc-url $MAINNET_RPC_URL --private-key $DEPLOYER_KEY --broadcast

# 4. Run post-