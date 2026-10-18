# API reference

Base: `http://<HOST>:<PORT>`. JSON bodies are limited to 256 KiB, uploads to 1 MiB.

**Authentication:** `Authorization: Bearer <token>`. Use either the admin token (`ADMIN_API_TOKEN`) or a
participant token returned once by `POST /api/admin/participants`. "Public" endpoints need no token.

**Writes:** send `Idempotency-Key: <unique per logical request>`. Retrying with the same key returns or
resumes the same operation. Reusing a key with a different body returns 409.

**Write responses** are an *operation view*:
`{operation_id, kind, status, retryable, tx_hash, block_number, record_digest, content_hash, cid, ipfs_url, encrypted, error, result}`.

| HTTP | `status` | Meaning |
|---|---|---|
| 200 | `confirmed` | Mined; side effects applied. |
| 202 | `prepared` / `stored` / `submitted` | In progress. Poll `GET /api/operations/{id}` or retry with the same key. |
| 409 | `failed` / `reverted` | `error` holds the contract's custom error, e.g. `NotCustodian`. |

Errors: `{error, code}`, where `code` is one of `bad_request`, `unauthenticated`, `forbidden`, `not_found`,
`conflict`, `contract_rejected`, `ipfs_error`, `chain_error`, `unavailable` or `internal_error`.

## Meta
| Method | Path | Auth | Notes |
|---|---|---|---|
| GET | `/health` | public | liveness |
| GET | `/api/health` | public | chain, IPFS and DB checked individually; 503 if any fails |
| GET | `/api/config` | public | chain id, contract addresses, gateway, supported crops |

## Participants
| Method | Path | Auth | Body |
|---|---|---|---|
| POST | `/api/admin/participants` | admin | `{name, roles: ["fpo"\|"warehouse"\|"logistics"\|"processor"\|"packager"\|"ai_oracle"\|"retailer"]}` → creates a wallet, grants roles on-chain and optionally funds gas; returns `api_token` once |
| GET | `/api/participants` | participant | directory (no secrets) |
| GET | `/api/me` | participant | roles in the DB vs roles actually held on-chain, balance |

## Supply chain (writes)
| Method | Path | Role | Body (main fields) |
|---|---|---|---|
| POST | `/api/farmer/register` | FPO | `{label, region?{state,district}, primary_crop?, wallet_address?, profile{name, phone?, village?, land_area_ha?, external_farmer_id?, crops[]}}`. Encrypted. Returns `farmer_id` |
| POST | `/api/fpo/purchase` | FPO | `{farmer_id, label, crop, variety?, season?, delivered_quantity_kg, committed_quantity_kg?, promised_delivery_date?, delivery_date, price_per_quintal_inr?, moisture_pct?, payment_reference?, attachments[]}`. Returns `batch_id` |
| POST | `/api/custody/transfer` | custodian | `{batch_id, to_participant_id, quantity_kg?, handover_at?, notes?}` |
| POST | `/api/warehouse/inspection` (alias `/api/warehouse/update`) | WAREHOUSE + custodian | `{batch_id, warehouse_ref, inspection{passed, grade?, moisture_pct?, foreign_matter_pct?, oil_content_pct?, aflatoxin_ppb?, inspected_at, inspector_ref?}, storage?{temperature_c?, humidity_pct?, iot_rollup_cid?}, attachments[]}` |
| POST | `/api/logistics/record` | LOGISTICS + custodian | `{batch_id, shipment_ref, checkpoint{name, at}, delivered, expected_arrival?, actual_arrival?, gps_trace_cid?, vehicle_ref?, distance_km?}` |
| POST | `/api/processing/batch` | PROCESSOR + custodian | `{input_batch_id, method, outputs[{label, product, quantity_kg}], lab_report?{...}, completed_at?}`. Returns output `batch_id`s |
| POST | `/api/packaging/sku` | PACKAGER + custodian of a product batch | `{batch_id, label, unit_ids[], package?{pack_size?, material?, best_before?, fssai_license_ref?, label_artwork_cid?}}`. Returns `sku_id`, `units_root` |
| POST | `/api/fraud/report` | any participant | `{batch_id \| sku_id, category, description, evidence_cids[]}`. Encrypted |
| POST | `/api/fraud/resolve` | admin (not the reporter) | `{report_digest, upheld, attribution: farmer\|fpo\|warehouse\|logistics\|processor\|packager\|unattributed, notes}` |
| POST | `/api/ai/commit` | AI_ORACLE | `{batch_id, assessment{model_id, model_version, method, inputs[{cid, description}], outputs{...}, limitations?}}` |
| POST | `/api/ai/reveal` | the committing oracle | `{batch_id}`. 409 before the 1 h window opens (the reconciler reveals automatically) |
| GET | `/api/ipfs/pins/{cid}` | public | backup replication state of one block |
| GET | `/api/admin/pins?status=pending\|remote_pending\|pinned\|failed` | admin | backup pin queue |
| POST | `/api/ipfs/upload` | participant | raw bytes (≤ 1 MiB) → `{cid, sha256, size, gateway_url}`. **Public on IPFS: no personal data** |
| POST | `/api/workflow/execute` | admin | full chain in one call (see `backend/src/workflows.rs`); resumable by `workflow_key` |

## Reads and verification
| Method | Path | Auth | Notes |
|---|---|---|---|
| GET | `/api/operations/{id}` | owner/admin | also advances a `submitted` op |
| GET | `/api/farmers`, `/api/farmers/{id}` | participant | pseudonymous label, region, crop, batches |
| GET | `/api/farmers/{id}/verify` (also `POST /api/farmer/verify`, `/api/workflow/verify-farmer`) | public | registration record verified; contents not disclosed |
| GET | `/api/batches` | participant | index |
| GET | `/api/batches/{id}` | public | on-chain state |
| GET | `/api/batches/{id}/provenance` | public | full lineage, every record re-verified (docs/INTEGRITY.md) |
| GET | `/api/skus/{id}/verify?unit_id=` | public | SKU origin, on-chain Merkle unit check and provenance (also `POST /api/workflow/verify-sku`, `/api/packaging/verify`) |
| GET | `/api/records/{digest}/verify` | public | single record |
| GET | `/api/ai/{batch_id}` | participant | commit/reveal state and window in chain time |

## NFTs
| Method | Path | Auth | Notes |
|---|---|---|---|
| POST | `/api/nft/mint-from-tx` | the FPO that anchored the procurement, or admin | `{tx_hash}` or `{record_digest}`. The tx must be a confirmed procurement whose record verifies. One token per record |
| POST | `/api/nft/credential` | the registering FPO, or admin | `{farmer_id}`. Needs the farmer's `wallet_address`; soulbound |
| GET | `/api/nft/provenance/{token_id}` | public | owner, custody alignment, metadata (CID-checked) and source-record verification |
| GET | `/api/nft/credential/{token_id}` | public | owner, `soulbound`, metadata check |
| GET | `/api/nfts` | participant | tokens this server has seen confirmed |

## AI
| Method | Path | Auth | Notes |
|---|---|---|---|
| GET | `/api/rankings/farmers?limit&offset&crop&sort=score\|lower_bound&min_evidence` | participant | docs/RANKING.md |
| GET | `/api/rankings/farmers/{id}` | participant | one farmer with evidence |
| GET | `/api/advisory/status` | public | whether the local model is configured, reachable and pulled |
| POST | `/api/advisory` | participant | `{question, language, farmer_id?, crop?, region?}`. 503 when no local model is configured |
