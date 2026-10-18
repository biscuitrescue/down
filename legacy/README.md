# Legacy files (not built, not used)

These files were moved here during the 2026-10 restructure instead of being deleted. Delete this folder
once you no longer need them for reference.

| File | What it was | Why it is retired |
|---|---|---|
| `nyx.sol` | A divergent copy of the contract (FPO registers farmers, extra `metadataHash`) | It did not compile (`FarmerRegistered` was emitted with 4 of 5 arguments). Its intent, FPO-led registration, is kept in `contracts/src/OilseedValueChain.sol`. |
| `foundry-template/Counter*.sol` | The untouched Foundry template | It was the only thing CI tested. |
| `scripts/mock_ipfs_server.js`, `package.json` | Express server that faked the IPFS API | Its CIDs (`Qm` + hex) were not valid CIDs and content was lost on restart. Local development now uses a real Kubo node (`scripts/dev-stack.sh`). |
| `scripts/test_api.sh`, `test_server.sh`, `test_deployment.sh` | curl smoke scripts | They targeted the old API. `test_server.sh` passed on HTTP 500, and the AI reveal test could never pass. They are replaced by `backend/tests/e2e.rs`. |

The `nyx`, `off` and `cafo` git branches were not merged. Notably, the 21k-line `nyx` branch modules
(privacy, schema, IPFS providers, indexer) were never wired into the server binary on that branch.
