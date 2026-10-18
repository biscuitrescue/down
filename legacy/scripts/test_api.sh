#!/bin/sh

# Comprehensive API Test Script for OilseedValueChain
# Tests all endpoints with realistic data

set -e

# Colors
RED=''
GREEN=''
YELLOW=''
BLUE=''
NC='' # No Color

API_URL="http://localhost:3000"
FAILED=0
PASSED=0

# Helper function to print test results
print_test() {
    if [ $1 -eq 0 ]; then
        echo "${GREEN}✓ $2${NC}"
        PASSED=$((PASSED + 1))
    else
        echo "${RED}✗ $2${NC}"
        FAILED=$((FAILED + 1))
    fi
}

# Helper function to make API calls
api_call() {
    curl -s -X POST "$1" \
        -H "Content-Type: application/json" \
        -d "$2"
}

echo "${BLUE}======================================${NC}"
echo "${BLUE}OilseedValueChain API Test Suite${NC}"
echo "${BLUE}======================================${NC}"
echo ""

# Test 1: Health Check
echo "${YELLOW}Test 1: Health Check${NC}"
RESPONSE=$(curl -s "${API_URL}/health")
if [ "$RESPONSE" = "OK" ]; then
    print_test 0 "Health check passed"
else
    print_test 1 "Health check failed"
fi
echo ""

# Test 2: Root endpoint
echo "${YELLOW}Test 2: Root Endpoint${NC}"
RESPONSE=$(curl -s "${API_URL}/")
if echo "$RESPONSE" | grep -q "operational"; then
    print_test 0 "Root endpoint responding"
else
    print_test 1 "Root endpoint failed"
fi
echo ""

# Test 3: IPFS Upload
echo "${YELLOW}Test 3: IPFS Upload${NC}"
IPFS_DATA='{
    "data": {
        "test": "ipfs upload",
        "timestamp": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'"
    }
}'
IPFS_RESPONSE=$(api_call "${API_URL}/api/ipfs/upload" "$IPFS_DATA")
if echo "$IPFS_RESPONSE" | grep -q "cid"; then
    print_test 0 "IPFS upload successful"
    IPFS_CID=$(echo "$IPFS_RESPONSE" | grep -o '"cid":"[^"]*"' | cut -d'"' -f4)
    echo "   CID: ${GREEN}${IPFS_CID}${NC}"
else
    print_test 1 "IPFS upload failed"
fi
echo ""

# Test 4: Farmer Registration
echo "${YELLOW}Test 4: Farmer Registration${NC}"
FARMER_DID="0x$(printf '%064x' 12345)"
FARMER_DATA='{
    "farmer_did": "'${FARMER_DID}'",
    "crop_id": "WHEAT-2024-001",
    "metadata": {
        "name": "Rajesh Kumar",
        "location": "Punjab, India",
        "land_area": "5 acres",
        "crops": ["wheat", "rice"],
        "contact": "+91-9876543210",
        "aadhar": "1234-5678-9012",
        "registration_date": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'"
    }
}'
FARMER_RESPONSE=$(api_call "${API_URL}/api/farmer/register" "$FARMER_DATA")
if echo "$FARMER_RESPONSE" | grep -q "tx_hash"; then
    print_test 0 "Farmer registration successful"
    FARMER_TX=$(echo "$FARMER_RESPONSE" | grep -o '"tx_hash":"[^"]*"' | cut -d'"' -f4)
    FARMER_CID=$(echo "$FARMER_RESPONSE" | grep -o '"metadata_cid":"[^"]*"' | cut -d'"' -f4)
    echo "   TX: ${GREEN}${FARMER_TX}${NC}"
    echo "   Metadata CID: ${GREEN}${FARMER_CID}${NC}"
else
    print_test 1 "Farmer registration failed"
    echo "$FARMER_RESPONSE"
fi
echo ""

# Test 5: Verify Farmer
echo "${YELLOW}Test 5: Verify Farmer${NC}"
VERIFY_FARMER_DATA='{
    "farmer_did": "'${FARMER_DID}'"
}'
VERIFY_RESPONSE=$(api_call "${API_URL}/api/farmer/verify" "$VERIFY_FARMER_DATA")
if echo "$VERIFY_RESPONSE" | grep -q "exists"; then
    print_test 0 "Farmer verification successful"
    echo "   Response: ${VERIFY_RESPONSE}"
else
    print_test 1 "Farmer verification failed"
fi
echo ""

# Test 6: FPO Purchase
echo "${YELLOW}Test 6: FPO Purchase${NC}"
BATCH_ID="BATCH-$(date +%s)"
FPO_DATA='{
    "batch_id": "'${BATCH_ID}'",
    "farmer_did": "'${FARMER_DID}'",
    "metadata": {
        "quantity_kg": 5000,
        "quality_grade": "A+",
        "moisture_content": "12%",
        "purchase_price": 250000,
        "purchase_date": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'",
        "inspector": "Amit Singh",
        "quality_report": "Premium quality wheat"
    }
}'
FPO_RESPONSE=$(api_call "${API_URL}/api/fpo/purchase" "$FPO_DATA")
if echo "$FPO_RESPONSE" | grep -q "tx_hash"; then
    print_test 0 "FPO purchase recorded"
    FPO_TX=$(echo "$FPO_RESPONSE" | grep -o '"tx_hash":"[^"]*"' | cut -d'"' -f4)
    FPO_CID=$(echo "$FPO_RESPONSE" | grep -o '"metadata_cid":"[^"]*"' | cut -d'"' -f4)
    echo "   TX: ${GREEN}${FPO_TX}${NC}"
    echo "   Batch: ${GREEN}${BATCH_ID}${NC}"
else
    print_test 1 "FPO purchase failed"
    echo "$FPO_RESPONSE"
fi
echo ""

# Test 7: Warehouse Update
echo "${YELLOW}Test 7: Warehouse Update${NC}"
WAREHOUSE_ID="WH-PUNJAB-001"
WAREHOUSE_DATA='{
    "warehouse_id": "'${WAREHOUSE_ID}'",
    "iot_data": {
        "temperature": 22.5,
        "humidity": 45.2,
        "co2_level": 400,
        "last_fumigation": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'",
        "pest_control": "active",
        "batch_stored": "'${BATCH_ID}'"
    }
}'
WAREHOUSE_RESPONSE=$(api_call "${API_URL}/api/warehouse/update" "$WAREHOUSE_DATA")
if echo "$WAREHOUSE_RESPONSE" | grep -q "tx_hash"; then
    print_test 0 "Warehouse state updated"
    WH_TX=$(echo "$WAREHOUSE_RESPONSE" | grep -o '"tx_hash":"[^"]*"' | cut -d'"' -f4)
    echo "   TX: ${GREEN}${WH_TX}${NC}"
else
    print_test 1 "Warehouse update failed"
    echo "$WAREHOUSE_RESPONSE"
fi
echo ""

# Test 8: Logistics Record
echo "${YELLOW}Test 8: Logistics Tracking${NC}"
SHIPMENT_ID="SHIP-$(date +%s)"
LOGISTICS_DATA='{
    "shipment_id": "'${SHIPMENT_ID}'",
    "location": "Delhi Distribution Center",
    "is_delivered": false,
    "gps_data": {
        "latitude": 28.7041,
        "longitude": 77.1025,
        "timestamp": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'",
        "vehicle_id": "DL-01-AB-1234",
        "driver": "Suresh Yadav"
    }
}'
LOGISTICS_RESPONSE=$(api_call "${API_URL}/api/logistics/record" "$LOGISTICS_DATA")
if echo "$LOGISTICS_RESPONSE" | grep -q "tx_hash"; then
    print_test 0 "Logistics milestone recorded"
    LOG_TX=$(echo "$LOGISTICS_RESPONSE" | grep -o '"tx_hash":"[^"]*"' | cut -d'"' -f4)
    echo "   TX: ${GREEN}${LOG_TX}${NC}"
else
    print_test 1 "Logistics record failed"
    echo "$LOGISTICS_RESPONSE"
fi
echo ""

# Test 9: Batch Processing
echo "${YELLOW}Test 9: Batch Processing${NC}"
OUTPUT_BATCH="FLOUR-$(date +%s)"
PROCESS_DATA='{
    "input_batch_id": "'${BATCH_ID}'",
    "output_batch_ids": ["'${OUTPUT_BATCH}'"],
    "process_metadata": {
        "process_type": "milling",
        "yield_percentage": 92.5,
        "mill_id": "MILL-PUN-001",
        "processed_at": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'",
        "quality_check": "passed",
        "output_type": "whole_wheat_flour"
    }
}'
PROCESS_RESPONSE=$(api_call "${API_URL}/api/processing/batch" "$PROCESS_DATA")
if echo "$PROCESS_RESPONSE" | grep -q "tx_hash"; then
    print_test 0 "Batch processing recorded"
    PROC_TX=$(echo "$PROCESS_RESPONSE" | grep -o '"tx_hash":"[^"]*"' | cut -d'"' -f4)
    echo "   TX: ${GREEN}${PROC_TX}${NC}"
    echo "   Output: ${GREEN}${OUTPUT_BATCH}${NC}"
else
    print_test 1 "Batch processing failed"
    echo "$PROCESS_RESPONSE"
fi
echo ""

# Test 10: SKU Creation
echo "${YELLOW}Test 10: SKU Packaging${NC}"
SKU_ID="SKU-FLOUR-$(date +%s)"
UNIT_IDS='["'${SKU_ID}'-U001", "'${SKU_ID}'-U002", "'${SKU_ID}'-U003"]'
SKU_DATA='{
    "sku_id": "'${SKU_ID}'",
    "parent_batch_id": "'${OUTPUT_BATCH}'",
    "unit_ids": '${UNIT_IDS}',
    "packaging_metadata": {
        "package_type": "5kg bags",
        "units_count": 3,
        "barcode": "8901234567890",
        "expiry_months": 12,
        "packaged_at": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'",
        "packer": "ABC Packaging Pvt Ltd"
    }
}'
SKU_RESPONSE=$(api_call "${API_URL}/api/packaging/sku" "$SKU_DATA")
if echo "$SKU_RESPONSE" | grep -q "tx_hash"; then
    print_test 0 "SKU created successfully"
    SKU_TX=$(echo "$SKU_RESPONSE" | grep -o '"tx_hash":"[^"]*"' | cut -d'"' -f4)
    echo "   TX: ${GREEN}${SKU_TX}${NC}"
    echo "   SKU: ${GREEN}${SKU_ID}${NC}"
else
    print_test 1 "SKU creation failed"
    echo "$SKU_RESPONSE"
fi
echo ""

# Test 11: Verify SKU
echo "${YELLOW}Test 11: Verify SKU Origin${NC}"
VERIFY_SKU_DATA='{
    "sku_id": "'${SKU_ID}'"
}'
VERIFY_SKU_RESPONSE=$(api_call "${API_URL}/api/packaging/verify" "$VERIFY_SKU_DATA")
if echo "$VERIFY_SKU_RESPONSE" | grep -q "parent_batch_hash"; then
    print_test 0 "SKU verification successful"
    echo "   Response: ${VERIFY_SKU_RESPONSE}"
else
    print_test 1 "SKU verification failed"
fi
echo ""

# Test 12: AI Score Commit
echo "${YELLOW}Test 12: AI Score Commit${NC}"
# Generate a mock reveal hash and nonce
REVEAL_HASH="0x$(printf '%064x' $(($(date +%s) + 1000)))"
NONCE="0x$(printf '%064x' $(date +%N 2>/dev/null || echo 123456789))"
COMMIT_DATA='{
    "batch_id": "'${BATCH_ID}'",
    "reveal_hash": "'${REVEAL_HASH}'",
    "nonce": "'${NONCE}'"
}'
COMMIT_RESPONSE=$(api_call "${API_URL}/api/ai/commit" "$COMMIT_DATA")
if echo "$COMMIT_RESPONSE" | grep -q "tx_hash"; then
    print_test 0 "AI score committed"
    COMMIT_TX=$(echo "$COMMIT_RESPONSE" | grep -o '"tx_hash":"[^"]*"' | cut -d'"' -f4)
    echo "   TX: ${GREEN}${COMMIT_TX}${NC}"
else
    print_test 1 "AI score commit failed"
    echo "$COMMIT_RESPONSE"
fi
echo ""

# Test 13: AI Score Reveal (after delay)
echo "${YELLOW}Test 13: AI Score Reveal (waiting 3s for timelock)${NC}"
sleep 3
REVEAL_DATA='{
    "batch_id": "'${BATCH_ID}'",
    "nonce": "'${NONCE}'",
    "score_data": {
        "quality_score": 95.5,
        "freshness_score": 98.2,
        "purity_score": 96.8,
        "model_version": "v2.1.0",
        "evaluation_date": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'",
        "ml_confidence": 0.97
    }
}'
REVEAL_RESPONSE=$(api_call "${API_URL}/api/ai/reveal" "$REVEAL_DATA")
if echo "$REVEAL_RESPONSE" | grep -q "tx_hash"; then
    print_test 0 "AI score revealed"
    REVEAL_TX=$(echo "$REVEAL_RESPONSE" | grep -o '"tx_hash":"[^"]*"' | cut -d'"' -f4)
    echo "   TX: ${GREEN}${REVEAL_TX}${NC}"
else
    print_test 1 "AI score reveal failed"
    echo "$REVEAL_RESPONSE"
fi
echo ""

# Test 14: Fraud Report
echo "${YELLOW}Test 14: Fraud Report${NC}"
FRAUD_DATA='{
    "sku_id": "'${SKU_ID}'",
    "evidence": {
        "report_type": "quality_issue",
        "description": "Test fraud report - package opened",
        "reporter": "Quality Inspector",
        "reported_at": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'",
        "severity": "low",
        "images": ["evidence-001.jpg", "evidence-002.jpg"]
    }
}'
FRAUD_RESPONSE=$(api_call "${API_URL}/api/fraud/report" "$FRAUD_DATA")
if echo "$FRAUD_RESPONSE" | grep -q "tx_hash"; then
    print_test 0 "Fraud report filed"
    FRAUD_TX=$(echo "$FRAUD_RESPONSE" | grep -o '"tx_hash":"[^"]*"' | cut -d'"' -f4)
    echo "   TX: ${GREEN}${FRAUD_TX}${NC}"
else
    print_test 1 "Fraud report failed"
    echo "$FRAUD_RESPONSE"
fi
echo ""

# Summary
echo "${BLUE}======================================${NC}"
echo "${BLUE}Test Summary${NC}"
echo "${BLUE}======================================${NC}"
echo "${GREEN}Passed: ${PASSED}${NC}"
if [ $FAILED -gt 0 ]; then
    echo "${RED}Failed: ${FAILED}${NC}"
    echo ""
    echo "${RED}Some tests failed. Please check the logs.${NC}"
    exit 1
else
    echo "${RED}Failed: ${FAILED}${NC}"
    echo ""
    echo "${GREEN}All tests passed! 🎉${NC}"
    echo ""
    echo "Blockchain Explorer (Anvil):"
    echo "  Block Explorer: Check anvil.log"
    echo ""
    echo "IPFS Storage:"
    echo "  curl http://localhost:5001/debug/list"
    echo ""
    echo "Transaction IDs:"
    echo "  Farmer: ${FARMER_TX}"
    echo "  FPO: ${FPO_TX}"
    echo "  Warehouse: ${WH_TX}"
    echo "  Logistics: ${LOG_TX}"
    echo "  Processing: ${PROC_TX}"
    echo "  SKU: ${SKU_TX}"
    echo "  AI Commit: ${COMMIT_TX}"
    echo "  AI Reveal: ${REVEAL_TX}"
    echo "  Fraud: ${FRAUD_TX}"
fi
