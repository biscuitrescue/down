//! Operation journal. Every write goes through `submit`, which is idempotent per (actor, Idempotency-Key)
//! and resumable at each step:
//!
//! ```text
//! prepared --IPFS block/put--> stored --eth_sendRawTransaction--> submitted --receipt--> confirmed | reverted
//!     \__ contract rejects at gas estimation ____________________________________________/--> failed
//! ```
//!
//! IPFS and the chain are not atomic. If the upload succeeds and the transaction fails, the op stays `stored`
//! and a retry resends only the transaction. If a confirmation is slow, the op stays `submitted`; the reconciler
//! (or GET /api/operations/{id}) keeps checking. Before (re)sending, the chain is checked for an existing anchor
//! so a lost response never produces a duplicate.

use crate::auth::{Actor, ADMIN_ACTOR_ID};
use crate::chain::{hex32, parse_b256, ChainAction, SendError};
use crate::db::{self, OpStatus, Operation};
use crate::error::ApiError;
use crate::records::PreparedRecord;
use crate::state::AppState;
use alloy::primitives::B256;
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;

const MAX_RESENDS: u32 = 5;

/// Operations currently being driven in this process (HTTP handler or reconciler). Prevents two tasks from
/// sending the same operation concurrently. Single-instance deployment is assumed (see docs/ARCHITECTURE.md).
static INFLIGHT: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);

struct InflightGuard(String);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = INFLIGHT.lock() {
            set.remove(&self.0);
        }
    }
}

fn try_claim(id: &str) -> Option<InflightGuard> {
    let mut set = INFLIGHT.lock().ok()?;
    set.insert(id.to_string())
        .then(|| InflightGuard(id.to_string()))
}

pub struct NewOperation {
    pub idem_key: String,
    pub kind: &'static str,
    pub request_hash: String,
    pub record: Option<PreparedRecord>,
    pub action: ChainAction,
    pub context: Value,
}

/// Read and validate the `Idempotency-Key` header (generated when absent: then retries are not deduplicated).
pub fn idempotency_key(headers: &HeaderMap) -> Result<String, ApiError> {
    match headers.get("idempotency-key") {
        Some(v) => {
            let key = v
                .to_str()
                .map_err(|_| ApiError::bad_request("Idempotency-Key must be ASCII"))?;
            if key.is_empty() || key.len() > 128 {
                return Err(ApiError::bad_request(
                    "Idempotency-Key must be 1-128 characters",
                ));
            }
            Ok(key.to_string())
        }
        None => Ok(db::new_id("auto")),
    }
}

/// Hash of the canonical request body, to detect an Idempotency-Key reused for a different request.
pub fn request_hash<T: Serialize>(kind: &str, req: &T) -> Result<String, ApiError> {
    let v = serde_json::to_value(req)?;
    let bytes = crate::canonical::canonical_json(&json!({"kind": kind, "body": v}))
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(hex::encode(crate::crypto::sha256(&bytes)))
}

/// If this (actor, Idempotency-Key) already has an operation, resume and return it. Call this before any
/// state-dependent validation: a retry after success must return the original result, not fail a pre-check
/// (e.g. "caller is no longer the custodian") that the first attempt itself made false.
pub async fn resume_existing(
    state: &AppState,
    actor: &Actor,
    idem_key: &str,
    request_hash: &str,
) -> Result<Option<Operation>, ApiError> {
    let (a, k) = (actor.id.clone(), idem_key.to_string());
    let Some(op) = state
        .db
        .call(move |c| db::operation_by_key(c, &a, &k))
        .await?
    else {
        return Ok(None);
    };
    if op.request_hash != request_hash {
        return Err(ApiError::Conflict(
            "Idempotency-Key was already used for a different request".into(),
        ));
    }
    Ok(Some(
        drive(state, op, state.chain_client.confirm_timeout).await?,
    ))
}

/// Create (or resume) an operation and drive it as far as possible within the confirmation timeout.
pub async fn submit(
    state: &AppState,
    actor: &Actor,
    new: NewOperation,
) -> Result<Operation, ApiError> {
    let now = db::now();
    let action = serde_json::to_string(&new.action)?;
    let op = Operation {
        id: db::new_id("op"),
        idem_key: new.idem_key,
        actor_id: actor.id.clone(),
        kind: new.kind.to_string(),
        request_hash: new.request_hash.clone(),
        status: OpStatus::Prepared,
        subject_id: new.record.as_ref().map(|r| hex32(r.subject_id)),
        record_type: new.record.as_ref().map(|r| r.record_type.as_u8()),
        content_hash: new.record.as_ref().map(|r| hex32(r.content_hash)),
        record_digest: new.record.as_ref().map(|r| hex32(r.record_digest)),
        cid: new.record.as_ref().map(|r| r.cid.to_string()),
        payload: new.record.as_ref().map(|r| r.payload.clone()),
        encrypted: new.record.as_ref().is_some_and(|r| r.encrypted),
        public_json: new.record.as_ref().and_then(|r| r.public_json.clone()),
        action,
        context: serde_json::to_string(&new.context)?,
        tx_hash: None,
        block_number: None,
        error: None,
        attempts: 0,
        submitted_at: None,
        created_at: now.clone(),
        updated_at: now,
    };
    let (stored, is_new) = state
        .db
        .call(move |c| db::insert_or_get_operation(c, &op))
        .await?;
    if !is_new && stored.request_hash != new.request_hash {
        return Err(ApiError::Conflict(
            "Idempotency-Key was already used for a different request".into(),
        ));
    }
    if !is_new {
        tracing::info!(op = %stored.id, status = stored.status.as_str(), "idempotent retry of existing operation");
    }
    drive(state, stored, state.chain_client.confirm_timeout).await
}

async fn save(state: &AppState, op: &Operation) -> Result<(), ApiError> {
    let op = op.clone();
    state.db.call(move |c| db::update_operation(c, &op)).await
}

/// Resolve the actor that owns an operation (to sign with its key).
pub async fn actor_for(state: &AppState, actor_id: &str) -> Result<Actor, ApiError> {
    if actor_id == ADMIN_ACTOR_ID {
        return Ok(Actor::admin(state));
    }
    let id = actor_id.to_string();
    let p = state
        .db
        .call(move |c| db::participant_by_id(c, &id))
        .await?
        .ok_or_else(|| ApiError::internal("operation actor no longer exists"))?;
    Actor::from_participant(&p)
}

/// Advance an operation. Returns the op in its latest state; `Err` only for retryable infrastructure failures.
pub async fn drive(state: &AppState, op: Operation, wait: Duration) -> Result<Operation, ApiError> {
    let Some(_guard) = try_claim(&op.id) else {
        // Another task is driving it; report the current state.
        return Ok(op);
    };
    // Reload under the claim so we never act on a stale status.
    let id = op.id.clone();
    let mut op = state
        .db
        .call(move |c| db::get_operation(c, &id))
        .await?
        .unwrap_or(op);
    loop {
        match op.status {
            OpStatus::Prepared => {
                if let Some(payload) = op.payload.clone() {
                    match state.ipfs_client.put_block(&payload).await {
                        Ok(cid) => {
                            if op.cid.as_deref() != Some(cid.to_string().as_str()) {
                                op.status = OpStatus::Failed;
                                op.error = Some("IPFS returned an unexpected CID".into());
                                save(state, &op).await?;
                                return Ok(op);
                            }
                            crate::pinning::enqueue(state, &cid).await;
                        }
                        Err(e) => {
                            op.attempts += 1;
                            op.error = Some(format!("ipfs: {e}"));
                            save(state, &op).await?;
                            return Err(e.into());
                        }
                    }
                }
                op.status = OpStatus::Stored;
                op.error = None;
                save(state, &op).await?;
            }
            OpStatus::Stored => {
                if let Some(tx) = already_applied(state, &op).await? {
                    tracing::info!(op = %op.id, "operation already applied on-chain; recovering");
                    op.tx_hash = tx.map(hex32);
                    op.status = OpStatus::Confirmed;
                    if let Some(tx) = tx {
                        if let Ok(Some(r)) = state.chain_client.receipt(tx).await {
                            op.block_number = r.block_number;
                        }
                    }
                    save(state, &op).await?;
                    finalize(state, &mut op).await;
                    return Ok(op);
                }
                if op.attempts >= MAX_RESENDS + 3 {
                    op.status = OpStatus::Failed;
                    op.error = Some("too many attempts".into());
                    save(state, &op).await?;
                    return Ok(op);
                }
                let action: ChainAction = serde_json::from_str(&op.action)?;
                let signer = actor_for(state, &op.actor_id).await?.signer(state)?;
                op.attempts += 1;
                match state.chain_client.send(signer, &action).await {
                    Ok(hash) => {
                        op.tx_hash = Some(hex32(hash));
                        op.submitted_at = Some(chrono::Utc::now().timestamp());
                        op.status = OpStatus::Submitted;
                        op.error = None;
                        save(state, &op).await?;
                        tracing::info!(op = %op.id, kind = %op.kind, tx = %hex32(hash), "transaction submitted");
                    }
                    Err(SendError::Rejected(reason)) | Err(SendError::Invalid(reason)) => {
                        op.status = OpStatus::Failed;
                        op.error = Some(reason);
                        save(state, &op).await?;
                        return Ok(op);
                    }
                    Err(SendError::Transport(e)) => {
                        op.error = Some(format!("rpc: {e}"));
                        save(state, &op).await?;
                        return Err(ApiError::Chain(e));
                    }
                }
            }
            OpStatus::Submitted => {
                let tx = parse_b256(op.tx_hash.as_deref().unwrap_or_default())
                    .map_err(|_| ApiError::internal("submitted operation without tx hash"))?;
                let receipt = state
                    .chain_client
                    .wait_for_receipt(tx, wait)
                    .await
                    .map_err(|e| ApiError::Chain(e.to_string()))?;
                match receipt {
                    Some(r) if r.status() => {
                        op.block_number = r.block_number;
                        if let Some(digest) = op.record_digest.as_deref() {
                            let anchored = state.chain_client.anchored_in_receipt(&r);
                            if !anchored.iter().any(|l| hex32(l.record_digest) == digest) {
                                op.status = OpStatus::Failed;
                                op.error = Some(
                                    "receipt does not contain the expected RecordAnchored event"
                                        .into(),
                                );
                                save(state, &op).await?;
                                return Ok(op);
                            }
                        }
                        op.status = OpStatus::Confirmed;
                        op.error = None;
                        save(state, &op).await?;
                        tracing::info!(op = %op.id, kind = %op.kind, block = ?op.block_number, "transaction confirmed");
                        finalize(state, &mut op).await;
                        return Ok(op);
                    }
                    Some(r) => {
                        op.block_number = r.block_number;
                        op.status = OpStatus::Reverted;
                        op.error = Some("transaction reverted on-chain".into());
                        save(state, &op).await?;
                        return Ok(op);
                    }
                    None => {
                        let age = chrono::Utc::now().timestamp() - op.submitted_at.unwrap_or(0);
                        let known = state
                            .chain_client
                            .transaction_known(tx)
                            .await
                            .map_err(|e| ApiError::Chain(e.to_string()))?;
                        if !known && age as u64 > state.chain_client.drop_after.as_secs() {
                            tracing::warn!(op = %op.id, tx = %hex32(tx), "transaction dropped; will resend");
                            op.status = OpStatus::Stored;
                            op.error = Some("previous transaction was dropped".into());
                            save(state, &op).await?;
                            continue;
                        }
                        return Ok(op);
                    }
                }
            }
            OpStatus::Confirmed | OpStatus::Reverted | OpStatus::Failed => return Ok(op),
        }
    }
}

/// Whether the operation's effect is already on-chain (crash after send, dropped-then-mined, lost response).
/// Returns the transaction hash when it can be found.
async fn already_applied(
    state: &AppState,
    op: &Operation,
) -> Result<Option<Option<B256>>, ApiError> {
    let chain = &state.chain_client;
    let rpc = |e: alloy::contract::Error| ApiError::Chain(e.to_string());
    if let Some(digest) = op.record_digest.as_deref() {
        let digest = parse_b256(digest).map_err(|e| ApiError::internal(e.to_string()))?;
        let anchor = chain.registry().anchors(digest).call().await.map_err(rpc)?;
        if anchor.anchoredAt == 0 {
            return Ok(None);
        }
        let log = chain
            .anchor_log(digest)
            .await
            .map_err(|e| ApiError::Chain(e.to_string()))?;
        return Ok(Some(log.and_then(|l| l.tx_hash)));
    }
    let action: ChainAction = serde_json::from_str(&op.action)?;
    match action {
        ChainAction::CommitAi {
            batch_id,
            commit_hash,
        } => {
            let c = chain
                .registry()
                .aiCommits(parse_b256(&batch_id).map_err(|e| ApiError::internal(e.to_string()))?)
                .call()
                .await
                .map_err(rpc)?;
            Ok((hex32(c.commitHash) == commit_hash).then_some(None))
        }
        ChainAction::MintProvenance { record_digest, .. } => {
            let token = chain
                .provenance()
                .tokenOfRecord(
                    parse_b256(&record_digest).map_err(|e| ApiError::internal(e.to_string()))?,
                )
                .call()
                .await
                .map_err(rpc)?;
            Ok((!token.is_zero()).then_some(None))
        }
        _ => Ok(None),
    }
}

/// Apply database side effects of a confirmed operation. Idempotent; failures are recorded on the op and
/// retried by the reconciler.
pub async fn finalize(state: &AppState, op: &mut Operation) {
    if let Err(e) = finalize_inner(state, op).await {
        tracing::error!(op = %op.id, error = %e, "finalize failed");
        op.error = Some(format!("finalize: {e}"));
        let _ = save(state, op).await;
    } else if op
        .error
        .as_deref()
        .is_some_and(|e| e.starts_with("finalize"))
    {
        op.error = None;
        let _ = save(state, op).await;
    }
}

async fn finalize_inner(state: &AppState, op: &Operation) -> Result<(), ApiError> {
    let ctx: Value = serde_json::from_str(&op.context)?;
    if let (Some(digest), Some(subject), Some(rt), Some(content_hash), Some(cid)) = (
        op.record_digest.clone(),
        op.subject_id.clone(),
        op.record_type,
        op.content_hash.clone(),
        op.cid.clone(),
    ) {
        let actor = actor_for(state, &op.actor_id).await?;
        let row = db::RecordRow {
            record_digest: digest,
            subject_id: subject,
            record_type: rt,
            content_hash,
            cid,
            encrypted: op.encrypted,
            public_json: op.public_json.clone(),
            submitter_id: op.actor_id.clone(),
            submitter_address: crate::chain::hexaddr(actor.address),
            tx_hash: op.tx_hash.clone(),
            block_number: op.block_number,
            op_id: op.id.clone(),
            created_at: db::now(),
        };
        state.db.call(move |c| db::upsert_record(c, &row)).await?;
    }
    match op.kind.as_str() {
        "register_farmer" => crate::supply_chain_handlers::finalize_farmer(state, op, &ctx).await,
        "procurement" => crate::supply_chain_handlers::finalize_procurement(state, op, &ctx).await,
        "processing" => crate::supply_chain_handlers::finalize_processing(state, op, &ctx).await,
        "packaging" => crate::supply_chain_handlers::finalize_packaging(state, op, &ctx).await,
        "fraud_report" => {
            crate::supply_chain_handlers::finalize_fraud_report(state, op, &ctx).await
        }
        "fraud_resolution" => {
            crate::supply_chain_handlers::finalize_fraud_resolution(state, op, &ctx).await
        }
        "ai_commit" => crate::supply_chain_handlers::finalize_ai_commit(state, op, &ctx).await,
        "ai_reveal" => crate::supply_chain_handlers::finalize_ai_reveal(state, op, &ctx).await,
        "mint_provenance" | "issue_credential" => crate::nft::finalize_mint(state, op, &ctx).await,
        _ => Ok(()),
    }
}

// ======================== API VIEW ========================

#[derive(Debug, Serialize)]
pub struct OperationView {
    pub operation_id: String,
    pub kind: String,
    pub status: OpStatus,
    /// `true` when a retry with the same Idempotency-Key can make progress.
    pub retryable: bool,
    pub tx_hash: Option<String>,
    pub block_number: Option<u64>,
    pub record_digest: Option<String>,
    pub content_hash: Option<String>,
    pub cid: Option<String>,
    pub ipfs_url: Option<String>,
    pub encrypted: bool,
    pub error: Option<String>,
    pub result: Value,
    pub created_at: String,
    pub updated_at: String,
}

pub fn view(state: &AppState, op: &Operation) -> OperationView {
    let ctx: Value = serde_json::from_str(&op.context).unwrap_or(Value::Null);
    OperationView {
        operation_id: op.id.clone(),
        kind: op.kind.clone(),
        status: op.status,
        retryable: matches!(
            op.status,
            OpStatus::Prepared | OpStatus::Stored | OpStatus::Submitted
        ),
        tx_hash: op.tx_hash.clone(),
        block_number: op.block_number,
        record_digest: op.record_digest.clone(),
        content_hash: op.content_hash.clone(),
        ipfs_url: op.cid.as_deref().map(|c| state.ipfs_client.gateway_url(c)),
        cid: op.cid.clone(),
        encrypted: op.encrypted,
        error: op.error.clone(),
        result: ctx.get("result").cloned().unwrap_or(Value::Null),
        created_at: op.created_at.clone(),
        updated_at: op.updated_at.clone(),
    }
}

/// HTTP mapping: confirmed → 200, still in flight → 202, rejected/reverted → 409 (body explains why).
pub fn respond(state: &AppState, op: &Operation) -> Response {
    let status = match op.status {
        OpStatus::Confirmed => StatusCode::OK,
        OpStatus::Prepared | OpStatus::Stored | OpStatus::Submitted => StatusCode::ACCEPTED,
        OpStatus::Failed | OpStatus::Reverted => StatusCode::CONFLICT,
    };
    (status, Json(view(state, op))).into_response()
}

/// GET /api/operations/{id} — also advances the operation (checks for a receipt).
pub async fn get_operation(
    axum::extract::State(state): axum::extract::State<AppState>,
    actor: Actor,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Response, ApiError> {
    let op = state
        .db
        .call(move |c| db::get_operation(c, &id))
        .await?
        .ok_or_else(|| ApiError::NotFound("operation not found".into()))?;
    if !actor.is_admin && op.actor_id != actor.id {
        return Err(ApiError::NotFound("operation not found".into()));
    }
    let op = if op.status == OpStatus::Submitted {
        drive(&state, op, Duration::ZERO).await?
    } else {
        op
    };
    Ok(respond(&state, &op))
}
