# API Specification - Oilseed Value Chain Backend

## 🌐 Base URL

**Development**: `http://localhost:3000`  
**Production**: `https://your-domain.com` (to be configured)

---

## 🔑 Authentication

Currently, no authentication is required for API endpoints. In production, implement:
- JWT tokens for user authentication
- API keys for service-to-service communication
- Role-based access control (RBAC)

---

## 📋 Endpoints Overview

| Category | Endpoint | Method | Description |
|----------|----------|--------|-------------|
| **Health** | `/health` | GET | Health check |
| **Info** | `/` | GET | API information |
| **Workflow** | `/api/workflow/execute` | POST | Execute complete workflow |
| **Workflow** | `/api/workflow/verify-sku` | POST | Verify SKU traceability |
| **Workflow** | `/api/workflow/verify-farmer` | POST | Verify farmer registration |
| **Farmer** | `/api/farmer/register` | POST | Register new farmer |
| **Farmer** | `/api/farmer/verify` | POST | Verify farmer exists |
| **FPO** | `/api/fpo/purchase` | POST | Record FPO purchase |
| **Warehouse** | `/api/warehouse/update` | POST | Update warehouse state |
| **Warehouse** | `/api/warehouse/batch-update` | POST | Batch warehouse updates |
| **Logistics** | `/api/logistics/record` | POST | Record logistics milestone |
| **Processing** | `/api/processing/batch` | POST | Process batch |
| **Packaging** | `/api/packaging/sku` | POST | Create new SKU |
| **Packaging** | `/api/packaging/verify` | POST | Verify SKU origin |
| **Fraud** | `/api/fraud/report` | POST | Report fraud |
| **AI** | `/api/ai/commit` | POST | Commit AI score |
| **AI** | `/api/ai/reveal` | POST | Reveal AI score |
| **IPFS** | `/api/ipfs/upload` | POST | Upload data to IPFS |

---

## 📖 Detailed API Reference

### 1. Health Check

**Endpoint**: `GET /health`

**Description**: Check if the server is running

**Request**: None

**Response**: 
```json
"OK"
```

**Status Codes**:
- `200 OK` - Server is healthy

**Example**:
```bash
curl http://localhost:3000/health
```

---

### 2. API Information

**Endpoint**: `GET /`

**Description**: Get API version and status

**Request**: None

**Response**: 
```json
"Oilseed Value Chain Backend API v0.1.0 - All systems operational"
```

**Status Codes**:
- `200 OK` - Success

---

### 3. Execute Complete Workflow

**Endpoint**: `POST /api/workflow/execute`

**Description**: Execute the entire supply chain workflow from farmer to retail in one request

**Request Body**:
```json
{
  "farmer": {
    "farmer_did": "0x0000000000000000000000000000000000000000000000000000000000000001",
    "crop_id": "MUSTARD-2024-001",
    "name": "Rajesh Kumar",
    "location": "Ludhiana, Punjab, India",
    "land_area": "10 acres",
    "crops": ["mustard", "wheat"],
    "contact": "+91-98765-43210"
  },
  "fpo_purchase": {
    "batch_id": "BATCH-MUSTARD-2024-001",
    "quantity_kg": 5000.0,
    "quality_grade": "A",
    "moisture_content": "7.5%",
    "purchase_price": 45000.0,
    "purchase_date": "2024-01-15"
  },
  "warehouse": {
    "warehouse_id": "WH-PUNJAB-001",
    "temperature_celsius": 22.5,
    "humidity_percent": 45.0,
    "storage_duration_days": 30
  },
  "logistics": {
    "shipment_id": "SHIP-2024-001",
    "origin": "Ludhiana, Punjab",
    "destination": "Indore Processing Plant, MP",
    "checkpoints": [
      {
        "location": "Ludhiana Warehouse",
        "latitude": 30.9010,
        "longitude": 75.8573,
        "timestamp": "2024-02-15T08:00:00Z"
      }
    ]
  },
  "processing": {
    "input_batch_id": "BATCH-MUSTARD-2024-001",
    "process_type": "Cold Press Extraction",
    "yield_percentage": 42.5,
    "output_products": [
      {
        "product_id": "MUSTARD-OIL-2024-001",
        "product_type": "Mustard Oil",
        "quantity_kg": 2125.0
      }
    ]
  },
  "packaging": {
    "sku_prefix": "MUSTARDOIL-2024",
    "package_type": "1L PET Bottle",
    "units_per_package": 12,
    "total_packages": 5,
    "expiry_months": 12
  },
  "ai_scoring": {
    "quality_score": 8.5,
    "freshness_score": 9.2,
    "purity_score": 8.8,
    "model_version": "v1.2.3"
  }
}
```

**Response**:
```json
{
  "farmer_tx": "0xabc123...",
  "fpo_tx": "0xdef456...",
  "warehouse_tx": "0x789...",
  "logistics_txs": ["0x111...", "0x222..."],
  "processing_tx": "0x333...",
  "packaging_txs": ["0x444...", "0x555..."],
  "ai_commit_tx": "0x666...",
  "ai_reveal_tx": "0x777...",
  "ipfs_cids": {
    "farmer_metadata": "QmFarmer123...",
    "fpo_metadata": "QmFPO456...",
    "warehouse_metadata": "QmWarehouse789...",
    "logistics_metadata": ["QmLog1...", "QmLog2..."],
    "processing_metadata": "QmProc123...",
    "packaging_metadata": ["QmPkg1...", "QmPkg2..."],
    "ai_metadata": "QmAI123..."
  },
  "final_skus": [
    "MUSTARDOIL-2024-0001",
    "MUSTARDOIL-2024-0002",
    "MUSTARDOIL-2024-0003",
    "MUSTARDOIL-2024-0004",
    "MUSTARDOIL-2024-0005"
  ],
  "summary": {
    "total_stages": 7,
    "successful_stages": 7,
    "total_transactions": 15,
    "total_ipfs_uploads": 12,
    "workflow_duration_secs": 45,
    "trace_path": "Farmer(0x01) → FPO(BATCH-001) → Warehouse(WH-001) → ..."
  }
}
```

**Status Codes**:
- `200 OK` - Workflow completed successfully
- `400 Bad Request` - Invalid input data
- `500 Internal Server Error` - Blockchain or IPFS error

---

### 4. Verify SKU Traceability

**Endpoint**: `POST /api/workflow/verify-sku`

**Description**: Verify the authenticity and trace the complete supply chain history of a product SKU

**Request Body**:
```json
{
  "sku_id": "MUSTARDOIL-2024-0001"
}
```

**Response**:
```json
{
  "sku_id": "MUSTARDOIL-2024-0001",
  "parent_batch_hash": "0xabc123...",
  "merkle_root": "0xdef456...",
  "packaged_at": 1709366400,
  "verified": true,
  "trace_summary": "SKU MUSTARDOIL-2024-0001 → Batch 0xabc... → Packaged at 1709366400"
}
```

**Status Codes**:
- `200 OK` - SKU verified successfully
- `400 Bad Request` - Invalid SKU ID
- `500 Internal Server Error` - Verification failed

---

### 5. Verify Farmer Registration

**Endpoint**: `POST /api/workflow/verify-farmer`

**Description**: Check if a farmer is registered on the blockchain

**Request Body**:
```json
{
  "farmer_did": "0x0000000000000000000000000000000000000000000000000000000000000001"
}
```

**Response**:
```json
{
  "farmer_did": "0x0000000000000000000000000000000000000000000000000000000000000001",
  "exists": true,
  "crop_id_hash": "0x123abc...",
  "registered_at": 1704067200
}
```

**Status Codes**:
- `200 OK` - Verification successful
- `400 Bad Request` - Invalid farmer DID format
- `500 Internal Server Error` - Verification failed

---

### 6. Register Farmer

**Endpoint**: `POST /api/farmer/register`

**Description**: Register a new farmer with metadata stored on IPFS

**Request Body**:
```json
{
  "farmer_did": "0x0000000000000000000000000000000000000000000000000000000000000001",
  "crop_id": "MUSTARD-2024-001",
  "metadata": {
    "name": "Rajesh Kumar",
    "location": "Ludhiana, Punjab, India",
    "land_area": "10 acres",
    "crops": ["mustard", "wheat", "cotton"],
    "contact": "+91-98765-43210",
    "bank_account": "PUNB0001234567890",
    "documents": {
      "land_ownership": "ipfs://QmLandDeed...",
      "kyc_verification": "ipfs://QmKYC..."
    }
  }
}
```

**Response**:
```json
{
  "tx_hash": "0xabc123...",
  "farmer_did": "0x0000000000000000000000000000000000000000000000000000000000000001",
  "crop_id_hash": "0xdef456...",
  "metadata_cid": "QmFarmer123...",
  "ipfs_url": "https://ipfs.io/ipfs/QmFarmer123..."
}
```

**Status Codes**:
- `200 OK` - Farmer registered successfully
- `400 Bad Request` - Invalid input or farmer already registered
- `500 Internal Server Error` - Registration failed

---

### 7. Record FPO Purchase

**Endpoint**: `POST /api/fpo/purchase`

**Description**: Record a purchase transaction from FPO to farmer

**Request Body**:
```json
{
  "batch_id": "BATCH-2024-001",
  "farmer_did": "0x0000000000000000000000000000000000000000000000000000000000000001",
  "metadata": {
    "quantity_kg": 5000,
    "quality_grade": "A",
    "moisture_content": "7.5%",
    "purchase_price": 45000,
    "photos": ["photo1.jpg", "photo2.jpg"]
  }
}
```

**Response**:
```json
{
  "tx_hash": "0xabc123...",
  "batch_hash": "0xdef456...",
  "metadata_cid": "QmBatch123...",
  "ipfs_url": "https://ipfs.io/ipfs/QmBatch123..."
}
```

**Status Codes**:
- `200 OK` - Purchase recorded successfully
- `400 Bad Request` - Invalid input or farmer not registered
- `500 Internal Server Error` - Transaction failed

---

### 8. Update Warehouse State

**Endpoint**: `POST /api/warehouse/update`

**Description**: Update warehouse IoT sensor data

**Request Body**:
```json
{
  "warehouse_id": "WH-001",
  "iot_data": {
    "temperature_celsius": 22.5,
    "humidity_percent": 45,
    "timestamp": "2024-01-15T10:30:00Z",
    "sensor_id": "SENSOR-001"
  }
}
```

**Response**:
```json
{
  "tx_hash": "0xabc123...",
  "warehouse_id": "WH-001",
  "state_hash": "0xdef456...",
  "metadata_cid": "QmWarehouse123...",
  "ipfs_url": "https://ipfs.io/ipfs/QmWarehouse123..."
}
```

**Status Codes**:
- `200 OK` - State updated successfully
- `400 Bad Request` - Invalid input
- `500 Internal Server Error` - Update failed

---

### 9. Batch Update Warehouses

**Endpoint**: `POST /api/warehouse/batch-update`

**Description**: Update multiple warehouses in a single transaction (gas efficient)

**Request Body**:
```json
{
  "updates": [
    {
      "warehouse_id": "WH-001",
      "iot_data": {
        "temperature_celsius": 22,
        "humidity_percent": 45
      }
    },
    {
      "warehouse_id": "WH-002",
      "iot_data": {
        "temperature_celsius": 23,
        "humidity_percent": 43
      }
    }
  ]
}
```

**Response**:
```json
{
  "tx_hash": "0xabc123...",
  "message": "Successfully updated 2 warehouses"
}
```

**Status Codes**:
- `200 OK` - Batch update successful
- `400 Bad Request` - Invalid input
- `500 Internal Server Error` - Update failed

---

### 10. Record Logistics Milestone

**Endpoint**: `POST /api/logistics/record`

**Description**: Record a logistics checkpoint

**Request Body**:
```json
{
  "shipment_id": "SHIP-2024-001",
  "location": "Delhi Hub",
  "is_delivered": false,
  "gps_data": {
    "latitude": 28.7041,
    "longitude": 77.1025,
    "timestamp": "2024-01-15T14:30:00Z",
    "driver": "Driver Name",
    "vehicle": "MH-01-AB-1234"
  }
}
```

**Response**:
```json
{
  "tx_hash": "0xabc123...",
  "shipment_id": "SHIP-2024-001",
  "location_hash": "0xdef456...",
  "metadata_cid": "QmLogistics123...",
  "ipfs_url": "https://ipfs.io/ipfs/QmLogistics123..."
}
```

**Status Codes**:
- `200 OK` - Milestone recorded successfully
- `400 Bad Request` - Invalid input
- `500 Internal Server Error` - Recording failed

---

### 11. Process Batch

**Endpoint**: `POST /api/processing/batch`

**Description**: Record batch processing and transformation

**Request Body**:
```json
{
  "input_batch_id": "BATCH-2024-001",
  "output_batch_ids": ["OIL-2024-001", "CAKE-2024-001"],
  "process_metadata": {
    "yield_percentage": 42.5,
    "oil_quality": "Premium",
    "lab_results": "All parameters within limits",
    "process_date": "2024-01-16T09:00:00Z"
  }
}
```

**Response**:
```json
{
  "tx_hash": "0xabc123...",
  "input_batch_hash": "0xdef456...",
  "transform_hash": "0x789...",
  "output_batch_hashes": ["0x111...", "0x222..."],
  "metadata_cid": "QmProcessing123...",
  "ipfs_url": "https://ipfs.io/ipfs/QmProcessing123..."
}
```

**Status Codes**:
- `200 OK` - Processing recorded successfully
- `400 Bad Request` - Invalid input
- `500 Internal Server Error` - Recording failed

---

### 12. Create SKU

**Endpoint**: `POST /api/packaging/sku`

**Description**: Create a new retail SKU with Merkle tree verification

**Request Body**:
```json
{
  "sku_id": "SKU-2024-001",
  "parent_batch_id": "OIL-2024-001",
  "unit_ids": ["UNIT-001", "UNIT-002", "UNIT-003"],
  "packaging_metadata": {
    "package_type": "1L Bottle",
    "units_count": 1000,
    "barcode": "1234567890123",
    "expiry_date": "2025-01-15"
  }
}
```

**Response**:
```json
{
  "tx_hash": "0xabc123...",
  "sku_id": "SKU-2024-001",
  "parent_batch_hash": "0xdef456...",
  "merkle_root": "0x789...",
  "metadata_cid": "QmPackaging123...",
  "ipfs_url": "https://ipfs.io/ipfs/QmPackaging123..."
}
```

**Status Codes**:
- `200 OK` - SKU created successfully
- `400 Bad Request` - Invalid input or SKU already exists
- `500 Internal Server Error` - Creation failed

---

### 13. Verify SKU (Alternative)

**Endpoint**: `POST /api/packaging/verify`

**Description**: Verify SKU origin (same as workflow verify-sku but different endpoint)

**Request Body**:
```json
{
  "sku_id": "SKU-2024-001"
}
```

**Response**:
```json
{
  "sku_id": "SKU-2024-001",
  "parent_batch_hash": "0xabc123...",
  "merkle_root": "0xdef456...",
  "packaged_at": 1709366400,
  "exists": true
}
```

**Status Codes**:
- `200 OK` - Verification successful
- `400 Bad Request` - Invalid SKU ID
- `500 Internal Server Error` - Verification failed

---

### 14. Report Fraud

**Endpoint**: `POST /api/fraud/report`

**Description**: Report fraudulent product (public endpoint, anyone can report)

**Request Body**:
```json
{
  "sku_id": "SKU-2024-001",
  "evidence": {
    "reporter_name": "John Consumer",
    "complaint": "Product appears tampered",
    "photos": ["evidence1.jpg", "evidence2.jpg"],
    "lab_report": "contamination_detected.pdf",
    "timestamp": "2024-01-20T10:00:00Z"
  }
}
```

**Response**:
```json
{
  "tx_hash": "0xabc123...",
  "sku_id": "SKU-2024-001",
  "evidence_hash": "0xdef456...",
  "evidence_cid": "QmEvidence123...",
  "ipfs_url": "https://ipfs.io/ipfs/QmEvidence123..."
}
```

**Status Codes**:
- `200 OK` - Fraud reported successfully
- `400 Bad Request` - Invalid SKU ID or evidence
- `500 Internal Server Error` - Reporting failed

---

### 15. Commit AI Score

**Endpoint**: `POST /api/ai/commit`

**Description**: Commit AI quality score (commit phase of commit-reveal pattern)

**Request Body**:
```json
{
  "batch_id": "BATCH-2024-001",
  "reveal_hash": "0xabc123...",
  "nonce": "0xdef456..."
}
```

**Response**:
```json
{
  "tx_hash": "0x789...",
  "batch_id": "BATCH-2024-001",
  "commit_hash": "0x111..."
}
```

**Status Codes**:
- `200 OK` - Score committed successfully
- `400 Bad Request` - Invalid input or already committed
- `500 Internal Server Error` - Commit failed

---

### 16. Reveal AI Score

**Endpoint**: `POST /api/ai/reveal`

**Description**: Reveal AI quality score (must wait 1+ hour after commit)

**Request Body**:
```json
{
  "batch_id": "BATCH-2024-001",
  "nonce": "0xdef456...",
  "score_data": {
    "quality_score": 8.5,
    "freshness_score": 9.2,
    "purity_score": 8.8,
    "model_version": "v1.2.3",
    "confidence": 0.95
  }
}
```

**Response**:
```json
{
  "tx_hash": "0xabc123...",
  "batch_id": "BATCH-2024-001",
  "reveal_hash": "0xdef456...",
  "metadata_cid": "QmAI123...",
  "ipfs_url": "https://ipfs.io/ipfs/QmAI123..."
}
```

**Status Codes**:
- `200 OK` - Score revealed successfully
- `400 Bad Request` - Invalid nonce or too early to reveal
- `500 Internal Server Error` - Reveal failed

---

### 17. Upload to IPFS

**Endpoint**: `POST /api/ipfs/upload`

**Description**: Direct IPFS upload (for arbitrary JSON data)

**Request Body**:
```json
{
  "data": {
    "arbitrary": "json",
    "can": "be uploaded",
    "nested": {
      "data": "supported"
    }
  }
}
```

**Response**:
```json
{
  "cid": "QmUpload123...",
  "gateway_url": "https://ipfs.io/ipfs/QmUpload123..."
}
```

**Status Codes**:
- `200 OK` - Upload successful
- `400 Bad Request` - Invalid JSON
- `500 Internal Server Error` - IPFS upload failed

---

## 🔄 Common Response Patterns

### Success Response
```json
{
  "tx_hash": "0x...",
  "...": "other fields"
}
```

### Error Response
```json
{
  "error": "Descriptive error message"
}
```

---

## 📊 HTTP Status Codes

| Code | Meaning | When Used |
|------|---------|-----------|
| `200` | OK | Request successful |
| `400` | Bad Request | Invalid input data |
| `404` | Not Found | Endpoint doesn't exist |
| `500` | Internal Server Error | Server/blockchain/IPFS error |

---

## 🎯 Frontend Integration Examples

### React/TypeScript Example

```typescript
// API client configuration
const API_BASE_URL = 'http://localhost:3000';

// Type definitions
interface FarmerRegistrationRequest {
  farmer_did: string;
  crop_id: string;
  metadata: {
    name: string;
    location: string;
    land_area: string;
    crops: string[];
    contact: string;
  };
}

interface FarmerRegistrationResponse {
  tx_hash: string;
  farmer_did: string;
  crop_id_hash: string;
  metadata_cid: string;
  ipfs_url: string;
}

// API function
async function registerFarmer(
  data: FarmerRegistrationRequest
): Promise<FarmerRegistrationResponse> {
  const response = await fetch(`${API_BASE_URL}/api/farmer/register`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(data),
  });

  if (!response.ok) {
    const error = await response.json();
    throw new Error(error.error || 'Failed to register farmer');
  }

  return response.json();
}

// Usage
try {
  const result = await registerFarmer({
    farmer_did: '0x0000...0001',
    crop_id: 'MUSTARD-2024-001',
    metadata: {
      name: 'Rajesh Kumar',
      location: 'Punjab, India',
      land_area: '10 acres',
      crops: ['mustard', 'wheat'],
      contact: '+91-98765-43210',
    },
  });
  
  console.log('Farmer registered!', result.tx_hash);
  console.log('IPFS URL:', result.ipfs_url);
} catch (error) {
  console.error('Registration failed:', error.message);
}
```

### JavaScript (Vanilla) Example

```javascript
// Execute complete workflow
async function executeWorkflow(workflowData) {
  try {
    const response = await fetch('http://localhost:3000/api/workflow/execute', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(workflowData),
    });

    if (!response.ok) {
      const error = await response.json();
      throw new Error(error.error);
    }

    const result = await response.json();
    console.log('Workflow completed successfully!');
    console.log('SKUs created:', result.final_skus);
    console.log('Total transactions:', result.summary.total_transactions);
    
    return result;
  } catch (error) {
    console.error('Workflow failed:', error.message);
    throw error;
  }
}
```

### Axios Example

```javascript
import axios from 'axios';

const api = axios.create({
  baseURL: 'http://localhost:3000',
  headers: {
    'Content-Type': 'application/json',
  },
});

// Verify SKU
async function verifySKU(skuId) {
  try {
    const { data } = await api.post('/api/workflow/verify-sku', {
      sku_id: skuId,
    });
    
    if (data.verified) {
      console.log('✅ SKU is authentic!');
      console.log('Parent batch:', data.parent_batch_hash);
      console.log('Packaged at:', new Date(data.packaged_at * 1000));
    } else {
      console.log('❌ SKU not found or invalid');
    }
    
    return data;
  } catch (error) {
    console.error('Verification failed:', error.response?.data?.error || error.message);
    throw error;
  }
}
```

---

## 🔍 Testing the API

### Using cURL

```bash
# Health check
curl http://localhost:3000/health

# Register farmer
curl -X POST http://localhost:3000/api/farmer/register \
  -H "Content-Type: application/json" \
  -d @test_data/farmer.json

# Verify SKU
curl -X POST http://localhost:3000/api/workflow/verify-sku \
  -H "Content-Type: application/json" \
  -d '{"sku_id": "MUSTARDOIL-2024-0001"}'
```

### Using Postman

1. Import the following collection URL (if available)
2. Or manually create requests using the endpoints above
3. Set base URL as environment variable: `{{baseUrl}} = http://localhost:3000`

---

## 📝 Notes for Frontend Developers

### Important Considerations

1. **Transaction Hashes**: All `tx_hash` fields are Ethereum transaction hashes. Use them to:
   - Track transaction status
   - Show transaction on block explorer
   - Verify transaction completion

2. **IPFS CIDs**: All `metadata_cid` and `*_cid` fields are IPFS content identifiers:
   - Use to fetch full metadata: `https://ipfs.io/ipfs/{cid}`
   - Can use other IPFS gateways if needed
   - Consider caching IPFS data on frontend

3. **Timestamps**: All `*_at` fields are Unix timestamps (seconds since epoch)
   - Convert to Date: `new Date(timestamp * 1000)`

4. **Hashes**: All `*_hash` fields are 32-byte hex strings starting with `0x`
   - Display first/last 6 characters: `0xabc...xyz`

5. **Error Handling**: Always check response status and handle errors
   - 400: Show validation errors to user
   - 500: Show "Try again later" message

### Recommended Flow for Frontend

**For Admin/Operator:**
1. Use individual endpoints for each stage
2. Show progress after each operation
3. Display transaction hashes and IPFS links

**For Consumers:**
1. Use verification endpoints only
2. Show QR code scanner
3. Display complete supply chain trace

**For Automation:**
1. Use complete workflow endpoint
2. Show summary at the end
3. Provide download of all transaction details

---

## 🚀 Getting Started Checklist

- [ ] Update `API_BASE_URL` to your backend URL
- [ ] Test health endpoint first
- [ ] Implement error handling for all requests
- [ ] Add loading states for async operations
- [ ] Cache IPFS data where possible
- [ ] Show transaction hashes as links to block explorer
- [ ] Implement retry logic for failed requests
- [ ] Add request timeout handling
- [ ] Test with sample data from `test_data/` folder

---

## 📞 Support

Questions about the API?
- Check `README.md` for setup instructions
- Check `WORKFLOW.md` for workflow details
- Check `INTEGRATION.md` for architecture
- Open an issue on GitHub

---

**API Version**: v0.1.0  
**Last Updated**: 2024-01-XX  
**Maintained By**: Backend Team