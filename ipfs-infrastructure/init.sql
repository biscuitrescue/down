-- PostgreSQL Initialization Script for IPFS Metadata Management
-- Oilseed Value Chain - Production Database Schema

-- Enable required extensions
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pgcrypto";
CREATE EXTENSION IF NOT EXISTS "pg_stat_statements";

-- Create database users and roles
DO $$
BEGIN
    IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'ipfs_readonly') THEN
        CREATE ROLE ipfs_readonly;
    END IF;

    IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'ipfs_readwrite') THEN
        CREATE ROLE ipfs_readwrite;
    END IF;
END
$$;

-- Grant basic permissions
GRANT CONNECT ON DATABASE ipfs_metadata TO ipfs_readonly;
GRANT CONNECT ON DATABASE ipfs_metadata TO ipfs_readwrite;

-- ======================== CORE TABLES ========================

-- IPFS Content Registry
CREATE TABLE IF NOT EXISTS ipfs_content (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    cid VARCHAR(255) UNIQUE NOT NULL,
    content_hash VARCHAR(64) NOT NULL,
    size_bytes BIGINT NOT NULL DEFAULT 0,
    content_type VARCHAR(100),
    stage VARCHAR(50) NOT NULL,
    metadata JSONB,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    pin_status VARCHAR(20) DEFAULT 'pending',
    pin_count INTEGER DEFAULT 0,
    last_accessed TIMESTAMP WITH TIME ZONE,
    access_count BIGINT DEFAULT 0
);

-- Pin Status Tracking
CREATE TABLE IF NOT EXISTS pin_status (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    content_id UUID REFERENCES ipfs_content(id) ON DELETE CASCADE,
    service_name VARCHAR(50) NOT NULL, -- 'local', 'infura', 'pinata', etc.
    pin_hash VARCHAR(255) NOT NULL,
    status VARCHAR(20) NOT NULL DEFAULT 'pending', -- 'pending', 'pinned', 'failed', 'unpinned'
    pinned_at TIMESTAMP WITH TIME ZONE,
    last_checked TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    error_message TEXT,
    retry_count INTEGER DEFAULT 0,
    service_metadata JSONB
);

-- Supply Chain Stages
CREATE TABLE IF NOT EXISTS supply_chain_stages (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    stage_name VARCHAR(50) UNIQUE NOT NULL,
    stage_order INTEGER NOT NULL,
    description TEXT,
    required_fields JSONB,
    validation_rules JSONB,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- ======================== SUPPLY CHAIN SPECIFIC TABLES ========================

-- Farmer Records
CREATE TABLE IF NOT EXISTS farmers (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    farmer_did VARCHAR(66) UNIQUE NOT NULL, -- bytes32 as hex
    name VARCHAR(255) NOT NULL,
    phone VARCHAR(20),
    address TEXT,
    land_area_hectares DECIMAL(10,2),
    soil_type VARCHAR(50),
    coordinates POINT,
    crop_id_hash VARCHAR(66) NOT NULL,
    registration_cid VARCHAR(255),
    verification_status VARCHAR(20) DEFAULT 'pending',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- FPO (Farmer Producer Organization) Records
CREATE TABLE IF NOT EXISTS fpos (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    name VARCHAR(255) NOT NULL,
    license_number VARCHAR(100) UNIQUE NOT NULL,
    address TEXT,
    contact_person VARCHAR(255),
    phone VARCHAR(20),
    email VARCHAR(255),
    registration_date DATE,
    status VARCHAR(20) DEFAULT 'active',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Batch Records
CREATE TABLE IF NOT EXISTS batches (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    batch_hash VARCHAR(66) UNIQUE NOT NULL, -- bytes32 as hex
    farmer_id UUID REFERENCES farmers(id),
    fpo_id UUID REFERENCES fpos(id),
    batch_id VARCHAR(100) NOT NULL,
    quantity_kg DECIMAL(10,2),
    quality_grade VARCHAR(10),
    moisture_content DECIMAL(5,2),
    price_per_kg DECIMAL(10,2),
    total_amount DECIMAL(15,2),
    purchase_date DATE,
    purchase_cid VARCHAR(255),
    current_location VARCHAR(255),
    status VARCHAR(20) DEFAULT 'purchased',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Warehouse Records
CREATE TABLE IF NOT EXISTS warehouses (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    warehouse_id VARCHAR(100) UNIQUE NOT NULL,
    name VARCHAR(255) NOT NULL,
    address TEXT,
    coordinates POINT,
    capacity_mt INTEGER,
    manager_name VARCHAR(255),
    phone VARCHAR(20),
    license_number VARCHAR(100),
    certifications JSONB,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Warehouse Storage Records
CREATE TABLE IF NOT EXISTS warehouse_storage (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    warehouse_id UUID REFERENCES warehouses(id),
    batch_id UUID REFERENCES batches(id),
    storage_unit VARCHAR(100),
    entry_date DATE,
    exit_date DATE,
    storage_conditions JSONB,
    iot_data_cid VARCHAR(255),
    fumigation_done BOOLEAN DEFAULT FALSE,
    fumigation_date DATE,
    fumigation_certificate_cid VARCHAR(255),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Logistics Records
CREATE TABLE IF NOT EXISTS logistics (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    shipment_id VARCHAR(100) UNIQUE NOT NULL,
    batch_id UUID REFERENCES batches(id),
    origin_warehouse_id UUID REFERENCES warehouses(id),
    destination TEXT,
    vehicle_number VARCHAR(20),
    driver_name VARCHAR(255),
    driver_license VARCHAR(50),
    planned_route_cid VARCHAR(255),
    actual_route_cid VARCHAR(255),
    departure_time TIMESTAMP WITH TIME ZONE,
    arrival_time TIMESTAMP WITH TIME ZONE,
    status VARCHAR(20) DEFAULT 'in_transit',
    current_location POINT,
    temperature_log_cid VARCHAR(255),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Processing Facilities
CREATE TABLE IF NOT EXISTS processing_facilities (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    facility_id VARCHAR(100) UNIQUE NOT NULL,
    name VARCHAR(255) NOT NULL,
    address TEXT,
    license_number VARCHAR(100),
    certifications JSONB,
    processing_types TEXT[],
    capacity_per_day_mt INTEGER,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Processing Records
CREATE TABLE IF NOT EXISTS processing_records (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    facility_id UUID REFERENCES processing_facilities(id),
    input_batch_id UUID REFERENCES batches(id),
    process_type VARCHAR(100),
    process_method VARCHAR(100),
    temperature_celsius DECIMAL(5,2),
    yield_percentage DECIMAL(5,2),
    processing_date DATE,
    lab_report_cid VARCHAR(255),
    process_metadata_cid VARCHAR(255),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Output Products from Processing
CREATE TABLE IF NOT EXISTS output_products (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    processing_record_id UUID REFERENCES processing_records(id),
    product_type VARCHAR(100) NOT NULL,
    output_batch_hash VARCHAR(66) UNIQUE NOT NULL,
    quantity_produced DECIMAL(10,2),
    unit VARCHAR(10),
    quality_parameters JSONB,
    lab_results_cid VARCHAR(255),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Packaging Records
CREATE TABLE IF NOT EXISTS packaging_records (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    sku_id VARCHAR(66) UNIQUE NOT NULL, -- bytes32 as hex
    parent_batch_id UUID REFERENCES output_products(id),
    packaging_facility VARCHAR(255),
    packaging_date DATE,
    unit_size VARCHAR(50),
    packaging_type VARCHAR(100),
    label_design_cid VARCHAR(255),
    units_produced INTEGER,
    merkle_root VARCHAR(66),
    merkle_proofs_cid VARCHAR(255),
    packaging_metadata_cid VARCHAR(255),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- AI Scoring Records
CREATE TABLE IF NOT EXISTS ai_scores (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    batch_hash VARCHAR(66) NOT NULL,
    model_version VARCHAR(50),
    model_hash VARCHAR(64),
    quality_score DECIMAL(4,2),
    trust_score DECIMAL(4,2),
    commit_hash VARCHAR(66),
    reveal_hash VARCHAR(66),
    committed_at TIMESTAMP WITH TIME ZONE,
    revealed_at TIMESTAMP WITH TIME ZONE,
    scoring_metadata_cid VARCHAR(255),
    explanation_cid VARCHAR(255),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Fraud Reports
CREATE TABLE IF NOT EXISTS fraud_reports (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    sku_id VARCHAR(66) NOT NULL,
    reporter_address VARCHAR(42), -- Ethereum address
    evidence_hash VARCHAR(66),
    evidence_cid VARCHAR(255),
    report_type VARCHAR(50),
    description TEXT,
    status VARCHAR(20) DEFAULT 'reported',
    investigation_notes TEXT,
    resolved_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- ======================== AUDIT AND LOGGING TABLES ========================

-- Blockchain Events Log
CREATE TABLE IF NOT EXISTS blockchain_events (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    event_name VARCHAR(100) NOT NULL,
    contract_address VARCHAR(42) NOT NULL,
    transaction_hash VARCHAR(66) NOT NULL,
    block_number BIGINT NOT NULL,
    block_timestamp TIMESTAMP WITH TIME ZONE,
    event_data JSONB NOT NULL,
    ipfs_cid VARCHAR(255),
    processed BOOLEAN DEFAULT FALSE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- System Audit Log
CREATE TABLE IF NOT EXISTS audit_log (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    table_name VARCHAR(50) NOT NULL,
    record_id UUID NOT NULL,
    operation VARCHAR(10) NOT NULL, -- INSERT, UPDATE, DELETE
    old_values JSONB,
    new_values JSONB,
    user_id VARCHAR(100),
    session_id VARCHAR(100),
    ip_address INET,
    user_agent TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- API Usage Statistics
CREATE TABLE IF NOT EXISTS api_usage (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    endpoint VARCHAR(255) NOT NULL,
    method VARCHAR(10) NOT NULL,
    user_id VARCHAR(100),
    ip_address INET,
    response_status INTEGER,
    response_time_ms INTEGER,
    request_size_bytes BIGINT,
    response_size_bytes BIGINT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- ======================== INDEXES ========================

-- IPFS Content Indexes
CREATE INDEX IF NOT EXISTS idx_ipfs_content_cid ON ipfs_content(cid);
CREATE INDEX IF NOT EXISTS idx_ipfs_content_stage ON ipfs_content(stage);
CREATE INDEX IF NOT EXISTS idx_ipfs_content_created_at ON ipfs_content(created_at);
CREATE INDEX IF NOT EXISTS idx_ipfs_content_pin_status ON ipfs_content(pin_status);

-- Pin Status Indexes
CREATE INDEX IF NOT EXISTS idx_pin_status_content_id ON pin_status(content_id);
CREATE INDEX IF NOT EXISTS idx_pin_status_service ON pin_status(service_name);
CREATE INDEX IF NOT EXISTS idx_pin_status_status ON pin_status(status);
CREATE INDEX IF NOT EXISTS idx_pin_status_last_checked ON pin_status(last_checked);

-- Supply Chain Indexes
CREATE INDEX IF NOT EXISTS idx_farmers_did ON farmers(farmer_did);
CREATE INDEX IF NOT EXISTS idx_farmers_phone ON farmers(phone);
CREATE INDEX IF NOT EXISTS idx_batches_hash ON batches(batch_hash);
CREATE INDEX IF NOT EXISTS idx_batches_farmer_id ON batches(farmer_id);
CREATE INDEX IF NOT EXISTS idx_batches_status ON batches(status);
CREATE INDEX IF NOT EXISTS idx_warehouse_storage_batch_id ON warehouse_storage(batch_id);
CREATE INDEX IF NOT EXISTS idx_logistics_shipment_id ON logistics(shipment_id);
CREATE INDEX IF NOT EXISTS idx_packaging_sku_id ON packaging_records(sku_id);

-- Audit Indexes
CREATE INDEX IF NOT EXISTS idx_blockchain_events_contract ON blockchain_events(contract_address);
CREATE INDEX IF NOT EXISTS idx_blockchain_events_block ON blockchain_events(block_number);
CREATE INDEX IF NOT EXISTS idx_blockchain_events_processed ON blockchain_events(processed);
CREATE INDEX IF NOT EXISTS idx_audit_log_table_record ON audit_log(table_name, record_id);
CREATE INDEX IF NOT EXISTS idx_api_usage_endpoint ON api_usage(endpoint);
CREATE INDEX IF NOT EXISTS idx_api_usage_created_at ON api_usage(created_at);

-- GIN indexes for JSONB columns
CREATE INDEX IF NOT EXISTS idx_ipfs_content_metadata_gin ON ipfs_content USING GIN(metadata);
CREATE INDEX IF NOT EXISTS idx_blockchain_events_data_gin ON blockchain_events USING GIN(event_data);

-- Spatial indexes for geographical data
CREATE INDEX IF NOT EXISTS idx_farmers_coordinates ON farmers USING GIST(coordinates);
CREATE INDEX IF NOT EXISTS idx_warehouses_coordinates ON warehouses USING GIST(coordinates);
CREATE INDEX IF NOT EXISTS idx_logistics_current_location ON logistics USING GIST(current_location);

-- ======================== FUNCTIONS AND TRIGGERS ========================

-- Update timestamp function
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

-- Apply update triggers
CREATE TRIGGER update_ipfs_content_updated_at BEFORE UPDATE ON ipfs_content FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
CREATE TRIGGER update_farmers_updated_at BEFORE UPDATE ON farmers FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
CREATE TRIGGER update_batches_updated_at BEFORE UPDATE ON batches FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
CREATE TRIGGER update_logistics_updated_at BEFORE UPDATE ON logistics FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Audit trigger function
CREATE OR REPLACE FUNCTION audit_trigger_function()
RETURNS TRIGGER AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        INSERT INTO audit_log (table_name, record_id, operation, old_values, created_at)
        VALUES (TG_TABLE_NAME, OLD.id, TG_OP, row_to_json(OLD), NOW());
        RETURN OLD;
    ELSIF TG_OP = 'UPDATE' THEN
        INSERT INTO audit_log (table_name, record_id, operation, old_values, new_values, created_at)
        VALUES (TG_TABLE_NAME, NEW.id, TG_OP, row_to_json(OLD), row_to_json(NEW), NOW());
        RETURN NEW;
    ELSIF TG_OP = 'INSERT' THEN
        INSERT INTO audit_log (table_name, record_id, operation, new_values, created_at)
        VALUES (TG_TABLE_NAME, NEW.id, TG_OP, row_to_json(NEW), NOW());
        RETURN NEW;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

-- Apply audit triggers to important tables
CREATE TRIGGER audit_farmers AFTER INSERT OR UPDATE OR DELETE ON farmers FOR EACH ROW EXECUTE FUNCTION audit_trigger_function();
CREATE TRIGGER audit_batches AFTER INSERT OR UPDATE OR DELETE ON batches FOR EACH ROW EXECUTE FUNCTION audit_trigger_function();
CREATE TRIGGER audit_packaging AFTER INSERT OR UPDATE OR DELETE ON packaging_records FOR EACH ROW EXECUTE FUNCTION audit_trigger_function();
CREATE TRIGGER audit_fraud_reports AFTER INSERT OR UPDATE OR DELETE ON fraud_reports FOR EACH ROW EXECUTE FUNCTION audit_trigger_function();

-- ======================== VIEWS ========================

-- Supply Chain Overview View
CREATE OR REPLACE VIEW supply_chain_overview AS
SELECT
    b.batch_hash,
    b.batch_id,
    f.name AS farmer_name,
    f.farmer_did,
    fpo.name AS fpo_name,
    b.quantity_kg,
    b.quality_grade,
    b.status AS batch_status,
    w.name AS current_warehouse,
    l.status AS logistics_status,
    l.current_location,
    pr.process_type,
    pkg.sku_id,
    ai.quality_score,
    ai.trust_score
FROM batches b
LEFT JOIN farmers f ON b.farmer_id = f.id
LEFT JOIN fpos fpo ON b.fpo_id = fpo.id
LEFT JOIN warehouse_storage ws ON b.id = ws.batch_id
LEFT JOIN warehouses w ON ws.warehouse_id = w.id
LEFT JOIN logistics l ON b.id = l.batch_id
LEFT JOIN processing_records pr ON b.id = pr.input_batch_id
LEFT JOIN output_products op ON pr.id = op.processing_record_id
LEFT JOIN packaging_records pkg ON op.id = pkg.parent_batch_id
LEFT JOIN ai_scores ai ON b.batch_hash = ai.batch_hash;

-- IPFS Content Summary View
CREATE OR REPLACE VIEW ipfs_content_summary AS
SELECT
    stage,
    COUNT(*) AS total_content,
    SUM(size_bytes) AS total_size_bytes,
    AVG(size_bytes) AS avg_size_bytes,
    COUNT(CASE WHEN pin_status = 'pinned' THEN 1 END) AS pinned_count,
    COUNT(CASE WHEN pin_status = 'pending' THEN 1 END) AS pending_count,
    COUNT(CASE WHEN pin_status = 'failed' THEN 1 END) AS failed_count
FROM ipfs_content
GROUP BY stage;

-- ======================== INITIAL DATA ========================

-- Insert supply chain stages
INSERT INTO supply_chain_stages (stage_name, stage_order, description) VALUES
('farmer_registration', 1, 'Initial farmer registration and verification'),
('fpo_purchase', 2, 'FPO purchase of produce from farmers'),
('warehouse_storage', 3, 'Storage of produce in warehouses'),
('logistics', 4, 'Transportation and logistics tracking'),
('processing', 5, 'Processing of raw materials'),
('packaging', 6, 'Packaging into final SKUs'),
('ai_scoring', 7, 'AI-based quality and trust scoring'),
('fraud_detection', 8, 'Fraud detection and reporting')
ON CONFLICT (stage_name) DO NOTHING;

-- ======================== PERMISSIONS ========================

-- Grant permissions to readonly role
GRANT SELECT ON ALL TABLES IN SCHEMA public TO ipfs_readonly;
GRANT SELECT ON ALL SEQUENCES IN SCHEMA public TO ipfs_readonly;

-- Grant permissions to readwrite role
GRANT SELECT, INSERT, UPDATE ON ALL TABLES IN SCHEMA public TO ipfs_readwrite;
GRANT SELECT, USAGE ON ALL SEQUENCES IN SCHEMA public TO ipfs_readwrite;

-- Grant future permissions
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO ipfs_readonly;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE ON TABLES TO ipfs_readwrite;

-- ======================== DATABASE MAINTENANCE ========================

-- Enable automatic statistics collection
ALTER SYSTEM SET track_activities = on;
ALTER SYSTEM SET track_counts = on;
ALTER SYSTEM SET track_io_timing = on;

-- Configure autovacuum for better performance
ALTER SYSTEM SET autovacuum = on;
ALTER SYSTEM SET autovacuum_max_workers = 3;
ALTER SYSTEM SET autovacuum_naptime = '1min';

COMMIT;
