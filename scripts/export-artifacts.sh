#!/usr/bin/env bash
# Builds the contracts and exports trimmed Foundry artifacts (ABI + bytecode) into backend/abi/.
# The backend's alloy `sol!` bindings are generated from these files, so contract and backend stay in sync.
# `--check` fails if the committed artifacts differ from a fresh build (used by CI).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/contracts"
forge build --silent
OUT_DIR="$ROOT/backend/abi"
if [[ "${1:-}" == "--check" ]]; then
  OUT_DIR="$(mktemp -d)"
fi
mkdir -p "$OUT_DIR"
for name in OilseedValueChain BatchProvenanceNFT FarmerCredential; do
  python3 - "$ROOT/contracts/out/$name.sol/$name.json" "$OUT_DIR/$name.json" <<'PY'
import json, sys
src, dst = sys.argv[1], sys.argv[2]
a = json.load(open(src))
trimmed = {
    "abi": a["abi"],
    "bytecode": {"object": a["bytecode"]["object"]},
    "deployedBytecode": {"object": a["deployedBytecode"]["object"]},
}
with open(dst, "w") as f:
    json.dump(trimmed, f, indent=1, sort_keys=True)
    f.write("\n")
PY
done
if [[ "${1:-}" == "--check" ]]; then
  diff -r "$OUT_DIR" "$ROOT/backend/abi" && echo "backend/abi is up to date"
else
  echo "exported artifacts to backend/abi/"
fi
