# IPFS Infrastructure Architecture

## Overview

This document outlines the production-ready IPFS architecture for the Oilseed Value Chain system, designed to integrate seamlessly with the Solidity smart contracts and Rust backend.

## Architecture Components

```
┌─────────────────────────────────────────────────────────────────┐
│                    IPFS DEPLOYMENT ARCHITECTURE                 │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌─────────────────┐    ┌─────────────────┐    ┌─────────────┐ │
│  │   Client Apps   │    │   Web Frontend  │    │  Mobile App │ │
│  └─────────────────┘    └─────────────────┘    └─────────────┘ │
│            │                       │                       │    │
│            └───────────────────────┼───────────────────────┘    │
│                                    │                            │
│  ┌─────────────────────────────────┼────────────────────────┐   │
│  │              RUST BACKEND       │                        │   │
│  │  ┌─────────────────────────────┐│┌─────────────────────┐ │   │
│  │  │     API Gateway/Router      │││   IPFS Client       │ │   │
│  │  │  - Upload endpoints         │││  - Content upload   │ │   │
│  │  │  - Retrieval endpoints      │││  - CID resolution   │ │   │
│  │  │  - Metadata validation      │││  - Pinning mgmt     │ │   │
│  │  └─────────────────────────────┘│└─────────────────────┘ │   │
│  └─────────────────────────────────┼────────────────────────┘   │
│                                    │                            │
│  ┌─────────────────────────────────┼────────────────────────┐   │
│  │            IPFS LAYER           │                        │   │
│  │                                 │                        │   │
│  │  PRIMARY: Infura IPFS          │  BACKUP: Pinata        │   │
│  │  ┌─────────────────────────┐   │  ┌───────────────────┐ │   │
│  │  │  - API endpoint         │   │  │  - API endpoint   │ │   │
│  │  │  - Auto-pinning         │   │  │  - Manual pinning │ │   │
│  │  │  - CDN integration      │   │  │  - Gateway access │ │   │
│  │  └─────────────────────────┘   │  └───────────────────┘ │   │
│  │                                 │                        │   │
│  │  DEVELOPMENT: Local IPFS Node  │  OPTIONAL: Self-hosted │   │
│  │  ┌─────────────────────────┐   │  ┌───────────────────┐ │   │
│  │  │  - Docker container     │   │  │  - Full IPFS node│ │   │
│  │  │  - Local development    │   │  │  - High throughput│ │   │
│  │  └─────────────────────────┘   │  └───────────────────┘ │   │
│  └─────────────────────────────────┼────────────────────────┘   │
│                                    │                            │
│  ┌─────────────────────────────────┼────────────────────────┐   │
│  │           BLOCKCHAIN LAYER      │                        │   │
│  │                                 │                        │   │
│  │  ┌─────────────────────────────┐│                       │   │
│  │  │   OilseedValueChain.sol     ││                       │   │
│  │  │                             ││                       │   │
│  │  │  Events with IPFS CIDs:     ││                       │   │
│  │  │  - FarmerRegistered         ││                       │   │
│  │  │  - OwnershipTransfer        ││                       │   │
│  │  │  - WarehouseStateUpdated    ││                       │   │
│  │  │  - LogisticsMilestone       ││                       │   │
│  │  │  - BatchProcessed           ││                       │   │
│  │  │  - SKUPackaged              ││                       │   │
│  │  │  - AIScoreRevealed          ││                       │   │
│  │  │  - FraudDetected            ││                       │   │
│  │  └─────────────────────────────┘│                       │   │
│  └─────────────────────────────────┼────────────────────────┘   │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

## Data Structure Design

### 1. Standardized JSON Schemas

Each supply chain stage has a standardized JSON schema stored on IPFS:

#### Farmer Registration Metadata
```json
{
  "version": "1.0",
  "timestamp": "2024-01-15T10:30:00Z",
  "stage": "farmer_registration",
  "farmerProfile": {
    "name": "Rajesh Kumar",
    "contactInfo": {
      "phone": "+91-9876543210",
      "address": "Village Khajuraho, Chhatarpur, MP"
    },
    "landDetails": {
      "totalArea": "5.2 hectares",
      "soilType": "Alluvial",
      "coordinates": [25.837, 79.919],
      "documents": ["land_deed_hash", "soil_report_hash"]
    },
    "cropInfo": {
      "variety": "Mustard",
      "plantingDate": "2024-01-10",
      "expectedYield": "1500 kg"
    }
  },
  "verification": {
    "documentHashes": ["0x123...", "0x456..."],
    "satelliteImagery": "satellite_image_cid",
    "certifications": ["organic_cert_cid"]
  }
}
```

#### FPO Purchase Metadata
```json
{
  "version": "1.0",
  "timestamp": "2024-03-15T14:20:00Z",
  "stage": "fpo_purchase",
  "transaction": {
    "batchId": "BATCH_2024_001",
    "quantity": "1200 kg",
    "pricePerKg": "45.50 INR",
    "totalAmount": "54600 INR",
    "qualityGrade": "A+",
    "moistureContent": "8.5%"
  },
  "documents": {
    "receipt": "receipt_scan_cid",
    "qualityReport": "quality_report_cid",
    "weighmentSlip": "weight_slip_cid",
    "photos": ["batch_photo_1_cid", "batch_photo_2_cid"]
  },
  "fpoInfo": {
    "name": "Bundelkhand FPO Ltd",
    "license": "FPO_LICENSE_123",
    "address": "Chhatarpur, Madhya Pradesh"
  }
}
```

#### Warehouse Storage Metadata
```json
{
  "version": "1.0",
  "timestamp": "2024-03-16T09:00:00Z",
  "stage": "warehouse_storage",
  "warehouseInfo": {
    "id": "WH_001_CHHATARPUR",
    "location": [25.837, 79.919],
    "capacity": "10000 MT",
    "certifications": ["fssai_cert_cid", "warehouse_license_cid"]
  },
  "storageDetails": {
    "batchId": "BATCH_2024_001",
    "storageUnit": "BIN_A_001",
    "entryDate": "2024-03-16",
    "fumigation": {
      "done": true,
      "date": "2024-03-17",
      "certificate": "fumigation_cert_cid"
    }
  },
  "iotData": {
    "rollupPeriod": "24h",
    "averages": {
      "temperature": "22.5°C",
      "humidity": "60%",
      "airQuality": "Good"
    },
    "alerts": [],
    "rawDataCid": "iot_raw_data_cid"
  }
}
```

#### Logistics Tracking Metadata
```json
{
  "version": "1.0",
  "timestamp": "2024-03-20T11:30:00Z",
  "stage": "logistics",
  "shipment": {
    "id": "SHIP_2024_001",
    "vehicle": {
      "number": "MP09AB1234",
      "type": "Truck",
      "driver": "Amit Singh",
      "license": "DL123456"
    },
    "route": {
      "origin": "Chhatarpur Warehouse",
      "destination": "Processing Unit Delhi",
      "plannedRoute": "route_plan_cid",
      "estimatedDuration": "8 hours"
    }
  },
  "tracking": {
    "currentLocation": [25.837, 79.919],
    "lastUpdated": "2024-03-20T11:30:00Z",
    "milestones": [
      {
        "location": "Chhatarpur Exit",
        "timestamp": "2024-03-20T08:00:00Z",
        "status": "departed"
      }
    ],
    "fullGpsTrace": "gps_trace_cid",
    "temperatureLog": "temp_log_cid"
  }
}
```

#### Processing Metadata
```json
{
  "version": "1.0",
  "timestamp": "2024-03-25T16:45:00Z",
  "stage": "processing",
  "processing": {
    "facilityId": "PROC_DELHI_001",
    "license": "food_processing_license_cid",
    "inputBatch": "BATCH_2024_001",
    "process": {
      "type": "Oil Extraction",
      "method": "Cold Press",
      "temperature": "40°C",
      "yield": "35%"
    },
    "outputs": [
      {
        "product": "Mustard Oil",
        "batchId": "OIL_BATCH_001",
        "quantity": "420L",
        "quality": {
          "acidity": "0.8%",
          "peroxideValue": "2.5",
          "labReport": "lab_report_oil_cid"
        }
      },
      {
        "product": "Oil Cake",
        "batchId": "CAKE_BATCH_001",
        "quantity": "780kg",
        "protein": "38%",
        "labReport": "lab_report_cake_cid"
      }
    ]
  }
}
```

#### Packaging Metadata
```json
{
  "version": "1.0",
  "timestamp": "2024-03-30T10:15:00Z",
  "stage": "packaging",
  "packaging": {
    "parentBatch": "OIL_BATCH_001",
    "facility": "PACK_FACILITY_001",
    "packagingDate": "2024-03-30",
    "units": [
      {
        "skuId": "SKU_MUSTARD_1L_001",
        "size": "1L",
        "type": "PET Bottle",
        "label": "label_design_cid",
        "barcodeData": "barcode_data",
        "qrCode": "https://verify.oilseed.com/SKU_MUSTARD_1L_001"
      }
    ],
    "qualityChecks": {
      "visualInspection": "passed",
      "weightCheck": "passed",
      "sealIntegrity": "passed",
      "photos": ["package_photo_cid"]
    },
    "merkleTree": {
      "root": "0xabc123...",
      "proofs": "merkle_proofs_cid"
    }
  }
}
```

#### AI Scoring Metadata
```json
{
  "version": "1.0",
  "timestamp": "2024-04-01T14:20:00Z",
  "stage": "ai_scoring",
  "aiAnalysis": {
    "model": {
      "version": "oilseed_quality_v2.1",
      "hash": "model_hash_123"
    },
    "inputs": {
      "farmerData": "farmer_profile_cid",
      "warehouseData": "warehouse_iot_cid",
      "processingData": "processing_params_cid",
      "labResults": "lab_results_cid"
    },
    "outputs": {
      "qualityScore": 8.7,
      "trustScore": 9.2,
      "recommendations": [
        "Maintain current storage conditions",
        "Consider premium pricing tier"
      ],
      "explainability": {
        "factors": {
          "storage_conditions": 0.25,
          "processing_quality": 0.30,
          "source_verification": 0.25,
          "lab_results": 0.20
        },
        "details": "ai_explanation_cid"
      }
    }
  }
}
```

## IPFS Infrastructure Options

### Option 1: Infura IPFS (Recommended for Production)

**Advantages:**
- Managed service with high uptime
- Built-in CDN and global distribution
- Automatic pinning and persistence
- Easy integration with existing Web3 stack

**Configuration:**
```env
IPFS_API_URL=https://ipfs.infura.io:5001
IPFS_PROJECT_ID=your_project_id
IPFS_PROJECT_SECRET=your_project_secret
IPFS_GATEWAY_URL=https://ipfs.io/ipfs/
```

### Option 2: Pinata (Alternative Production Option)

**Advantages:**
- Dedicated IPFS pinning service
- Advanced metadata and search features
- Competitive pricing
- Good developer experience

**Configuration:**
```env
IPFS_API_URL=https://api.pinata.cloud
IPFS_PROJECT_ID=your_api_key
IPFS_PROJECT_SECRET=your_secret_key
IPFS_GATEWAY_URL=https://gateway.pinata.cloud/ipfs/
```

### Option 3: Self-Hosted IPFS Node

**Advantages:**
- Full control over data
- No third-party dependencies
- Customizable configuration
- Cost-effective for high volume

**Setup:**
- Docker-based deployment
- Cluster configuration for redundancy
- Custom gateway and API endpoints

### Option 4: Hybrid Approach (Recommended)

**Strategy:**
- Primary: Infura/Pinata for production reliability
- Secondary: Self-hosted node for redundancy
- Development: Local IPFS node for testing

## Deployment Strategy

### 1. Environment Configuration

Create environment-specific configurations:

- **Development:** Local IPFS node + Mock server
- **Staging:** Infura testnet + backup pinning
- **Production:** Infura mainnet + Pinata backup + self-hosted failover

### 2. Content Pinning Strategy

**Automatic Pinning:**
- All uploaded content pinned immediately
- Redundant pinning across multiple services
- Monitoring and alerts for failed pins

**Pin Management:**
- Regular pin status checks
- Automatic re-pinning for critical content
- Garbage collection for temporary data

### 3. Performance Optimization

**Content Delivery:**
- CDN integration for faster retrieval
- Regional gateway optimization
- Caching strategy for frequently accessed content

**Upload Optimization:**
- Chunking for large files
- Parallel uploads for redundancy
- Progress tracking and resumable uploads

## Integration Points

### 1. Rust Backend Integration

The IPFS client in your Rust backend should:
- Support multiple IPFS providers
- Implement fallback mechanisms
- Provide content validation
- Handle large file uploads efficiently

### 2. Solidity Contract Integration

Your smart contract events already include IPFS CID fields:
- `metadataCID` in FarmerRegistered
- `metadataCID` in OwnershipTransfer
- `metadataCID` in WarehouseStateUpdated
- `evidenceCID` in FraudDetected

### 3. Frontend Integration

Web and mobile clients should:
- Access content through your Rust backend (not directly from IPFS)
- Implement content caching
- Provide offline access to critical data

## Security Considerations

### 1. Content Validation

- Hash verification for all uploaded content
- Schema validation for JSON metadata
- File type and size restrictions

### 2. Access Control

- Authentication for upload endpoints
- Role-based access to sensitive data
- Audit logging for all operations

### 3. Privacy

- Encryption for sensitive data before IPFS upload
- Public/private content segregation
- GDPR compliance for personal data

## Monitoring and Observability

### 1. Health Checks

- IPFS node connectivity
- Pin status verification
- Gateway response times

### 2. Metrics

- Upload/download volumes
- Pin success/failure rates
- Gateway performance metrics

### 3. Alerting

- Failed pinning operations
- High error rates
- Performance degradation

## Cost Optimization

### 1. Data Lifecycle Management

- Archive old data to cheaper storage
- Implement data retention policies
- Optimize pin duration based on usage

### 2. Traffic Management

- Implement caching to reduce gateway costs
- Use compression for large JSON files
- Batch operations where possible

## Disaster Recovery

### 1. Backup Strategy

- Multiple pinning services
- Regular content audits
- Geographic distribution of pins

### 2. Recovery Procedures

- Automated failover to backup services
- Content restoration procedures
- Data integrity verification

This architecture provides a robust, scalable, and production-ready IPFS infrastructure that integrates seamlessly with your existing Solidity contracts and Rust backend.