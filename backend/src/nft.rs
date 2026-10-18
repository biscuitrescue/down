//! NFT lifecycle.
//!
//! * Batch provenance NFT (ERC-721, `BatchProvenanceNFT`): one token per verified procurement record (the
//!   farmer's sale to an FPO). Ownership follows on-chain custody.
//! * Farmer credential (ERC-721 + ERC-5192 soulbound, `FarmerCredential`): issued by the registering FPO to a
//!   farmer-controlled wallet; transfers revert in the contract.
//!
//! Metadata is canonical JSON stored as a raw IPFS block; `tokenURI` is `ipfs://<cid>`. Tokens are shown as
//! minted only after the mint transaction is confirmed.

use crate::auth::Actor;
use crate::canonical::{self, RecordType};
use crate::chain::{hex32, hexaddr, parse_b256, roles, ChainAction};
use crate::cid::Cid;
use crate::db::{self, OpStatus, Operation};
use crate::error::ApiError;
use crate::ops::{self, NewOperation};
use crate::state::AppState;
use crate::supply_chain_handlers::parse_id;
use crate::verify::{self, RecordVerification};
use alloy::primitives::{B256, U256};
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const PROVENANCE_SCHEMA: &str = "oilschain.nft.provenance.v1";
pub const CREDENTIAL_SCHEMA: &str = "oilschain.nft.credential.v1";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MintFromTxRequest {
    /// Hash of the confirmed procurement transaction.
    #[serde(default)]
    pub tx_hash: Option<String>,
    /// Alternatively, the procurement record digest.
    #[serde(default)]
    pub record_digest: Option<String>,
}

/// Resolve and validate the procurement record a mint refers to.
async fn resolve_procurement(
    state: &AppState,
    actor: &Actor,
    req: &MintFromTxRequest,
) -> Result<(B256, Option<B256>), ApiError> {
    let chain = &state.chain_client;
    let (digest, tx) = match (&req.tx_hash, &req.record_digest) {
        (Some(tx), None) => {
            let tx_hash = parse_id("tx_hash", tx)?;
            let receipt = chain
                .receipt(tx_hash)
                .await
                .map_err(|e| ApiError::Chain(e.to_string()))?
                .ok_or_else(|| {
                    ApiError::NotFound("transaction not found or not yet mined".into())
                })?;
            if !receipt.status() {
                return Err(ApiError::Conflict(
                    "transaction reverted; nothing to mint".into(),
                ));
            }
            let procurement: Vec<_> = chain
                .anchored_in_receipt(&receipt)
                .into_iter()
                .filter(|l| l.record_type == RecordType::Procurement.as_u8())
                .collect();
            match procurement.as_slice() {
                [one] => (one.record_digest, Some(tx_hash)),
                [] => {
                    return Err(ApiError::Conflict(
                        "transaction is not an OilsChain procurement (yield sale) record".into(),
                    ))
                }
                _ => {
                    return Err(ApiError::Conflict(
                        "transaction anchors several procurement records; pass record_digest"
                            .into(),
                    ))
                }
            }
        }
        (None, Some(d)) => (parse_id("record_digest", d)?, None),
        _ => {
            return Err(ApiError::bad_request(
                "provide exactly one of tx_hash or record_digest",
            ))
        }
    };
    let anchor = chain
        .registry()
        .anchors(digest)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    if anchor.anchoredAt == 0 || anchor.recordType != RecordType::Procurement.as_u8() {
        return Err(ApiError::Conflict(
            "record is not an anchored procurement record".into(),
        ));
    }
    if !actor.is_admin && anchor.submitter != actor.address {
        return Err(ApiError::Forbidden(
            "only the FPO that recorded the procurement (or an admin) can mint".into(),
        ));
    }
    Ok((digest, tx))
}

async fn names(state: &AppState) -> Result<HashMap<String, String>, ApiError> {
    Ok(state
        .db
        .call(db::list_participants)
        .await?
        .into_iter()
        .map(|p| (p.address.to_lowercase(), p.name))
        .collect())
}

/// POST /api/nft/mint-from-tx (FPO that anchored the procurement, or admin)
pub async fn mint_from_tx(
    State(state): State<AppState>,
    actor: Actor,
    Json(req): Json<MintFromTxRequest>,
) -> Result<Response, ApiError> {
    let op = mint_from_tx_op(&state, &actor, req).await?;
    Ok(ops::respond(&state, &op))
}

pub async fn mint_from_tx_op(
    state: &AppState,
    actor: &Actor,
    req: MintFromTxRequest,
) -> Result<Operation, ApiError> {
    let state = state.clone();
    let actor = actor.clone();
    if !actor.is_admin {
        actor.require_role(roles::FPO, "minting a provenance NFT")?;
    }
    let (digest, tx_from_request) = resolve_procurement(&state, &actor, &req).await?;

    let existing = state
        .chain_client
        .provenance()
        .tokenOfRecord(digest)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    let idem_key = format!("mint-provenance:{}", hex32(digest));
    if !existing.is_zero() {
        // Already minted: return the existing operation if we made it, otherwise report the conflict.
        let d = hex32(digest);
        if let Some(op) = state
            .db
            .call(move |c| {
                c.query_row(
                    "SELECT id FROM operations WHERE kind = 'mint_provenance' AND idem_key = ?1",
                    [format!("mint-provenance:{d}")],
                    |r| r.get::<_, String>(0),
                )
                .optional_ok()
            })
            .await?
        {
            let op = state
                .db
                .call(move |c| db::get_operation(c, &op))
                .await?
                .expect("exists");
            return Ok(op);
        }
        return Err(ApiError::Conflict(format!(
            "procurement record already has token {existing}"
        )));
    }

    // The source record must verify before anything is minted against it.
    let log = state
        .chain_client
        .anchor_log(digest)
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("procurement event not found".into()))?;
    let source = verify::verify_log(&state, &log, &names(&state).await?).await;
    if !source.verified {
        return Err(ApiError::Conflict(format!(
            "procurement record failed verification ({:?}); refusing to mint",
            source.status
        )));
    }
    let data = source.data.clone().unwrap_or(Value::Null);
    let batch_id = log.subject_id;
    let batch_key = hex32(batch_id);
    let label = state
        .db
        .call(move |c| db::get_batch(c, &batch_key))
        .await?
        .map(|b| b.label)
        .or_else(|| data["label"].as_str().map(str::to_string))
        .unwrap_or_else(|| hex32(batch_id));
    let fpo_name = source
        .submitter_name
        .clone()
        .unwrap_or_else(|| source.submitter.clone());

    let mut attributes = vec![json!({"trait_type": "Crop", "value": data["crop"]})];
    if let Some(q) = data["delivered_quantity_kg"].as_f64() {
        attributes
            .push(json!({"trait_type": "Quantity (kg)", "value": q, "display_type": "number"}));
    }
    for (k, name) in [
        ("delivery_date", "Delivery date"),
        ("season", "Season"),
        ("variety", "Variety"),
    ] {
        if let Some(v) = data[k].as_str() {
            attributes.push(json!({"trait_type": name, "value": v}));
        }
    }
    let metadata = json!({
        "name": format!("OilsChain lot {label}"),
        "description": format!(
            "Provenance certificate for oilseed lot {label} procured by {fpo_name}. It links to an on-chain anchored \
             procurement record (see properties); it is not by itself a certificate of quality or ownership of goods."
        ),
        "attributes": attributes,
        "properties": {
            "schema": PROVENANCE_SCHEMA,
            "chain_id": state.chain_client.chain_id,
            "registry": hexaddr(state.chain_client.registry_address),
            "batch_id": hex32(batch_id),
            "farmer_id": data["farmer_id"],
            "source_record_digest": hex32(digest),
            "source_record_cid": log.cid,
            "source_content_hash": hex32(log.content_hash),
            "source_tx_hash": log.tx_hash.or(tx_from_request).map(hex32),
        }
    });
    let bytes = canonical::canonical_json(&metadata).map_err(ApiError::internal)?;
    let cid = state.ipfs_client.put_block(&bytes).await?;
    crate::pinning::enqueue(&state, &cid).await;

    let op = ops::submit(
        &state,
        &actor,
        NewOperation {
            idem_key,
            kind: "mint_provenance",
            request_hash: format!("mint-provenance:{}", hex32(digest)),
            record: None,
            action: ChainAction::MintProvenance {
                record_digest: hex32(digest),
                metadata_cid: cid.to_string(),
            },
            context: json!({
                "result": {"record_digest": hex32(digest), "batch_id": hex32(batch_id), "metadata_cid": cid.to_string(), "token_uri": format!("ipfs://{cid}")},
            }),
        },
    )
    .await?;
    Ok(op)
}

trait OptionalOk<T> {
    fn optional_ok(self) -> rusqlite::Result<Option<T>>;
}
impl<T> OptionalOk<T> for rusqlite::Result<T> {
    fn optional_ok(self) -> rusqlite::Result<Option<T>> {
        rusqlite::OptionalExtension::optional(self)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IssueCredentialRequest {
    pub farmer_id: String,
}

/// POST /api/nft/credential (registering FPO or admin) — soulbound credential to the farmer's own wallet.
pub async fn issue_credential(
    State(state): State<AppState>,
    actor: Actor,
    Json(req): Json<IssueCredentialRequest>,
) -> Result<Response, ApiError> {
    let op = issue_credential_op(&state, &actor, req).await?;
    Ok(ops::respond(&state, &op))
}

pub async fn issue_credential_op(
    state: &AppState,
    actor: &Actor,
    req: IssueCredentialRequest,
) -> Result<Operation, ApiError> {
    let state = state.clone();
    let actor = actor.clone();
    if !actor.is_admin {
        actor.require_role(roles::FPO, "issuing a farmer credential")?;
    }
    let farmer_id = parse_id("farmer_id", &req.farmer_id)?;
    let key = hex32(farmer_id);
    let idem = format!("issue-credential:{key}");
    if let Some(op) = ops::resume_existing(&state, &actor, &idem, &idem).await? {
        return Ok(op);
    }
    let k = key.clone();
    let farmer = state
        .db
        .call(move |c| db::get_farmer(c, &k))
        .await?
        .ok_or_else(|| ApiError::NotFound("farmer not found".into()))?;
    let wallet = farmer.wallet_address.clone().ok_or_else(|| {
        ApiError::Conflict("farmer has no wallet address; a soulbound credential must go to a farmer-controlled wallet".into())
    })?;
    let chain = &state.chain_client;
    let onchain = chain
        .registry()
        .farmers(farmer_id)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    if onchain.registeredAt == 0 {
        return Err(ApiError::NotFound("farmer not registered on-chain".into()));
    }
    if !actor.is_admin && onchain.registeredBy != actor.address {
        return Err(ApiError::Forbidden(
            "only the FPO that registered this farmer can issue the credential".into(),
        ));
    }
    let existing = chain
        .credential()
        .credentialOf(farmer_id)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    if !existing.is_zero() {
        return Err(ApiError::Conflict(format!(
            "farmer already holds credential {existing}"
        )));
    }

    // Evidence: this farmer's confirmed procurement records whose cached content still matches the commitment.
    let fk = key.clone();
    let evidence = state
        .db
        .call(move |c| {
            let mut stmt = c.prepare(
                "SELECT r.record_digest, r.content_hash, r.public_json FROM records r JOIN batches b ON b.batch_id = r.subject_id
                 WHERE b.farmer_id = ?1 AND b.parent_batch_id IS NULL AND r.record_type = 2 ORDER BY r.block_number",
            )?;
            let rows = stmt.query_map([fk], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .await?;
    let mut digests = Vec::new();
    let mut total_kg = 0.0;
    for (digest, content_hash, json) in evidence {
        let Some(json) = json else { continue };
        if hex32(canonical::content_hash(json.as_bytes())) != content_hash {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(&json) {
            total_kg += v["data"]["delivered_quantity_kg"].as_f64().unwrap_or(0.0);
        }
        digests.push(digest);
    }
    let metadata = json!({
        "name": format!("OilsChain farmer credential — {}", farmer.label),
        "description": "Non-transferable attestation by the registering FPO that this farmer is registered and has the \
                        listed procurement records anchored on OilsChain. It is not a credit score or a loan decision.",
        "attributes": [
            {"trait_type": "Verified procurement records", "value": digests.len(), "display_type": "number"},
            {"trait_type": "Total delivered (kg)", "value": (total_kg * 1000.0).round() / 1000.0, "display_type": "number"},
            {"trait_type": "Primary crop", "value": farmer.primary_crop},
        ],
        "properties": {
            "schema": CREDENTIAL_SCHEMA,
            "chain_id": chain.chain_id,
            "registry": hexaddr(chain.registry_address),
            "farmer_id": key,
            "issuer": hexaddr(actor.address),
            "evidence_record_digests": digests,
            "issued_at": db::now(),
        }
    });
    let bytes = canonical::canonical_json(&metadata).map_err(ApiError::internal)?;
    let cid = state.ipfs_client.put_block(&bytes).await?;
    crate::pinning::enqueue(&state, &cid).await;
    let op = ops::submit(
        &state,
        &actor,
        NewOperation {
            idem_key: format!("issue-credential:{key}"),
            kind: "issue_credential",
            request_hash: format!("issue-credential:{key}"),
            record: None,
            action: ChainAction::IssueCredential {
                to: wallet.clone(),
                farmer_id: key.clone(),
                metadata_cid: cid.to_string(),
            },
            context: json!({"result": {"farmer_id": key, "to": wallet, "metadata_cid": cid.to_string(), "token_uri": format!("ipfs://{cid}")}}),
        },
    )
    .await?;
    Ok(op)
}

pub async fn finalize_mint(state: &AppState, op: &Operation, ctx: &Value) -> Result<(), ApiError> {
    let chain = &state.chain_client;
    let rpc = |e: alloy::contract::Error| ApiError::Chain(e.to_string());
    let result = &ctx["result"];
    let metadata_cid = result["metadata_cid"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let (kind, source_ref, token, contract, batch_id, farmer_id) = if op.kind == "mint_provenance" {
        let digest = parse_b256(result["record_digest"].as_str().unwrap_or_default())
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let token = chain
            .provenance()
            .tokenOfRecord(digest)
            .call()
            .await
            .map_err(rpc)?;
        (
            "provenance",
            hex32(digest),
            token,
            chain.provenance_address,
            result["batch_id"].as_str().map(str::to_string),
            None,
        )
    } else {
        let farmer = parse_b256(result["farmer_id"].as_str().unwrap_or_default())
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let token = chain
            .credential()
            .credentialOf(farmer)
            .call()
            .await
            .map_err(rpc)?;
        (
            "credential",
            hex32(farmer),
            token,
            chain.credential_address,
            None,
            Some(hex32(farmer)),
        )
    };
    if token.is_zero() {
        return Err(ApiError::Chain(
            "confirmed mint but token not found on-chain".into(),
        ));
    }
    let owner = if kind == "provenance" {
        chain
            .provenance()
            .ownerOf(token)
            .call()
            .await
            .map_err(rpc)?
    } else {
        chain
            .credential()
            .ownerOf(token)
            .call()
            .await
            .map_err(rpc)?
    };
    let row = db::NftRow {
        kind: kind.into(),
        source_ref,
        token_id: Some(
            token
                .try_into()
                .map_err(|_| ApiError::internal("token id overflow"))?,
        ),
        contract_address: hexaddr(contract),
        chain_id: chain.chain_id,
        batch_id,
        farmer_id,
        owner: Some(hexaddr(owner)),
        token_uri: format!("ipfs://{metadata_cid}"),
        metadata_cid,
        op_id: op.id.clone(),
        status: "minted".into(),
        tx_hash: op.tx_hash.clone(),
        created_at: db::now(),
        updated_at: db::now(),
    };
    state.db.call(move |c| db::upsert_nft(c, &row)).await
}

#[derive(Debug, Serialize)]
pub struct TokenView {
    pub kind: &'static str,
    pub token_id: u64,
    pub contract_address: String,
    pub chain_id: u64,
    pub owner: String,
    pub owner_name: Option<String>,
    pub token_uri: String,
    pub metadata_cid: String,
    /// Metadata fetched from IPFS, CID-checked and consistent with the on-chain token.
    pub metadata_verified: bool,
    pub metadata_problem: Option<String>,
    pub metadata: Option<Value>,
    pub soulbound: bool,
    pub batch_id: Option<String>,
    pub current_custodian: Option<String>,
    pub owner_is_custodian: Option<bool>,
    pub source_record: Option<RecordVerification>,
    pub mint_tx_hash: Option<String>,
}

/// Fetch untrusted metadata by CID and check it is canonical JSON with the expected schema.
async fn fetch_metadata(
    state: &AppState,
    token_uri: &str,
    schema: &str,
) -> (String, Result<Value, String>) {
    let cid_str = token_uri
        .strip_prefix("ipfs://")
        .unwrap_or_default()
        .to_string();
    let result = async {
        let cid = Cid::parse(&cid_str).map_err(|e| format!("tokenURI is not ipfs://<cid>: {e}"))?;
        let bytes = state
            .ipfs_client
            .get_block(&cid)
            .await
            .map_err(|e| e.to_string())?;
        let v = canonical::parse_canonical(&bytes).map_err(|e| e.to_string())?;
        if v["properties"]["schema"].as_str() != Some(schema) {
            return Err("metadata schema mismatch".to_string());
        }
        Ok(v)
    }
    .await;
    (cid_str, result)
}

/// GET /api/nft/provenance/{token_id} (public)
pub async fn get_provenance_token(
    State(state): State<AppState>,
    Path(token_id): Path<u64>,
) -> Result<Json<TokenView>, ApiError> {
    let chain = &state.chain_client;
    let nft = chain.provenance();
    let id = U256::from(token_id);
    let rpc = |e: alloy::contract::Error| ApiError::Chain(e.to_string());
    let digest = nft.recordOfToken(id).call().await.map_err(rpc)?;
    if digest == B256::ZERO {
        return Err(ApiError::NotFound("token does not exist".into()));
    }
    let owner = nft.ownerOf(id).call().await.map_err(rpc)?;
    let token_uri = nft.tokenURI(id).call().await.map_err(rpc)?;
    let batch_id = nft.batchOfToken(id).call().await.map_err(rpc)?;
    let custodian = chain
        .registry()
        .custodianOf(batch_id)
        .call()
        .await
        .map_err(rpc)?;
    let names = names(&state).await?;

    let (metadata_cid, meta) = fetch_metadata(&state, &token_uri, PROVENANCE_SCHEMA).await;
    let (metadata, mut problem) = match meta {
        Ok(v) => (Some(v), None),
        Err(e) => (None, Some(e)),
    };
    if let Some(m) = &metadata {
        let p = &m["properties"];
        if p["source_record_digest"].as_str() != Some(hex32(digest).as_str())
            || p["batch_id"].as_str() != Some(hex32(batch_id).as_str())
            || p["chain_id"].as_u64() != Some(chain.chain_id)
        {
            problem = Some("metadata does not match the on-chain token".into());
        }
    }
    let source_record = match chain
        .anchor_log(digest)
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?
    {
        Some(log) => Some(verify::verify_log(&state, &log, &names).await),
        None => None,
    };
    let mint = state
        .db
        .call(move |c| db::nft_by_token(c, "provenance", token_id))
        .await?;
    Ok(Json(TokenView {
        kind: "provenance",
        token_id,
        contract_address: hexaddr(chain.provenance_address),
        chain_id: chain.chain_id,
        owner_name: names.get(&hexaddr(owner)).cloned(),
        owner: hexaddr(owner),
        token_uri,
        metadata_cid,
        metadata_verified: problem.is_none(),
        metadata_problem: problem,
        metadata,
        soulbound: false,
        batch_id: Some(hex32(batch_id)),
        current_custodian: Some(hexaddr(custodian)),
        owner_is_custodian: Some(owner == custodian),
        source_record,
        mint_tx_hash: mint.and_then(|m| m.tx_hash),
    }))
}

/// GET /api/nft/credential/{token_id} (public)
pub async fn get_credential_token(
    State(state): State<AppState>,
    Path(token_id): Path<u64>,
) -> Result<Json<TokenView>, ApiError> {
    let chain = &state.chain_client;
    let nft = chain.credential();
    let id = U256::from(token_id);
    let rpc = |e: alloy::contract::Error| ApiError::Chain(e.to_string());
    let farmer = nft.farmerOfToken(id).call().await.map_err(rpc)?;
    if farmer == B256::ZERO {
        return Err(ApiError::NotFound(
            "credential does not exist (or was revoked)".into(),
        ));
    }
    let owner = nft.ownerOf(id).call().await.map_err(rpc)?;
    let token_uri = nft.tokenURI(id).call().await.map_err(rpc)?;
    let locked = nft.locked(id).call().await.map_err(rpc)?;
    let (metadata_cid, meta) = fetch_metadata(&state, &token_uri, CREDENTIAL_SCHEMA).await;
    let (metadata, mut problem) = match meta {
        Ok(v) => (Some(v), None),
        Err(e) => (None, Some(e)),
    };
    if let Some(m) = &metadata {
        if m["properties"]["farmer_id"].as_str() != Some(hex32(farmer).as_str()) {
            problem = Some("metadata does not match the on-chain token".into());
        }
    }
    let mint = state
        .db
        .call(move |c| db::nft_by_token(c, "credential", token_id))
        .await?;
    Ok(Json(TokenView {
        kind: "credential",
        token_id,
        contract_address: hexaddr(chain.credential_address),
        chain_id: chain.chain_id,
        owner: hexaddr(owner),
        owner_name: None,
        token_uri,
        metadata_cid,
        metadata_verified: problem.is_none(),
        metadata_problem: problem,
        metadata,
        soulbound: locked,
        batch_id: None,
        current_custodian: None,
        owner_is_custodian: None,
        source_record: None,
        mint_tx_hash: mint.and_then(|m| m.tx_hash),
    }))
}

/// GET /api/nfts — tokens this server has seen confirmed.
pub async fn list_nfts(
    State(state): State<AppState>,
    actor: Actor,
) -> Result<Json<Vec<db::NftRow>>, ApiError> {
    if !actor.is_admin {
        actor.require_any_role()?;
    }
    let rows = state.db.call(db::list_nfts).await?;
    Ok(Json(
        rows.into_iter().filter(|r| r.status == "minted").collect(),
    ))
}

/// Used by tests and the workflow: is this op a confirmed mint?
pub fn is_confirmed(op: &Operation) -> bool {
    op.status == OpStatus::Confirmed
}
