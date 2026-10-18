#!/usr/bin/env bash
# Runs the backend end-to-end test against real anvil + Kubo processes (spawned by the test).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export ANVIL_BIN="${ANVIL_BIN:-$(command -v anvil)}"
export IPFS_BIN="${IPFS_BIN:-$(command -v ipfs)}"
cd "$ROOT/backend"
cargo test --test e2e -- --ignored --nocapture
