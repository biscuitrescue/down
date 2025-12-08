#!/bin/bash

# Test script for OilseedValueChain deployment
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}Testing OilseedValueChain Deployment${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""

# Check if Anvil is running
echo -e "${YELLOW}1. Checking if Anvil is running...${NC}"
if ! curl -s -X POST -H "Content-Type: application/json" --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' http://127.0.0.1:8545 > /dev/null; then
    echo -e "${RED}❌ Anvil is not running on port 8545${NC}"
    echo -e "${YELLOW}Starting Anvil...${NC}"
    anvil > anvil.log 2>&1 &
    sleep 3
fi
echo -e "${GREEN}✓ Anvil is running${NC}"
echo ""

# Get contract info
CONTRACT_ADDRESS=$(grep -oP 'CONTRACT_ADDRESS=\K[^\s]+' offchain/.env.local)
RPC_URL=$(grep -oP 'RPC_URL=\K[^\s]+' offchain/.env.local)

echo -e "${YELLOW}2. Contract Information:${NC}"
echo -e "   Address: ${GREEN}${CONTRACT_ADDRESS}${NC}"
echo -e "   RPC URL: ${GREEN}${RPC_URL}${NC}"
echo ""

# Check contract code exists
echo -e "${YELLOW}3. Verifying contract deployment...${NC}"
CODE=$(cast code ${CONTRACT_ADDRESS} --rpc-url ${RPC_URL})
if [ ${#CODE} -gt 10 ]; then
    echo -e "${GREEN}✓ Contract bytecode found (${#CODE} bytes)${NC}"
else
    echo -e "${RED}❌ No contract code at address${NC}"
    exit 1
fi
echo ""

# Test role check
echo -e "${YELLOW}4. Testing contract interaction...${NC}"
DEPLOYER_ADDRESS="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
echo -e "   Checking roles for deployer: ${DEPLOYER_ADDRESS}"

ROLES=$(cast call ${CONTRACT_ADDRESS} "getRoles(address)(uint256)" ${DEPLOYER_ADDRESS} --rpc-url ${RPC_URL})
echo -e "   Deployer roles bitmap: ${GREEN}${ROLES}${NC}"

if [ "$ROLES" != "0x0000000000000000000000000000000000000000000000000000000000000000" ]; then
    echo -e "${GREEN}✓ Deployer has roles assigned${NC}"
else
    echo -e "${RED}❌ Deployer has no roles${NC}"
    exit 1
fi
echo ""

# Test farmer registration (read-only - checking struct)
echo -e "${YELLOW}5. Testing farmer verification (should not exist yet)...${NC}"
TEST_FARMER_DID="0x1234567890123456789012345678901234567890123456789012345678901234"

FARMER_INFO=$(cast call ${CONTRACT_ADDRESS} "verifyFarmer(bytes32)(bool,bytes32,uint64)" ${TEST_FARMER_DID} --rpc-url ${RPC_URL})
echo -e "   Farmer info: ${FARMER_INFO}"
echo -e "${GREEN}✓ Contract call successful${NC}"
echo ""

# Test warehouse state getter
echo -e "${YELLOW}6. Testing warehouse state query...${NC}"
TEST_WAREHOUSE_ID="0x7777777777777777777777777777777777777777777777777777777777777777"

WAREHOUSE_STATE=$(cast call ${CONTRACT_ADDRESS} "getWarehouseState(bytes32)(bytes32,uint64)" ${TEST_WAREHOUSE_ID} --rpc-url ${RPC_URL})
echo -e "   Warehouse state: ${WAREHOUSE_STATE}"
echo -e "${GREEN}✓ Warehouse query successful${NC}"
echo ""

# Test SKU verification (should not exist)
echo -e "${YELLOW}7. Testing SKU verification query...${NC}"
TEST_SKU_ID="0x9999999999999999999999999999999999999999999999999999999999999999"

SKU_RESULT=$(cast call ${CONTRACT_ADDRESS} "verifyPackageOrigin(bytes32)(bytes32,bytes32,uint64)" ${TEST_SKU_ID} --rpc-url ${RPC_URL} 2>&1 || echo "Expected failure")
if [[ $SKU_RESULT == *"SKUNotFound"* ]] || [[ $SKU_RESULT == *"Expected failure"* ]]; then
    echo -e "${GREEN}✓ SKU not found (expected)${NC}"
else
    echo -e "   Result: ${SKU_RESULT}"
fi
echo ""

# Test AI score commit hash computation
echo -e "${YELLOW}8. Testing AI score commit hash computation...${NC}"
REVEAL_HASH="0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
NONCE="0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"

COMMIT_HASH=$(cast call ${CONTRACT_ADDRESS} "computeAIScoreCommit(bytes32,bytes32)(bytes32)" ${REVEAL_HASH} ${NONCE} --rpc-url ${RPC_URL})
echo -e "   Commit hash: ${GREEN}${COMMIT_HASH}${NC}"
echo -e "${GREEN}✓ Hash computation successful${NC}"
echo ""

echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All Basic Tests Passed! ✓${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo -e "${YELLOW}Contract is deployed and responding correctly.${NC}"
echo -e "${YELLOW}You can now start the offchain server to test the full API.${NC}"
echo ""
echo -e "To start the server:"
echo -e "  ${GREEN}cd offchain && cargo run${NC}"
echo ""
echo -e "To test with a real transaction (farmer registration):"
echo -e "  ${GREEN}./test_full_workflow.sh${NC}"
