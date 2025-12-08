//! JSON Schema Definitions for All Supply Chain Stages
//!
//! This module contains the actual schema definitions that are:
//! - Used for validation before IPFS upload
//! - Stored on IPFS themselves for reference
//! - Versioned for forward compatibility

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{SchemaDefinition, SchemaReference, SupplyChainStage};

/// Generate the JSON schema definition for a given supply chain stage
pub fn get_schema_definition(stage: SupplyChainStage) -> SchemaDefinition {
    match stage {
        SupplyChainStage::FarmerRegistration => farmer_registration_schema(),
        SupplyChainStage::FpoPurchase => fpo_purchase_schema(),
        SupplyChainStage::WarehouseStorage => warehouse_storage_schema(),
        SupplyChainStage::Logistics => logistics_schema(),
        SupplyChainStage::Processing => processing_schema(),
        SupplyChainStage::Packaging => packaging_schema(),
        SupplyChainStage::AiScoring => ai_scoring_schema(),
        SupplyChainStage::FraudReport => fraud_report_schema(),
    }
}

/// Get all schema definitions
pub fn get_all_schemas() -> Vec<SchemaDefinition> {
    vec![
        farmer_registration_schema(),
        fpo_purchase_schema(),
        warehouse_storage_schema(),
        logistics_schema(),
        processing_schema(),
        packaging_schema(),
        ai_scoring_schema(),
        fraud_report_schema(),
    ]
}

// ============================================================================
// FARMER REGISTRATION SCHEMA (v1.0.0)
// ============================================================================

fn farmer_registration_schema() -> SchemaDefinition {
    SchemaDefinition {
        schema: SchemaReference::new("farmer_registration", "1.0.0"),
        description: "Schema for farmer registration data including profile, land details, and crop information. PII fields are marked and must be encrypted before storage.".to_string(),
        json_schema: json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "$id": "https://oilseed.chain/schemas/farmer_registration/1.0.0",
            "type": "object",
            "required": ["farmer_id", "registration_date", "land_details", "crop_info", "verification_status"],
            "properties": {
                "farmer_id": {
                    "type": "string",
                    "description": "Unique farmer identifier (non-PII, derived from DID)",
                    "pattern": "^FRM-[A-Z0-9]{8,16}$"
                },
                "registration_date": {
                    "type": "string",
                    "format": "date-time",
                    "description": "ISO 8601 registration timestamp"
                },
                "profile_encrypted_cid": {
                    "type": "string",
                    "description": "IPFS CID of encrypted farmer profile (contains PII)",
                    "pattern": "^Qm[1-9A-HJ-NP-Za-km-z]{44}$|^bafy[a-z0-9]{50,}$"
                },
                "land_details": {
                    "type": "object",
                    "required": ["total_area_hectares", "soil_type", "geo_hash"],
                    "properties": {
                        "total_area_hectares": {
                            "type": "number",
                            "minimum": 0.01,
                            "maximum": 10000,
                            "description": "Total cultivable area in hectares"
                        },
                        "soil_type": {
                            "type": "string",
                            "enum": ["alluvial", "black", "red", "laterite", "desert", "mountain", "other"],
                            "description": "Soil classification"
                        },
                        "geo_hash": {
                            "type": "string",
                            "description": "Geohash of approximate location (not exact for privacy)",
                            "minLength": 4,
                            "maxLength": 8
                        },
                        "irrigation_type": {
                            "type": "string",
                            "enum": ["rainfed", "canal", "tubewell", "drip", "sprinkler", "mixed"],
                            "description": "Primary irrigation method"
                        },
                        "land_document_hashes": {
                            "type": "array",
                            "items": {
                                "type": "string",
                                "pattern": "^0x[a-fA-F0-9]{64}$"
                            },
                            "description": "SHA256 hashes of land ownership documents (documents stored encrypted)"
                        }
                    }
                },
                "crop_info": {
                    "type": "object",
                    "required": ["variety", "planting_season", "expected_yield_kg"],
                    "properties": {
                        "variety": {
                            "type": "string",
                            "enum": ["mustard", "groundnut", "soybean", "sunflower", "sesame", "safflower", "linseed", "castor", "niger", "other"],
                            "description": "Oilseed crop variety"
                        },
                        "planting_season": {
                            "type": "string",
                            "enum": ["kharif", "rabi", "zaid"],
                            "description": "Planting season"
                        },
                        "planting_date": {
                            "type": "string",
                            "format": "date",
                            "description": "Actual or planned planting date"
                        },
                        "expected_yield_kg": {
                            "type": "number",
                            "minimum": 0,
                            "description": "Expected yield in kilograms"
                        },
                        "seed_variety_code": {
                            "type": "string",
                            "description": "Certified seed variety code if applicable"
                        },
                        "organic_certified": {
                            "type": "boolean",
                            "default": false,
                            "description": "Whether organic certification is claimed"
                        }
                    }
                },
                "verification_status": {
                    "type": "object",
                    "required": ["status", "verified_at"],
                    "properties": {
                        "status": {
                            "type": "string",
                            "enum": ["pending", "verified", "rejected", "suspended"],
                            "description": "Current verification status"
                        },
                        "verified_at": {
                            "type": "string",
                            "format": "date-time",
                            "description": "Timestamp of last verification update"
                        },
                        "verifier_id": {
                            "type": "string",
                            "description": "ID of the verifying entity"
                        },
                        "verification_method": {
                            "type": "string",
                            "enum": ["physical_visit", "video_kyc", "document_review", "satellite_verification"],
                            "description": "Method used for verification"
                        },
                        "certifications": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "type": { "type": "string" },
                                    "issuer": { "type": "string" },
                                    "valid_until": { "type": "string", "format": "date" },
                                    "certificate_hash": { "type": "string" }
                                }
                            },
                            "description": "List of certifications (organic, etc.)"
                        }
                    }
                },
                "satellite_data": {
                    "type": "object",
                    "properties": {
                        "imagery_cid": {
                            "type": "string",
                            "description": "IPFS CID of satellite imagery analysis"
                        },
                        "ndvi_score": {
                            "type": "number",
                            "minimum": -1,
                            "maximum": 1,
                            "description": "Normalized Difference Vegetation Index"
                        },
                        "capture_date": {
                            "type": "string",
                            "format": "date"
                        }
                    },
                    "description": "Optional satellite verification data"
                }
            },
            "additionalProperties": true
        }),
        required_fields: vec![
            "farmer_id".to_string(),
            "registration_date".to_string(),
            "land_details".to_string(),
            "crop_info".to_string(),
            "verification_status".to_string(),
        ],
        optional_fields: vec![
            "profile_encrypted_cid".to_string(),
            "satellite_data".to_string(),
        ],
        pii_fields: vec![
            "profile.name".to_string(),
            "profile.phone".to_string(),
            "profile.address".to_string(),
            "profile.aadhaar_hash".to_string(),
            "profile.bank_details".to_string(),
        ],
        created_at: "2024-01-01T00:00:00Z".to_string(),
        previous_version_cid: None,
        changelog: Some("Initial schema version".to_string()),
    }
}

// ============================================================================
// FPO PURCHASE SCHEMA (v1.0.0)
// ============================================================================

fn fpo_purchase_schema() -> SchemaDefinition {
    SchemaDefinition {
        schema: SchemaReference::new("fpo_purchase", "1.0.0"),
        description: "Schema for FPO (Farmer Producer Organization) purchase transactions from farmers.".to_string(),
        json_schema: json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "$id": "https://oilseed.chain/schemas/fpo_purchase/1.0.0",
            "type": "object",
            "required": ["batch_id", "farmer_id", "fpo_id", "purchase_date", "quantity", "quality_assessment"],
            "properties": {
                "batch_id": {
                    "type": "string",
                    "description": "Unique batch identifier",
                    "pattern": "^BATCH-[A-Z0-9]{12,20}$"
                },
                "farmer_id": {
                    "type": "string",
                    "description": "Reference to farmer who sold the produce",
                    "pattern": "^FRM-[A-Z0-9]{8,16}$"
                },
                "farmer_registration_cid": {
                    "type": "string",
                    "description": "IPFS CID of farmer's registration document for traceability"
                },
                "fpo_id": {
                    "type": "string",
                    "description": "FPO identifier",
                    "pattern": "^FPO-[A-Z0-9]{8,16}$"
                },
                "purchase_date": {
                    "type": "string",
                    "format": "date-time",
                    "description": "Timestamp of purchase transaction"
                },
                "quantity": {
                    "type": "object",
                    "required": ["gross_weight_kg", "net_weight_kg"],
                    "properties": {
                        "gross_weight_kg": {
                            "type": "number",
                            "minimum": 0,
                            "description": "Gross weight before deductions"
                        },
                        "net_weight_kg": {
                            "type": "number",
                            "minimum": 0,
                            "description": "Net weight after moisture/foreign matter deductions"
                        },
                        "bags_count": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Number of bags"
                        },
                        "deductions": {
                            "type": "object",
                            "properties": {
                                "moisture_percent": { "type": "number" },
                                "foreign_matter_percent": { "type": "number" },
                                "damaged_percent": { "type": "number" }
                            }
                        }
                    }
                },
                "quality_assessment": {
                    "type": "object",
                    "required": ["grade", "moisture_content", "oil_content_estimated"],
                    "properties": {
                        "grade": {
                            "type": "string",
                            "enum": ["A+", "A", "B", "C", "rejected"],
                            "description": "Quality grade assigned"
                        },
                        "moisture_content": {
                            "type": "number",
                            "minimum": 0,
                            "maximum": 100,
                            "description": "Moisture content percentage"
                        },
                        "oil_content_estimated": {
                            "type": "number",
                            "minimum": 0,
                            "maximum": 100,
                            "description": "Estimated oil content percentage"
                        },
                        "foreign_matter_percent": {
                            "type": "number",
                            "minimum": 0,
                            "maximum": 100
                        },
                        "damaged_grains_percent": {
                            "type": "number",
                            "minimum": 0,
                            "maximum": 100
                        },
                        "color_assessment": {
                            "type": "string",
                            "enum": ["normal", "discolored", "mixed"]
                        },
                        "test_weight_kg_per_hl": {
                            "type": "number",
                            "description": "Test weight (kg per hectoliter)"
                        }
                    }
                },
                "pricing": {
                    "type": "object",
                    "required": ["price_per_kg", "total_amount", "currency"],
                    "properties": {
                        "price_per_kg": {
                            "type": "number",
                            "minimum": 0,
                            "description": "Price per kilogram"
                        },
                        "total_amount": {
                            "type": "number",
                            "minimum": 0,
                            "description": "Total transaction amount"
                        },
                        "currency": {
                            "type": "string",
                            "default": "INR",
                            "description": "Currency code"
                        },
                        "msp_reference": {
                            "type": "number",
                            "description": "Minimum Support Price for reference"
                        },
                        "premium_paid": {
                            "type": "number",
                            "description": "Premium over MSP if any"
                        }
                    }
                },
                "documents": {
                    "type": "object",
                    "properties": {
                        "receipt_hash": {
                            "type": "string",
                            "pattern": "^0x[a-fA-F0-9]{64}$",
                            "description": "SHA256 hash of purchase receipt"
                        },
                        "weighment_slip_hash": {
                            "type": "string",
                            "pattern": "^0x[a-fA-F0-9]{64}$"
                        },
                        "quality_report_cid": {
                            "type": "string",
                            "description": "IPFS CID of detailed quality report"
                        },
                        "photo_cids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "maxItems": 10,
                            "description": "IPFS CIDs of batch photos"
                        }
                    }
                },
                "collection_point": {
                    "type": "object",
                    "properties": {
                        "location_id": { "type": "string" },
                        "geo_hash": { "type": "string", "minLength": 4, "maxLength": 8 },
                        "type": {
                            "type": "string",
                            "enum": ["fpo_center", "mandi", "village_collection", "farm_gate"]
                        }
                    }
                }
            },
            "additionalProperties": true
        }),
        required_fields: vec![
            "batch_id".to_string(),
            "farmer_id".to_string(),
            "fpo_id".to_string(),
            "purchase_date".to_string(),
            "quantity".to_string(),
            "quality_assessment".to_string(),
        ],
        optional_fields: vec![
            "pricing".to_string(),
            "documents".to_string(),
            "collection_point".to_string(),
            "farmer_registration_cid".to_string(),
        ],
        pii_fields: vec![], // FPO purchase doesn't contain direct PII
        created_at: "2024-01-01T00:00:00Z".to_string(),
        previous_version_cid: None,
        changelog: Some("Initial schema version".to_string()),
    }
}

// ============================================================================
// WAREHOUSE STORAGE SCHEMA (v1.0.0)
// ============================================================================

fn warehouse_storage_schema() -> SchemaDefinition {
    SchemaDefinition {
        schema: SchemaReference::new("warehouse_storage", "1.0.0"),
        description: "Schema for warehouse storage records including IoT sensor data and storage conditions.".to_string(),
        json_schema: json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "$id": "https://oilseed.chain/schemas/warehouse_storage/1.0.0",
            "type": "object",
            "required": ["warehouse_id", "batch_id", "storage_unit", "entry_date", "storage_conditions"],
            "properties": {
                "warehouse_id": {
                    "type": "string",
                    "description": "Unique warehouse identifier",
                    "pattern": "^WH-[A-Z0-9]{8,16}$"
                },
                "batch_id": {
                    "type": "string",
                    "description": "Batch being stored",
                    "pattern": "^BATCH-[A-Z0-9]{12,20}$"
                },
                "source_batch_cid": {
                    "type": "string",
                    "description": "IPFS CID of the source batch (FPO purchase) for traceability"
                },
                "storage_unit": {
                    "type": "object",
                    "required": ["unit_id", "type"],
                    "properties": {
                        "unit_id": {
                            "type": "string",
                            "description": "Specific storage bin/silo/section ID"
                        },
                        "type": {
                            "type": "string",
                            "enum": ["silo", "bin", "bag_stack", "bulk_floor", "cold_storage"]
                        },
                        "capacity_mt": {
                            "type": "number",
                            "description": "Storage unit capacity in metric tons"
                        },
                        "current_occupancy_percent": {
                            "type": "number",
                            "minimum": 0,
                            "maximum": 100
                        }
                    }
                },
                "entry_date": {
                    "type": "string",
                    "format": "date-time",
                    "description": "When the batch entered storage"
                },
                "exit_date": {
                    "type": "string",
                    "format": "date-time",
                    "description": "When the batch exited storage (null if still stored)"
                },
                "storage_conditions": {
                    "type": "object",
                    "required": ["temperature_celsius", "humidity_percent"],
                    "properties": {
                        "temperature_celsius": {
                            "type": "number",
                            "description": "Current/average temperature"
                        },
                        "humidity_percent": {
                            "type": "number",
                            "minimum": 0,
                            "maximum": 100,
                            "description": "Relative humidity percentage"
                        },
                        "co2_ppm": {
                            "type": "number",
                            "description": "CO2 concentration for pest monitoring"
                        },
                        "ventilation_status": {
                            "type": "string",
                            "enum": ["active", "passive", "sealed"]
                        }
                    }
                },
                "iot_data": {
                    "type": "object",
                    "properties": {
                        "rollup_period": {
                            "type": "string",
                            "enum": ["1h", "6h", "12h", "24h"],
                            "description": "Aggregation period for this rollup"
                        },
                        "sensor_readings": {
                            "type": "object",
                            "properties": {
                                "temperature": {
                                    "type": "object",
                                    "properties": {
                                        "min": { "type": "number" },
                                        "max": { "type": "number" },
                                        "avg": { "type": "number" },
                                        "readings_count": { "type": "integer" }
                                    }
                                },
                                "humidity": {
                                    "type": "object",
                                    "properties": {
                                        "min": { "type": "number" },
                                        "max": { "type": "number" },
                                        "avg": { "type": "number" },
                                        "readings_count": { "type": "integer" }
                                    }
                                }
                            }
                        },
                        "alerts": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "type": { "type": "string" },
                                    "severity": { "type": "string", "enum": ["low", "medium", "high", "critical"] },
                                    "timestamp": { "type": "string", "format": "date-time" },
                                    "message": { "type": "string" }
                                }
                            }
                        },
                        "raw_data_cid": {
                            "type": "string",
                            "description": "IPFS CID of full raw IoT data (compressed)"
                        }
                    }
                },
                "treatments": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": {
                                "type": "string",
                                "enum": ["fumigation", "aeration", "pest_control", "moisture_control"]
                            },
                            "date": { "type": "string", "format": "date-time" },
                            "chemical_used": { "type": "string" },
                            "dosage": { "type": "string" },
                            "certificate_hash": { "type": "string" },
                            "operator_id": { "type": "string" }
                        }
                    },
                    "description": "List of treatments applied during storage"
                },
                "inspections": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "date": { "type": "string", "format": "date-time" },
                            "inspector_id": { "type": "string" },
                            "type": { "type": "string" },
                            "result": { "type": "string", "enum": ["pass", "conditional", "fail"] },
                            "notes": { "type": "string" },
                            "report_cid": { "type": "string" }
                        }
                    }
                },
                "warehouse_info": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "license_number": { "type": "string" },
                        "geo_hash": { "type": "string" },
                        "certifications": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    }
                }
            },
            "additionalProperties": true
        }),
        required_fields: vec![
            "warehouse_id".to_string(),
            "batch_id".to_string(),
            "storage_unit".to_string(),
            "entry_date".to_string(),
            "storage_conditions".to_string(),
        ],
        optional_fields: vec![
            "exit_date".to_string(),
            "iot_data".to_string(),
            "treatments".to_string(),
            "inspections".to_string(),
            "source_batch_cid".to_string(),
        ],
        pii_fields: vec![], // Warehouse data doesn't contain PII
        created_at: "2024-01-01T00:00:00Z".to_string(),
        previous_version_cid: None,
        changelog: Some("Initial schema version".to_string()),
    }
}

// ============================================================================
// LOGISTICS SCHEMA (v1.0.0)
// ============================================================================

fn logistics_schema() -> SchemaDefinition {
    SchemaDefinition {
        schema: SchemaReference::new("logistics", "1.0.0"),
        description: "Schema for logistics and transportation tracking including GPS traces and delivery milestones.".to_string(),
        json_schema: json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "$id": "https://oilseed.chain/schemas/logistics/1.0.0",
            "type": "object",
            "required": ["shipment_id", "batch_id", "origin", "destination", "departure_time", "status"],
            "properties": {
                "shipment_id": {
                    "type": "string",
                    "pattern": "^SHIP-[A-Z0-9]{12,20}$"
                },
                "batch_id": {
                    "type": "string",
                    "pattern": "^BATCH-[A-Z0-9]{12,20}$"
                },
                "source_storage_cid": {
                    "type": "string",
                    "description": "IPFS CID of source warehouse storage record"
                },
                "origin": {
                    "type": "object",
                    "required": ["location_id", "geo_hash"],
                    "properties": {
                        "location_id": { "type": "string" },
                        "location_type": { "type": "string", "enum": ["warehouse", "processing_unit", "fpo_center", "port"] },
                        "geo_hash": { "type": "string" }
                    }
                },
                "destination": {
                    "type": "object",
                    "required": ["location_id", "geo_hash"],
                    "properties": {
                        "location_id": { "type": "string" },
                        "location_type": { "type": "string", "enum": ["warehouse", "processing_unit", "distribution_center", "port", "retail"] },
                        "geo_hash": { "type": "string" }
                    }
                },
                "vehicle": {
                    "type": "object",
                    "properties": {
                        "vehicle_id": { "type": "string" },
                        "type": { "type": "string", "enum": ["truck", "container", "rail", "ship"] },
                        "registration_number": { "type": "string" },
                        "capacity_mt": { "type": "number" }
                    }
                },
                "departure_time": {
                    "type": "string",
                    "format": "date-time"
                },
                "estimated_arrival": {
                    "type": "string",
                    "format": "date-time"
                },
                "actual_arrival": {
                    "type": "string",
                    "format": "date-time"
                },
                "status": {
                    "type": "string",
                    "enum": ["scheduled", "in_transit", "delayed", "delivered", "cancelled"]
                },
                "milestones": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "timestamp": { "type": "string", "format": "date-time" },
                            "geo_hash": { "type": "string" },
                            "milestone_type": { "type": "string", "enum": ["departure", "checkpoint", "border_crossing", "rest_stop", "arrival"] },
                            "notes": { "type": "string" }
                        }
                    }
                },
                "tracking_data": {
                    "type": "object",
                    "properties": {
                        "gps_trace_cid": {
                            "type": "string",
                            "description": "IPFS CID of compressed GPS trace data"
                        },
                        "temperature_log_cid": {
                            "type": "string",
                            "description": "IPFS CID of temperature log during transit"
                        },
                        "current_geo_hash": { "type": "string" },
                        "last_update": { "type": "string", "format": "date-time" }
                    }
                },
                "documents": {
                    "type": "object",
                    "properties": {
                        "eway_bill_hash": { "type": "string" },
                        "lorry_receipt_hash": { "type": "string" },
                        "pod_hash": { "type": "string", "description": "Proof of Delivery hash" },
                        "customs_docs_cid": { "type": "string" }
                    }
                },
                "conditions_on_arrival": {
                    "type": "object",
                    "properties": {
                        "seal_intact": { "type": "boolean" },
                        "temperature_ok": { "type": "boolean" },
                        "visual_inspection": { "type": "string", "enum": ["good", "acceptable", "damaged"] },
                        "quantity_variance_kg": { "type": "number" },
                        "photos_cid": { "type": "string" }
                    }
                }
            },
            "additionalProperties": true
        }),
        required_fields: vec![
            "shipment_id".to_string(),
            "batch_id".to_string(),
            "origin".to_string(),
            "destination".to_string(),
            "departure_time".to_string(),
            "status".to_string(),
        ],
        optional_fields: vec![
            "vehicle".to_string(),
            "milestones".to_string(),
            "tracking_data".to_string(),
            "documents".to_string(),
            "conditions_on_arrival".to_string(),
        ],
        pii_fields: vec![
            "vehicle.driver_name".to_string(),
            "vehicle.driver_phone".to_string(),
            "vehicle.driver_license".to_string(),
        ],
        created_at: "2024-01-01T00:00:00Z".to_string(),
        previous_version_cid: None,
        changelog: Some("Initial schema version".to_string()),
    }
}

// ============================================================================
// PROCESSING SCHEMA (v1.0.0)
// ============================================================================

fn processing_schema() -> SchemaDefinition {
    SchemaDefinition {
        schema: SchemaReference::new("processing", "1.0.0"),
        description: "Schema for oil extraction and processing records including yield data and lab results.".to_string(),
        json_schema: json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "$id": "https://oilseed.chain/schemas/processing/1.0.0",
            "type": "object",
            "required": ["processing_id", "facility_id", "input_batches", "process_type", "processing_date", "outputs"],
            "properties": {
                "processing_id": {
                    "type": "string",
                    "pattern": "^PROC-[A-Z0-9]{12,20}$"
                },
                "facility_id": {
                    "type": "string",
                    "pattern": "^FAC-[A-Z0-9]{8,16}$"
                },
                "input_batches": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "batch_id": { "type": "string" },
                            "source_cid": { "type": "string", "description": "IPFS CID of input batch document" },
                            "quantity_kg": { "type": "number" }
                        }
                    },
                    "minItems": 1
                },
                "process_type": {
                    "type": "string",
                    "enum": ["cold_press", "expeller", "solvent_extraction", "refining", "blending"]
                },
                "processing_date": {
                    "type": "string",
                    "format": "date-time"
                },
                "process_parameters": {
                    "type": "object",
                    "properties": {
                        "temperature_celsius": { "type": "number" },
                        "pressure_bar": { "type": "number" },
                        "duration_minutes": { "type": "number" },
                        "equipment_id": { "type": "string" }
                    }
                },
                "outputs": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["output_batch_id", "product_type", "quantity"],
                        "properties": {
                            "output_batch_id": { "type": "string" },
                            "product_type": { "type": "string", "enum": ["crude_oil", "refined_oil", "oil_cake", "de_oiled_cake", "by_product"] },
                            "quantity": { "type": "number" },
                            "unit": { "type": "string", "enum": ["kg", "liters", "mt"] }
                        }
                    }
                },
                "yield_data": {
                    "type": "object",
                    "properties": {
                        "oil_yield_percent": { "type": "number", "minimum": 0, "maximum": 100 },
                        "cake_yield_percent": { "type": "number", "minimum": 0, "maximum": 100 },
                        "loss_percent": { "type": "number", "minimum": 0, "maximum": 100 }
                    }
                },
                "quality_results": {
                    "type": "object",
                    "properties": {
                        "ffa_percent": { "type": "number", "description": "Free Fatty Acid content" },
                        "moisture_percent": { "type": "number" },
                        "peroxide_value": { "type": "number" },
                        "iodine_value": { "type": "number" },
                        "color_lovibond": { "type": "string" },
                        "lab_report_cid": { "type": "string", "description": "IPFS CID of detailed lab report" }
                    }
                },
                "facility_info": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "license_number": { "type": "string" },
                        "fssai_number": { "type": "string" },
                        "certifications": { "type": "array", "items": { "type": "string" } }
                    }
                }
            },
            "additionalProperties": true
        }),
        required_fields: vec![
            "processing_id".to_string(),
            "facility_id".to_string(),
            "input_batches".to_string(),
            "process_type".to_string(),
            "processing_date".to_string(),
            "outputs".to_string(),
        ],
        optional_fields: vec![
            "process_parameters".to_string(),
            "yield_data".to_string(),
            "quality_results".to_string(),
            "facility_info".to_string(),
        ],
        pii_fields: vec![],
        created_at: "2024-01-01T00:00:00Z".to_string(),
        previous_version_cid: None,
        changelog: Some("Initial schema version".to_string()),
    }
}

// ============================================================================
// PACKAGING SCHEMA (v1.0.0)
// ============================================================================

fn packaging_schema() -> SchemaDefinition {
    SchemaDefinition {
        schema: SchemaReference::new("packaging", "1.0.0"),
        description: "Schema for packaging and SKU creation including Merkle tree proofs for unit-level traceability.".to_string(),
        json_schema: json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "$id": "https://oilseed.chain/schemas/packaging/1.0.0",
            "type": "object",
            "required": ["sku_id", "parent_batch_id", "packaging_date", "units", "merkle_root"],
            "properties": {
                "sku_id": {
                    "type": "string",
                    "pattern": "^SKU-[A-Z0-9]{12,20}$"
                },
                "parent_batch_id": {
                    "type": "string",
                    "description": "Processing output batch this SKU is derived from"
                },
                "source_processing_cid": {
                    "type": "string",
                    "description": "IPFS CID of source processing record"
                },
                "packaging_date": {
                    "type": "string",
                    "format": "date-time"
                },
                "facility_id": {
                    "type": "string"
                },
                "product_info": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "brand": { "type": "string" },
                        "variant": { "type": "string" },
                        "description": { "type": "string" }
                    }
                },
                "units": {
                    "type": "object",
                    "required": ["total_count", "unit_size", "unit_type"],
                    "properties": {
                        "total_count": { "type": "integer", "minimum": 1 },
                        "unit_size": { "type": "number" },
                        "unit_type": { "type": "string", "enum": ["ml", "liters", "grams", "kg"] },
                        "packaging_type": { "type": "string", "enum": ["pet_bottle", "glass_bottle", "pouch", "tin", "hdpe_container", "bulk"] },
                        "batch_codes": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Individual unit batch codes for traceability"
                        }
                    }
                },
                "merkle_root": {
                    "type": "string",
                    "pattern": "^0x[a-fA-F0-9]{64}$",
                    "description": "Merkle root of all unit serial numbers for verification"
                },
                "merkle_proofs_cid": {
                    "type": "string",
                    "description": "IPFS CID of Merkle proofs file for individual unit verification"
                },
                "labeling": {
                    "type": "object",
                    "properties": {
                        "mrp": { "type": "number" },
                        "currency": { "type": "string", "default": "INR" },
                        "best_before_months": { "type": "integer" },
                        "manufacturing_date": { "type": "string", "format": "date" },
                        "expiry_date": { "type": "string", "format": "date" },
                        "fssai_number": { "type": "string" },
                        "nutritional_info_cid": { "type": "string" },
                        "allergen_info": { "type": "array", "items": { "type": "string" } }
                    }
                },
                "quality_checks": {
                    "type": "object",
                    "properties": {
                        "visual_inspection": { "type": "string", "enum": ["pass", "fail"] },
                        "seal_integrity": { "type": "string", "enum": ["pass", "fail"] },
                        "weight_check": { "type": "string", "enum": ["pass", "fail"] },
                        "label_check": { "type": "string", "enum": ["pass", "fail"] },
                        "qc_report_cid": { "type": "string" }
                    }
                },
                "traceability_qr": {
                    "type": "object",
                    "properties": {
                        "url_template": { "type": "string", "format": "uri" },
                        "verification_endpoint": { "type": "string", "format": "uri" }
                    }
                }
            },
            "additionalProperties": true
        }),
        required_fields: vec![
            "sku_id".to_string(),
            "parent_batch_id".to_string(),
            "packaging_date".to_string(),
            "units".to_string(),
            "merkle_root".to_string(),
        ],
        optional_fields: vec![
            "source_processing_cid".to_string(),
            "product_info".to_string(),
            "labeling".to_string(),
            "quality_checks".to_string(),
            "merkle_proofs_cid".to_string(),
        ],
        pii_fields: vec![],
        created_at: "2024-01-01T00:00:00Z".to_string(),
        previous_version_cid: None,
        changelog: Some("Initial schema version".to_string()),
    }
}

// ============================================================================
// AI SCORING SCHEMA (v1.0.0)
// ============================================================================

fn ai_scoring_schema() -> SchemaDefinition {
    SchemaDefinition {
        schema: SchemaReference::new("ai_scoring", "1.0.0"),
        description: "Schema for AI-based quality and trust scoring with full reproducibility and explainability.".to_string(),
        json_schema: json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "$id": "https://oilseed.chain/schemas/ai_scoring/1.0.0",
            "type": "object",
            "required": ["scoring_id", "batch_id", "model_info", "inputs", "outputs", "scoring_timestamp"],
            "properties": {
                "scoring_id": {
                    "type": "string",
                    "pattern": "^SCORE-[A-Z0-9]{12,20}$"
                },
                "batch_id": {
                    "type": "string",
                    "description": "Batch being scored"
                },
                "scoring_timestamp": {
                    "type": "string",
                    "format": "date-time"
                },
                "model_info": {
                    "type": "object",
                    "required": ["model_id", "version", "weights_hash"],
                    "properties": {
                        "model_id": {
                            "type": "string",
                            "description": "Unique model identifier"
                        },
                        "version": {
                            "type": "string",
                            "description": "Model version (semantic versioning)"
                        },
                        "weights_hash": {
                            "type": "string",
                            "pattern": "^0x[a-fA-F0-9]{64}$",
                            "description": "SHA256 hash of model weights file"
                        },
                        "config_hash": {
                            "type": "string",
                            "pattern": "^0x[a-fA-F0-9]{64}$",
                            "description": "SHA256 hash of model config (architecture, hyperparameters)"
                        },
                        "training_data_hash": {
                            "type": "string",
                            "description": "Hash of training dataset for audit"
                        },
                        "model_card_cid": {
                            "type": "string",
                            "description": "IPFS CID of model card (documentation)"
                        }
                    }
                },
                "inputs": {
                    "type": "object",
                    "required": ["input_cids"],
                    "properties": {
                        "input_cids": {
                            "type": "object",
                            "description": "IPFS CIDs of all input documents used for scoring",
                            "properties": {
                                "farmer_registration": { "type": "string" },
                                "fpo_purchase": { "type": "string" },
                                "warehouse_storage": { "type": "string" },
                                "logistics": { "type": "string" },
                                "processing": { "type": "string" },
                                "lab_reports": { "type": "string" }
                            }
                        },
                        "input_snapshot_cid": {
                            "type": "string",
                            "description": "IPFS CID of combined/preprocessed input data snapshot"
                        },
                        "input_hash": {
                            "type": "string",
                            "pattern": "^0x[a-fA-F0-9]{64}$",
                            "description": "Hash of all inputs for integrity verification"
                        }
                    }
                },
                "outputs": {
                    "type": "object",
                    "required": ["quality_score", "trust_score"],
                    "properties": {
                        "quality_score": {
                            "type": "number",
                            "minimum": 0,
                            "maximum": 10,
                            "description": "Overall quality score (0-10)"
                        },
                        "trust_score": {
                            "type": "number",
                            "minimum": 0,
                            "maximum": 10,
                            "description": "Supply chain trust/integrity score (0-10)"
                        },
                        "confidence": {
                            "type": "number",
                            "minimum": 0,
                            "maximum": 1,
                            "description": "Model confidence in the scores"
                        },
                        "risk_level": {
                            "type": "string",
                            "enum": ["low", "medium", "high", "critical"]
                        },
                        "anomalies_detected": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "type": { "type": "string" },
                                    "severity": { "type": "string" },
                                    "description": { "type": "string" }
                                }
                            }
                        },
                        "recommendations": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    }
                },
                "explainability": {
                    "type": "object",
                    "properties": {
                        "factor_weights": {
                            "type": "object",
                            "description": "Contribution of each factor to final score",
                            "properties": {
                                "source_verification": { "type": "number" },
                                "storage_conditions": { "type": "number" },
                                "processing_quality": { "type": "number" },
                                "logistics_integrity": { "type": "number" },
                                "lab_results": { "type": "number" },
                                "documentation_completeness": { "type": "number" }
                            }
                        },
                        "key_drivers": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "factor": { "type": "string" },
                                    "impact": { "type": "string", "enum": ["positive", "negative", "neutral"] },
                                    "weight": { "type": "number" },
                                    "explanation": { "type": "string" }
                                }
                            }
                        },
                        "detailed_report_cid": {
                            "type": "string",
                            "description": "IPFS CID of full explainability report"
                        }
                    }
                },
                "commit_reveal": {
                    "type": "object",
                    "description": "On-chain commit-reveal data for integrity",
                    "properties": {
                        "commit_hash": { "type": "string", "pattern": "^0x[a-fA-F0-9]{64}$" },
                        "reveal_hash": { "type": "string", "pattern": "^0x[a-fA-F0-9]{64}$" },
                        "nonce": { "type": "string" },
                        "committed_at": { "type": "string", "format": "date-time" },
                        "revealed_at": { "type": "string", "format": "date-time" }
                    }
                },
                "audit_trail": {
                    "type": "object",
                    "properties": {
                        "oracle_id": { "type": "string", "description": "ID of AI oracle that ran scoring" },
                        "execution_time_ms": { "type": "integer" },
                        "gpu_used": { "type": "string" },
                        "reproducibility_seed": { "type": "integer" }
                    }
                }
            },
            "additionalProperties": true
        }),
        required_fields: vec![
            "scoring_id".to_string(),
            "batch_id".to_string(),
            "model_info".to_string(),
            "inputs".to_string(),
            "outputs".to_string(),
            "scoring_timestamp".to_string(),
        ],
        optional_fields: vec![
            "explainability".to_string(),
            "commit_reveal".to_string(),
            "audit_trail".to_string(),
        ],
        pii_fields: vec![],
        created_at: "2024-01-01T00:00:00Z".to_string(),
        previous_version_cid: None,
        changelog: Some("Initial schema version".to_string()),
    }
}

// ============================================================================
// FRAUD REPORT SCHEMA (v1.0.0)
// ============================================================================

fn fraud_report_schema() -> SchemaDefinition {
    SchemaDefinition {
        schema: SchemaReference::new("fraud_report", "1.0.0"),
        description: "Schema for fraud detection reports and evidence documentation.".to_string(),
        json_schema: json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "$id": "https://oilseed.chain/schemas/fraud_report/1.0.0",
            "type": "object",
            "required": ["report_id", "sku_id", "report_type", "reported_at", "evidence"],
            "properties": {
                "report_id": {
                    "type": "string",
                    "pattern": "^FRAUD-[A-Z0-9]{12,20}$"
                },
                "sku_id": {
                    "type": "string",
                    "description": "SKU being reported"
                },
                "batch_id": {
                    "type": "string",
                    "description": "Related batch if known"
                },
                "report_type": {
                    "type": "string",
                    "enum": ["counterfeit", "adulteration", "mislabeling", "quality_issue", "documentation_fraud", "supply_chain_breach", "other"]
                },
                "severity": {
                    "type": "string",
                    "enum": ["low", "medium", "high", "critical"]
                },
                "reported_at": {
                    "type": "string",
                    "format": "date-time"
                },
                "reporter": {
                    "type": "object",
                    "properties": {
                        "type": { "type": "string", "enum": ["consumer", "retailer", "regulator", "ai_system", "internal_audit"] },
                        "reporter_id_hash": { "type": "string", "description": "Hashed reporter ID for privacy" },
                        "contact_encrypted_cid": { "type": "string", "description": "Encrypted contact info if provided" }
                    }
                },
                "evidence": {
                    "type": "object",
                    "required": ["evidence_hash", "description"],
                    "properties": {
                        "evidence_hash": {
                            "type": "string",
                            "pattern": "^0x[a-fA-F0-9]{64}$",
                            "description": "Hash of all evidence for on-chain reference"
                        },
                        "description": {
                            "type": "string",
                            "minLength": 10,
                            "maxLength": 5000
                        },
                        "photos_cid": {
                            "type": "string",
                            "description": "IPFS CID of photo evidence"
                        },
                        "lab_report_cid": {
                            "type": "string",
                            "description": "IPFS CID of independent lab analysis"
                        },
                        "purchase_proof_hash": {
                            "type": "string",
                            "description": "Hash of purchase receipt/invoice"
                        },
                        "location": {
                            "type": "object",
                            "properties": {
                                "geo_hash": { "type": "string" },
                                "retailer_id": { "type": "string" },
                                "purchase_date": { "type": "string", "format": "date" }
                            }
                        }
                    }
                },
                "investigation": {
                    "type": "object",
                    "properties": {
                        "status": { "type": "string", "enum": ["reported", "under_review", "investigating", "confirmed", "dismissed", "resolved"] },
                        "assigned_to": { "type": "string" },
                        "started_at": { "type": "string", "format": "date-time" },
                        "findings_cid": { "type": "string" },
                        "resolution": {
                            "type": "object",
                            "properties": {
                                "outcome": { "type": "string" },
                                "action_taken": { "type": "string" },
                                "resolved_at": { "type": "string", "format": "date-time" },
                                "compensation_provided": { "type": "boolean" }
                            }
                        }
                    }
                },
                "affected_batches": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of potentially affected batch IDs"
                },
                "recall_initiated": {
                    "type": "boolean",
                    "default": false
                }
            },
            "additionalProperties": true
        }),
        required_fields: vec![
            "report_id".to_string(),
            "sku_id".to_string(),
            "report_type".to_string(),
            "reported_at".to_string(),
            "evidence".to_string(),
        ],
        optional_fields: vec![
            "batch_id".to_string(),
            "severity".to_string(),
            "reporter".to_string(),
            "investigation".to_string(),
            "affected_batches".to_string(),
        ],
        pii_fields: vec![
            "reporter.contact_info".to_string(),
        ],
        created_at: "2024-01-01T00:00:00Z".to_string(),
        previous_version_cid: None,
        changelog: Some("Initial schema version".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_schemas_valid() {
        let schemas = get_all_schemas();
        assert_eq!(schemas.len(), 8);

        for schema in schemas {
            assert!(!schema.schema.name.is_empty());
            assert!(!schema.schema.version.is_empty());
            assert!(!schema.required_fields.is_empty());
        }
    }

    #[test]
    fn test_farmer_schema_pii_fields() {
        let schema = farmer_registration_schema();
        assert!(!schema.pii_fields.is_empty());
        assert!(schema.pii_fields.contains(&"profile.name".to_string()));
        assert!(schema.pii_fields.contains(&"profile.phone".to_string()));
    }

    #[test]
    fn test_schema_json_valid() {
        let schema = fpo_purchase_schema();
        // Verify the JSON schema can be serialized/deserialized
        let json_str = serde_json::to_string(&schema.json_schema).unwrap();
        let _: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    }
}
