#!/usr/bin/env bash
# Everything CI runs, locally. Requires forge; the e2e step also needs anvil and ipfs (skipped if absent).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FORGE="${FORGE_BIN:-forge}"
step() { printf '\n== %s\n' "$*"; }

step "contracts: fmt / build / test"
(cd "$ROOT/contracts" && "$FORGE" fmt --check && "$FORGE" build --sizes && "$FORGE" test)
step "contracts: gas snapshot is current"
(cd "$ROOT/contracts" && "$FORGE" snapshot --check --no-match-contract Invariant)
step "backend ABI artifacts are current"
PATH="$(dirname "$(command -v "$FORGE")"):$PATH" "$ROOT/scripts/export-artifacts.sh" --check
step "backend: fmt / clippy / tests"
(cd "$ROOT/backend" && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test)
step "frontend: syntax"
for f in "$ROOT"/frontend/*.js; do node --check "$f"; done
if command -v "${ANVIL_BIN:-anvil}" >/dev/null && command -v "${IPFS_BIN:-ipfs}" >/dev/null; then
  step "end-to-end (anvil + Kubo)"
  "$ROOT/scripts/e2e.sh"
else
  step "end-to-end SKIPPED: anvil and/or ipfs not on PATH"
fi
