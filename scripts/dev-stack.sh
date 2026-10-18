#!/usr/bin/env bash
# Local development stack: anvil (chain 31337) + Kubo IPFS (offline) + contract deployment + backend + frontend.
# Everything lives in .devstack/ (gitignored). Ctrl-C stops all processes. Never use these keys outside anvil.
#
# Binaries are taken from PATH, or from ANVIL_BIN / FORGE_BIN / IPFS_BIN.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STACK="$ROOT/.devstack"
ANVIL="${ANVIL_BIN:-anvil}"
FORGE="${FORGE_BIN:-forge}"
IPFS="${IPFS_BIN:-ipfs}"
RPC_PORT="${RPC_PORT:-8545}"
IPFS_API_PORT="${IPFS_API_PORT:-5001}"
IPFS_GW_PORT="${IPFS_GW_PORT:-8080}"
API_PORT="${PORT:-3000}"
# anvil's well-known account #0 — local development only
ANVIL_KEY0="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

missing=()
for bin in "$ANVIL" "$FORGE" "$IPFS" cargo curl openssl python3; do
  command -v "$bin" >/dev/null || missing+=("$bin")
done
if (( ${#missing[@]} )); then
  echo "missing on PATH: ${missing[*]}" >&2
  echo "install Foundry (foundryup) and Kubo, or set ANVIL_BIN / FORGE_BIN / IPFS_BIN" >&2
  exit 1
fi

mkdir -p "$STACK"
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT INT TERM

echo "▶ anvil on :$RPC_PORT"
"$ANVIL" --port "$RPC_PORT" --silent --state "$STACK/anvil-state.json" >"$STACK/anvil.log" 2>&1 &
pids+=($!)

export IPFS_PATH="$STACK/ipfs"
if [[ ! -f "$IPFS_PATH/config" ]]; then
  "$IPFS" init --profile=test >/dev/null
fi
# Set the ports on every start so an existing repo cannot keep stale ones.
"$IPFS" config Addresses.API "/ip4/127.0.0.1/tcp/$IPFS_API_PORT"
"$IPFS" config Addresses.Gateway "/ip4/127.0.0.1/tcp/$IPFS_GW_PORT"
echo "▶ Kubo (offline) API :$IPFS_API_PORT gateway :$IPFS_GW_PORT"
"$IPFS" daemon --offline >"$STACK/ipfs.log" 2>&1 &
pids+=($!)

ready=0
for _ in $(seq 1 50); do
  curl -sf -X POST "http://127.0.0.1:$IPFS_API_PORT/api/v0/version" >/dev/null && \
  curl -sf -H 'content-type: application/json' --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' "http://127.0.0.1:$RPC_PORT" >/dev/null && { ready=1; break; }
  sleep 0.2
done
if [[ $ready == 0 ]]; then
  echo "anvil or Kubo did not start; see $STACK/anvil.log and $STACK/ipfs.log" >&2
  exit 1
fi

DEPLOYMENT="$ROOT/contracts/deployments/31337.json"
needs_deploy=1
if [[ -f "$DEPLOYMENT" ]]; then
  addr=$(python3 -c "import json;print(json.load(open('$DEPLOYMENT'))['OilseedValueChain'])")
  code=$(curl -s -H 'content-type: application/json' --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getCode\",\"params\":[\"$addr\",\"latest\"]}" "http://127.0.0.1:$RPC_PORT" | python3 -c "import json,sys;print(json.load(sys.stdin)['result'])")
  [[ "$code" != "0x" ]] && needs_deploy=0
fi
if [[ $needs_deploy == 1 ]]; then
  echo "▶ deploying contracts"
  (cd "$ROOT/contracts" && PRIVATE_KEY="$ANVIL_KEY0" "$FORGE" script script/Deploy.s.sol --rpc-url "http://127.0.0.1:$RPC_PORT" --broadcast --silent)
fi

ENV_FILE="$STACK/backend.env"
# Secrets are generated once and kept: the database's participant wallets are sealed with KEY_ENCRYPTION_KEY.
# Everything else is rewritten on every start so a changed port can never leave a stale URL behind.
SECRETS_FILE="$STACK/secrets.env"
if [[ ! -f "$SECRETS_FILE" ]]; then
  if [[ -f "$ENV_FILE" ]]; then
    grep -E '^(ADMIN_API_TOKEN|KEY_ENCRYPTION_KEY|PII_ENCRYPTION_KEY)=' "$ENV_FILE" >"$SECRETS_FILE"
  else
    cat >"$SECRETS_FILE" <<ENV
ADMIN_API_TOKEN=$(openssl rand -hex 24)
KEY_ENCRYPTION_KEY=$(openssl rand -hex 32)
PII_ENCRYPTION_KEY=$(openssl rand -hex 32)
ENV
  fi
  chmod 600 "$SECRETS_FILE"
fi
{
  cat <<ENV
ENVIRONMENT=development
HOST=127.0.0.1
PORT=$API_PORT
DATABASE_PATH=$STACK/oilschain.db
FRONTEND_DIR=$ROOT/frontend
RPC_URL=http://127.0.0.1:$RPC_PORT
CHAIN_ID=31337
DEPLOYMENT_FILE=$DEPLOYMENT
ADMIN_PRIVATE_KEY=$ANVIL_KEY0
PARTICIPANT_FUNDING_WEI=1000000000000000000
IPFS_API_URL=http://127.0.0.1:$IPFS_API_PORT
IPFS_GATEWAY_URL=http://127.0.0.1:$IPFS_GW_PORT/ipfs/
ENV
  cat "$SECRETS_FILE"
} >"$ENV_FILE"
chmod 600 "$ENV_FILE"
echo "▶ backend on http://127.0.0.1:$API_PORT (admin token in $ENV_FILE)"
set -a; source "$ENV_FILE"; set +a
cd "$ROOT/backend"
cargo run --quiet --bin oilschain-backend
