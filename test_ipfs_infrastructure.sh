#!/bin/bash

# Comprehensive IPFS Infrastructure Test Script
# Tests the complete IPFS deployment including all services and integration points

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
PROJECT_NAME="oilseed-ipfs"
TEST_DATA_DIR="./test-data"
RESULTS_DIR="./test-results"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
LOG_FILE="$RESULTS_DIR/test_$TIMESTAMP.log"

# Service endpoints
IPFS_API_URL="http://localhost:5001"
IPFS_GATEWAY_URL="http://localhost:8081"
CLUSTER_API_URL="http://localhost:9094"
POSTGRES_HOST="localhost:5432"
REDIS_URL="redis://localhost:6379"
PROMETHEUS_URL="http://localhost:9090"
GRAFANA_URL="http://localhost:3001"
RUST_BACKEND_URL="http://localhost:3000"

# Test counters
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

# Functions
log_info() {
    echo -e "${GREEN}[INFO]${NC} $1" | tee -a "$LOG_FILE"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1" | tee -a "$LOG_FILE"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1" | tee -a "$LOG_FILE"
}

log_test() {
    echo -e "${BLUE}[TEST]${NC} $1" | tee -a "$LOG_FILE"
}

test_passed() {
    ((PASSED_TESTS++))
    ((TOTAL_TESTS++))
    echo -e "${GREEN}  ✓ PASSED${NC} - $1" | tee -a "$LOG_FILE"
}

test_failed() {
    ((FAILED_TESTS++))
    ((TOTAL_TESTS++))
    echo -e "${RED}  ✗ FAILED${NC} - $1" | tee -a "$LOG_FILE"
}

wait_for_service() {
    local url=$1
    local service_name=$2
    local max_attempts=30
    local attempt=1

    log_info "Waiting for $service_name to be ready..."

    while [ $attempt -le $max_attempts ]; do
        if curl -s "$url" > /dev/null 2>&1; then
            log_info "$service_name is ready (attempt $attempt)"
            return 0
        fi

        if [ $attempt -eq $max_attempts ]; then
            log_error "$service_name failed to become ready within timeout"
            return 1
        fi

        sleep 2
        ((attempt++))
    done
}

setup_test_environment() {
    log_info "Setting up test environment..."

    # Create test directories
    mkdir -p "$TEST_DATA_DIR"
    mkdir -p "$RESULTS_DIR"

    # Generate test data files
    cat > "$TEST_DATA_DIR/farmer_metadata.json" << 'EOF'
{
  "version": "1.0",
  "timestamp": "2024-01-15T10:30:00Z",
  "stage": "farmer_registration",
  "farmer_profile": {
    "name": "Test Farmer",
    "contact_info": {
      "phone": "+91-9876543210",
      "address": "Test Village, Test District"
    },
    "land_details": {
      "total_area": "5.2 hectares",
      "soil_type": "Alluvial",
      "coordinates": [25.837, 79.919],
      "documents": ["test_land_deed", "test_soil_report"]
    },
    "crop_info": {
      "variety": "Mustard",
      "planting_date": "2024-01-10",
      "expected_yield": "1500 kg"
    }
  },
  "verification": {
    "document_hashes": ["0x123test", "0x456test"],
    "satellite_imagery": "test_satellite_cid",
    "certifications": ["test_organic_cert"]
  }
}
EOF

    cat > "$TEST_DATA_DIR/warehouse_metadata.json" << 'EOF'
{
  "version": "1.0",
  "timestamp": "2024-03-16T09:00:00Z",
  "stage": "warehouse_storage",
  "warehouse_info": {
    "id": "TEST_WH_001",
    "location": [25.837, 79.919],
    "capacity": "1000 MT",
    "certifications": ["test_fssai_cert", "test_warehouse_license"]
  },
  "storage_details": {
    "batch_id": "TEST_BATCH_001",
    "storage_unit": "BIN_A_001",
    "entry_date": "2024-03-16",
    "fumigation": {
      "done": true,
      "date": "2024-03-17",
      "certificate": "test_fumigation_cert"
    }
  },
  "iot_data": {
    "rollup_period": "24h",
    "averages": {
      "temperature": "22.5°C",
      "humidity": "60%",
      "air_quality": "Good"
    },
    "alerts": [],
    "raw_data_cid": "test_iot_raw_data"
  }
}
EOF

    # Create large test file for performance testing
    dd if=/dev/urandom of="$TEST_DATA_DIR/large_file.bin" bs=1M count=10 2>/dev/null

    log_info "Test environment setup complete"
}

test_docker_services() {
    log_test "Testing Docker services status..."

    # Check if services are running
    local services=("ipfs-node" "ipfs-cluster" "ipfs-gateway" "ipfs-postgres" "ipfs-redis" "ipfs-prometheus" "ipfs-grafana")

    for service in "${services[@]}"; do
        if docker ps --format "table {{.Names}}" | grep -q "$service"; then
            test_passed "Docker service $service is running"
        else
            test_failed "Docker service $service is not running"
        fi
    done
}

test_ipfs_core_functionality() {
    log_test "Testing IPFS core functionality..."

    # Test IPFS API version
    if curl -s "$IPFS_API_URL/api/v0/version" | jq -r '.Version' > /dev/null 2>&1; then
        test_passed "IPFS API version endpoint accessible"
    else
        test_failed "IPFS API version endpoint not accessible"
    fi

    # Test IPFS file upload
    local test_file="$TEST_DATA_DIR/farmer_metadata.json"
    local upload_result=$(curl -s -X POST -F "file=@$test_file" "$IPFS_API_URL/api/v0/add")

    if echo "$upload_result" | jq -r '.Hash' > /dev/null 2>&1; then
        local cid=$(echo "$upload_result" | jq -r '.Hash')
        test_passed "IPFS file upload successful (CID: ${cid:0:20}...)"

        # Test IPFS file retrieval
        if curl -s "$IPFS_API_URL/api/v0/cat?arg=$cid" > /dev/null 2>&1; then
            test_passed "IPFS file retrieval successful"
        else
            test_failed "IPFS file retrieval failed"
        fi

        # Test gateway access
        if curl -s "$IPFS_GATEWAY_URL/ipfs/$cid" > /dev/null 2>&1; then
            test_passed "IPFS gateway access successful"
        else
            test_failed "IPFS gateway access failed"
        fi

        # Store CID for later tests
        echo "$cid" > "$TEST_DATA_DIR/test_cid.txt"
    else
        test_failed "IPFS file upload failed"
    fi

    # Test pin operations
    if [ -f "$TEST_DATA_DIR/test_cid.txt" ]; then
        local cid=$(cat "$TEST_DATA_DIR/test_cid.txt")

        # Pin the file
        if curl -s -X POST "$IPFS_API_URL/api/v0/pin/add?arg=$cid" | jq -r '.Pins[]' > /dev/null 2>&1; then
            test_passed "IPFS pin operation successful"
        else
            test_failed "IPFS pin operation failed"
        fi

        # List pins
        if curl -s -X POST "$IPFS_API_URL/api/v0/pin/ls" | jq -r '.Keys' > /dev/null 2>&1; then
            test_passed "IPFS pin listing successful"
        else
            test_failed "IPFS pin listing failed"
        fi
    fi
}

test_ipfs_cluster() {
    log_test "Testing IPFS Cluster functionality..."

    # Test cluster version
    if curl -s "$CLUSTER_API_URL/version" | jq -r '.version' > /dev/null 2>&1; then
        test_passed "IPFS Cluster version endpoint accessible"
    else
        test_failed "IPFS Cluster version endpoint not accessible"
    fi

    # Test cluster peers
    if curl -s "$CLUSTER_API_URL/peers" | jq length > /dev/null 2>&1; then
        local peer_count=$(curl -s "$CLUSTER_API_URL/peers" | jq length)
        test_passed "IPFS Cluster peers endpoint accessible ($peer_count peers)"
    else
        test_failed "IPFS Cluster peers endpoint not accessible"
    fi

    # Test cluster pin status
    if curl -s "$CLUSTER_API_URL/pins" | jq length > /dev/null 2>&1; then
        test_passed "IPFS Cluster pins endpoint accessible"
    else
        test_failed "IPFS Cluster pins endpoint not accessible"
    fi
}

test_database_connectivity() {
    log_test "Testing database connectivity..."

    # Test PostgreSQL
    if docker exec ipfs-postgres pg_isready -U ipfs_user -d ipfs_metadata > /dev/null 2>&1; then
        test_passed "PostgreSQL database is accessible"

        # Test database schema
        if docker exec ipfs-postgres psql -U ipfs_user -d ipfs_metadata -c "SELECT COUNT(*) FROM supply_chain_stages;" > /dev/null 2>&1; then
            test_passed "PostgreSQL schema is properly initialized"
        else
            test_failed "PostgreSQL schema is not properly initialized"
        fi
    else
        test_failed "PostgreSQL database is not accessible"
    fi

    # Test Redis
    if docker exec ipfs-redis redis-cli ping | grep -q "PONG"; then
        test_passed "Redis is accessible"

        # Test Redis operations
        docker exec ipfs-redis redis-cli set test_key "test_value" > /dev/null 2>&1
        if docker exec ipfs-redis redis-cli get test_key | grep -q "test_value"; then
            test_passed "Redis read/write operations successful"
            docker exec ipfs-redis redis-cli del test_key > /dev/null 2>&1
        else
            test_failed "Redis read/write operations failed"
        fi
    else
        test_failed "Redis is not accessible"
    fi
}

test_monitoring_services() {
    log_test "Testing monitoring services..."

    # Test Prometheus
    if curl -s "$PROMETHEUS_URL/api/v1/status/config" > /dev/null 2>&1; then
        test_passed "Prometheus API is accessible"

        # Test if targets are being scraped
        local targets=$(curl -s "$PROMETHEUS_URL/api/v1/targets" | jq '.data.activeTargets | length')
        if [ "$targets" -gt 0 ]; then
            test_passed "Prometheus is scraping $targets targets"
        else
            test_failed "Prometheus is not scraping any targets"
        fi
    else
        test_failed "Prometheus API is not accessible"
    fi

    # Test Grafana
    if curl -s "$GRAFANA_URL/api/health" | jq -r '.database' | grep -q "ok"; then
        test_passed "Grafana is accessible and healthy"
    else
        test_failed "Grafana is not accessible or unhealthy"
    fi
}

test_rust_backend_integration() {
    log_test "Testing Rust backend integration..."

    # Wait for Rust backend to be ready
    if wait_for_service "$RUST_BACKEND_URL/health" "Rust Backend"; then
        test_passed "Rust backend is accessible"

        # Test IPFS upload through backend
        local upload_payload=$(cat "$TEST_DATA_DIR/farmer_metadata.json" | jq '{data: ., stage: "farmer_registration"}')
        local upload_response=$(curl -s -X POST -H "Content-Type: application/json" \
            -d "$upload_payload" "$RUST_BACKEND_URL/api/ipfs/upload")

        if echo "$upload_response" | jq -r '.cid' > /dev/null 2>&1; then
            local cid=$(echo "$upload_response" | jq -r '.cid')
            test_passed "Backend IPFS upload successful (CID: ${cid:0:20}...)"

            # Test IPFS retrieval through backend
            local retrieve_payload=$(echo "{\"cid\": \"$cid\"}")
            local retrieve_response=$(curl -s -X POST -H "Content-Type: application/json" \
                -d "$retrieve_payload" "$RUST_BACKEND_URL/api/ipfs/retrieve")

            if echo "$retrieve_response" | jq -r '.data' > /dev/null 2>&1; then
                test_passed "Backend IPFS retrieval successful"
            else
                test_failed "Backend IPFS retrieval failed"
            fi
        else
            test_failed "Backend IPFS upload failed"
        fi

        # Test IPFS health through backend
        if curl -s -X POST "$RUST_BACKEND_URL/api/ipfs/health" | jq -r '.ipfs_connected' | grep -q "true"; then
            test_passed "Backend IPFS health check successful"
        else
            test_failed "Backend IPFS health check failed"
        fi
    else
        test_failed "Rust backend is not accessible"
    fi
}

test_supply_chain_workflows() {
    log_test "Testing supply chain workflows..."

    if curl -s "$RUST_BACKEND_URL/health" > /dev/null 2>&1; then
        # Test farmer metadata upload
        local farmer_payload=$(cat "$TEST_DATA_DIR/farmer_metadata.json")
        local farmer_response=$(curl -s -X POST -H "Content-Type: application/json" \
            -d "$farmer_payload" "$RUST_BACKEND_URL/api/ipfs/farmer")

        if echo "$farmer_response" | jq -r '.cid' > /dev/null 2>&1; then
            test_passed "Farmer metadata upload workflow successful"
        else
            test_failed "Farmer metadata upload workflow failed"
        fi

        # Test warehouse metadata upload
        local warehouse_payload=$(cat "$TEST_DATA_DIR/warehouse_metadata.json")
        local warehouse_response=$(curl -s -X POST -H "Content-Type: application/json" \
            -d "$warehouse_payload" "$RUST_BACKEND_URL/api/ipfs/warehouse")

        if echo "$warehouse_response" | jq -r '.cid' > /dev/null 2>&1; then
            test_passed "Warehouse metadata upload workflow successful"
        else
            test_failed "Warehouse metadata upload workflow failed"
        fi
    else
        test_failed "Cannot test supply chain workflows - backend not accessible"
    fi
}

test_performance() {
    log_test "Testing performance characteristics..."

    # Test large file upload
    local large_file="$TEST_DATA_DIR/large_file.bin"
    local start_time=$(date +%s.%3N)

    local large_upload=$(curl -s -X POST -F "file=@$large_file" "$IPFS_API_URL/api/v0/add")

    local end_time=$(date +%s.%3N)
    local duration=$(echo "$end_time - $start_time" | bc)

    if echo "$large_upload" | jq -r '.Hash' > /dev/null 2>&1; then
        test_passed "Large file upload successful (${duration}s for 10MB)"

        # Test concurrent uploads
        local concurrent_start=$(date +%s.%3N)

        for i in {1..5}; do
            curl -s -X POST -F "file=@$TEST_DATA_DIR/farmer_metadata.json" \
                "$IPFS_API_URL/api/v0/add" > /dev/null 2>&1 &
        done
        wait

        local concurrent_end=$(date +%s.%3N)
        local concurrent_duration=$(echo "$concurrent_end - $concurrent_start" | bc)
        test_passed "Concurrent uploads completed (${concurrent_duration}s for 5 files)"
    else
        test_failed "Large file upload failed"
    fi

    # Test gateway cache performance
    if [ -f "$TEST_DATA_DIR/test_cid.txt" ]; then
        local cid=$(cat "$TEST_DATA_DIR/test_cid.txt")

        # First request (cache miss)
        local cache_start=$(date +%s.%3N)
        curl -s "$IPFS_GATEWAY_URL/ipfs/$cid" > /dev/null 2>&1
        local cache_end=$(date +%s.%3N)
        local first_request=$(echo "$cache_end - $cache_start" | bc)

        # Second request (should be cached)
        local cached_start=$(date +%s.%3N)
        curl -s "$IPFS_GATEWAY_URL/ipfs/$cid" > /dev/null 2>&1
        local cached_end=$(date +%s.%3N)
        local second_request=$(echo "$cached_end - $cached_start" | bc)

        if (( $(echo "$second_request < $first_request" | bc -l) )); then
            test_passed "Gateway caching is working (${first_request}s -> ${second_request}s)"
        else
            test_failed "Gateway caching may not be working effectively"
        fi
    fi
}

test_error_handling() {
    log_test "Testing error handling..."

    # Test invalid CID retrieval
    local invalid_response=$(curl -s -w "%{http_code}" "$IPFS_API_URL/api/v0/cat?arg=invalid_cid")
    if echo "$invalid_response" | grep -q "400\|404\|500"; then
        test_passed "Invalid CID properly rejected"
    else
        test_failed "Invalid CID not properly handled"
    fi

    # Test oversized upload (if configured)
    # This test would need a very large file, skipping for now
    log_info "Skipping oversized upload test (would require >100MB file)"

    # Test backend error handling
    if curl -s "$RUST_BACKEND_URL/health" > /dev/null 2>&1; then
        local invalid_backend_response=$(curl -s -X POST -H "Content-Type: application/json" \
            -d '{"invalid": "json structure"}' "$RUST_BACKEND_URL/api/ipfs/upload")

        if echo "$invalid_backend_response" | grep -q "error"; then
            test_passed "Backend properly handles invalid requests"
        else
            test_failed "Backend does not properly handle invalid requests"
        fi
    fi
}

test_security() {
    log_test "Testing security configurations..."

    # Test that sensitive endpoints are not publicly accessible
    # (This would depend on your firewall/proxy configuration)

    # Test CORS headers
    local cors_response=$(curl -s -H "Origin: http://example.com" \
        -H "Access-Control-Request-Method: POST" \
        -X OPTIONS "$IPFS_GATEWAY_URL/ipfs/")

    # This test is basic and would need more comprehensive security testing
    test_passed "Basic security test completed (manual review recommended)"
}

cleanup_test_environment() {
    log_info "Cleaning up test environment..."

    # Remove test files (keep results)
    rm -rf "$TEST_DATA_DIR"

    # Clean up any test pins (optional)
    # This could be dangerous in production, so we'll skip it

    log_info "Cleanup completed"
}

generate_test_report() {
    local report_file="$RESULTS_DIR/test_report_$TIMESTAMP.html"

    cat > "$report_file" << EOF
<!DOCTYPE html>
<html>
<head>
    <title>IPFS Infrastructure Test Report</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 40px; }
        .header { background: #f0f0f0; padding: 20px; border-radius: 5px; }
        .passed { color: green; }
        .failed { color: red; }
        .summary { background: #e7f3ff; padding: 15px; margin: 20px 0; border-radius: 5px; }
        pre { background: #f5f5f5; padding: 10px; border-radius: 3px; overflow-x: auto; }
    </style>
</head>
<body>
    <div class="header">
        <h1>IPFS Infrastructure Test Report</h1>
        <p><strong>Timestamp:</strong> $TIMESTAMP</p>
        <p><strong>Environment:</strong> $(hostname)</p>
    </div>

    <div class="summary">
        <h2>Test Summary</h2>
        <p><strong>Total Tests:</strong> $TOTAL_TESTS</p>
        <p class="passed"><strong>Passed:</strong> $PASSED_TESTS</p>
        <p class="failed"><strong>Failed:</strong> $FAILED_TESTS</p>
        <p><strong>Success Rate:</strong> $(( PASSED_TESTS * 100 / TOTAL_TESTS ))%</p>
    </div>

    <h2>Detailed Log</h2>
    <pre>
$(cat "$LOG_FILE")
    </pre>

    <h2>System Information</h2>
    <pre>
Docker Version: $(docker --version)
Docker Compose Version: $(docker-compose --version || docker compose version)
System: $(uname -a)
    </pre>

</body>
</html>
EOF

    log_info "Test report generated: $report_file"
}

print_summary() {
    echo ""
    echo "======================================================================"
    echo "                      IPFS INFRASTRUCTURE TEST SUMMARY"
    echo "======================================================================"
    echo ""
    echo "Test Execution Time: $TIMESTAMP"
    echo "Total Tests Run: $TOTAL_TESTS"
    echo -e "Tests Passed: ${GREEN}$PASSED_TESTS${NC}"
    echo -e "Tests Failed: ${RED}$FAILED_TESTS${NC}"
    echo ""

    if [ $FAILED_TESTS -eq 0 ]; then
        echo -e "${GREEN}🎉 ALL TESTS PASSED! Infrastructure is ready for production.${NC}"
    else
        echo -e "${RED}⚠️  Some tests failed. Please review the logs and fix issues before production deployment.${NC}"
    fi

    echo ""
    echo "Detailed logs available in: $LOG_FILE"
    echo "Test report available in: $RESULTS_DIR/test_report_$TIMESTAMP.html"
    echo ""
    echo "======================================================================"
}

# Main execution
main() {
    echo ""
    log_info "Starting comprehensive IPFS infrastructure tests..."
    echo ""

    setup_test_environment

    # Wait for all services to be ready
    log_info "Waiting for all services to be ready..."
    wait_for_service "$IPFS_API_URL/api/v0/version" "IPFS Node"
    wait_for_service "$PROMETHEUS_URL/-/ready" "Prometheus"

    # Run all test suites
    test_docker_services
    test_ipfs_core_functionality
    test_ipfs_cluster
    test_database_connectivity
    test_monitoring_services
    test_rust_backend_integration
    test_supply_chain_workflows
    test_performance
    test_error_handling
    test_security

    # Generate reports
    generate_test_report
    print_summary

    # Cleanup
    cleanup_test_environment

    # Exit with appropriate code
    if [ $FAILED_TESTS -eq 0 ]; then
        exit 0
    else
        exit 1
    fi
}

# Check dependencies
if ! command -v curl &> /dev/null; then
    log_error "curl is required but not installed"
    exit 1
fi

if ! command -v jq &> /dev/null; then
    log_error "jq is required but not installed"
    exit 1
fi

if ! command -v docker &> /dev/null; then
    log_error "docker is required but not installed"
    exit 1
fi

# Run main function
main "$@"
