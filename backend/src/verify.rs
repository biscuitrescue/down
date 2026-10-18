//! Provenance verification. The timeline is read from on-chain `RecordAnchored` events (not the database);
//! every record is re-fetched from IPFS, checked against its CID, decrypted if confidential, re-canonicalized,
//! re-hashed and matched against the on-chain commitment, and each batch's hash chain is recomputed to prove
//! the timeline is complete. Every failure is reported explicitly; nothing is "verified" unless all checks ran.

use crate::canonical::{self, RecordType, PRIVATE_CIPHERTEXT_FIELD, PRIVATE_COMMITMENT_FIELD};
use crate::chain::{hex32, hexaddr, AnchoredLog, STAGES};
use crate::cid::Cid;
use crate::crypto;
use crate::db;
use crate::error::ApiError;
use crate::ipfs::IpfsError;
use crate::merkle::UnitTree;
use crate::records::envelope_aad;
use crate::state::AppState;
use crate::supply_chain_handlers::parse_id;
use alloy::primitives::B256;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
    /// Content fetched, CID matched, canonical, hash matched, bound to the right subject/type/chain.
    Verified,
    DigestMismatch,
    NotAnchored,
    UnknownRecordType,
    MalformedCid,
    ContentUnavailable,
    CidMismatch,
    DecryptionFailed,
    MalformedPayload,
    HashMismatch,
    AssociationMismatch,
    /// A field declared private for this record type appears in the public part.
    PrivateFieldExposed,
    /// The decrypted private part does not match the committed `private_commitment`.
    PrivateCommitmentMismatch,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordVerification {
    pub record_digest: String,
    pub record_type: String,
    pub subject_id: String,
    pub submitter: String,
    pub submitter_name: Option<String>,
    pub anchored_at: Option<u64>,
    pub tx_hash: Option<String>,
    pub block_number: Option<u64>,
    pub cid: String,
    pub ipfs_url: String,
    pub content_hash: String,
    pub encrypted: bool,
    /// Whether the CID itself was checked against the fetched bytes (raw sha2-256 CIDs only).
    pub cid_checked: bool,
    pub status: RecordStatus,
    pub verified: bool,
    pub detail: Option<String>,
    pub created_at: Option<String>,
    /// Public `data` of the record. Private fields are never included.
    pub data: Option<Value>,
    /// Names (not values) of the encrypted private fields.
    pub private_fields: Vec<String>,
    /// `Some(true)` when the private part was decrypted and matched its commitment (server-side check).
    pub private_verified: Option<bool>,
    /// Backup pinning state of this record's block, as recorded by this server's pin queue.
    pub replicas: Vec<Replica>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Replica {
    pub target: String,
    pub status: String,
    pub verified_at: Option<String>,
}

pub async fn verify_log(
    state: &AppState,
    log: &AnchoredLog,
    names: &HashMap<String, String>,
) -> RecordVerification {
    let submitter = hexaddr(log.submitter);
    let cid_str = log.cid.clone();
    let mut v = RecordVerification {
        record_digest: hex32(log.record_digest),
        record_type: RecordType::from_u8(log.record_type)
            .map(|t| t.name().to_string())
            .unwrap_or_else(|| format!("unknown({})", log.record_type)),
        subject_id: hex32(log.subject_id),
        submitter_name: names.get(&submitter).cloned(),
        submitter,
        anchored_at: None,
        tx_hash: log.tx_hash.map(hex32),
        block_number: log.block_number,
        ipfs_url: state.ipfs_client.gateway_url(&cid_str),
        cid: cid_str,
        content_hash: hex32(log.content_hash),
        encrypted: false,
        cid_checked: false,
        status: RecordStatus::Verified,
        verified: false,
        detail: None,
        created_at: None,
        data: None,
        private_fields: vec![],
        private_verified: None,
        replicas: vec![],
    };
    let cid_key = log.cid.clone();
    if let Ok(jobs) = state.db.call(move |c| db::pins_for_cid(c, &cid_key)).await {
        v.replicas = jobs
            .into_iter()
            .map(|j| Replica {
                target: j.target,
                status: j.status,
                verified_at: j.verified_at,
            })
            .collect();
    }
    let fail = |mut v: RecordVerification, status: RecordStatus, detail: String| {
        v.status = status;
        v.detail = Some(detail);
        v.verified = false;
        v
    };

    let Some(rt) = RecordType::from_u8(log.record_type) else {
        return fail(
            v,
            RecordStatus::UnknownRecordType,
            "record type is not known to this backend".into(),
        );
    };
    let domain = state.domain();
    let expected = canonical::record_digest(
        domain.chain_id,
        domain.registry,
        rt,
        log.subject_id,
        log.content_hash,
    );
    if expected != log.record_digest {
        return fail(
            v,
            RecordStatus::DigestMismatch,
            "event digest does not match its fields".into(),
        );
    }
    match state
        .chain_client
        .registry()
        .anchors(log.record_digest)
        .call()
        .await
    {
        Ok(a)
            if a.anchoredAt != 0
                && a.subjectId == log.subject_id
                && a.recordType == log.record_type =>
        {
            v.anchored_at = Some(a.anchoredAt);
        }
        Ok(_) => {
            return fail(
                v,
                RecordStatus::NotAnchored,
                "no matching anchor in contract storage".into(),
            )
        }
        Err(e) => {
            return fail(
                v,
                RecordStatus::NotAnchored,
                format!("could not read anchor: {e}"),
            )
        }
    }

    let cid = match Cid::parse(&log.cid) {
        Ok(c) => c,
        Err(e) => return fail(v, RecordStatus::MalformedCid, e.to_string()),
    };
    v.cid_checked = cid.is_locally_verifiable();
    let bytes = match state.ipfs_client.get_block(&cid).await {
        Ok(b) => b,
        Err(IpfsError::IntegrityFailure { .. }) => {
            return fail(
                v,
                RecordStatus::CidMismatch,
                "IPFS returned bytes that do not match the CID".into(),
            )
        }
        Err(e) => return fail(v, RecordStatus::ContentUnavailable, e.to_string()),
    };

    let key = &state.config.secrets.pii_encryption_key;
    // Legacy layout: the whole record inside one envelope.
    let (committed, ciphertext) = if crypto::is_envelope(&bytes) {
        v.encrypted = true;
        match crypto::decrypt_envelope(key, &bytes) {
            Ok((pt, aad)) if aad == envelope_aad(rt, log.subject_id) => (pt, None),
            Ok(_) => {
                return fail(
                    v,
                    RecordStatus::AssociationMismatch,
                    "envelope is bound to another record".into(),
                )
            }
            Err(e) => return fail(v, RecordStatus::DecryptionFailed, e.to_string()),
        }
    } else {
        // Current layout: committed record + optional `private_ciphertext`.
        let mut stored = match canonical::parse_canonical(&bytes) {
            Ok(r) => r,
            Err(e) => return fail(v, RecordStatus::MalformedPayload, e.to_string()),
        };
        let ciphertext = stored
            .as_object_mut()
            .and_then(|o| o.remove(PRIVATE_CIPHERTEXT_FIELD));
        match canonical::canonical_json(&stored) {
            Ok(c) => (c, ciphertext),
            Err(e) => return fail(v, RecordStatus::MalformedPayload, e.to_string()),
        }
    };

    let record = match canonical::parse_canonical(&committed) {
        Ok(r) => r,
        Err(e) => return fail(v, RecordStatus::MalformedPayload, e.to_string()),
    };
    if canonical::content_hash(&committed) != log.content_hash {
        return fail(
            v,
            RecordStatus::HashMismatch,
            "keccak256(canonical record) differs from the anchored contentHash".into(),
        );
    }
    if let Err(e) = canonical::check_association(&record, domain, rt, log.subject_id) {
        return fail(v, RecordStatus::AssociationMismatch, e.to_string());
    }
    v.created_at = record
        .get("created_at")
        .and_then(Value::as_str)
        .map(str::to_string);
    let legacy = v.encrypted;
    let data = record.get("data").cloned().unwrap_or(Value::Null);
    if !legacy {
        if let Some(exposed) = rt.private_fields().iter().find(|f| data.get(**f).is_some()) {
            return fail(
                v,
                RecordStatus::PrivateFieldExposed,
                format!("private field `{exposed}` is stored in the public part"),
            );
        }
    }

    // Private part: must be present iff committed, decrypt, and match the commitment.
    match (record.get(PRIVATE_COMMITMENT_FIELD), ciphertext) {
        (None, None) => {}
        (None, Some(_)) => {
            return fail(
                v,
                RecordStatus::MalformedPayload,
                "ciphertext without a private commitment".into(),
            )
        }
        (Some(_), None) => {
            return fail(
                v,
                RecordStatus::MalformedPayload,
                "private commitment without ciphertext".into(),
            )
        }
        (Some(c), Some(env)) => {
            v.encrypted = true;
            let Some(commitment) = c.as_str().and_then(|c| c.parse::<B256>().ok()) else {
                return fail(
                    v,
                    RecordStatus::MalformedPayload,
                    "private_commitment is not 32-byte hex".into(),
                );
            };
            let env_bytes = match canonical::canonical_json(&env) {
                Ok(b) => b,
                Err(e) => return fail(v, RecordStatus::MalformedPayload, e.to_string()),
            };
            let private_bytes = match crypto::decrypt_envelope(key, &env_bytes) {
                Ok((pt, aad)) if aad == canonical::private_aad(rt, log.subject_id, commitment) => {
                    pt
                }
                Ok(_) => {
                    return fail(
                        v,
                        RecordStatus::AssociationMismatch,
                        "private part is bound to another record".into(),
                    )
                }
                Err(e) => return fail(v, RecordStatus::DecryptionFailed, e.to_string()),
            };
            if canonical::content_hash(&private_bytes) != commitment {
                v.private_verified = Some(false);
                return fail(
                    v,
                    RecordStatus::PrivateCommitmentMismatch,
                    "decrypted private part does not match private_commitment".into(),
                );
            }
            match canonical::parse_canonical(&private_bytes) {
                Ok(Value::Object(m)) => {
                    v.private_fields = m.keys().filter(|k| k.as_str() != "salt").cloned().collect();
                }
                _ => {
                    return fail(
                        v,
                        RecordStatus::MalformedPayload,
                        "private part is not a canonical object".into(),
                    )
                }
            }
            v.private_verified = Some(true);
        }
    }
    if !legacy {
        v.data = Some(data);
    }
    v.verified = true;
    v
}

#[derive(Debug, Serialize)]
pub struct BatchProvenance {
    pub batch_id: String,
    pub label: Option<String>,
    pub parent_batch_id: Option<String>,
    pub farmer_id: String,
    pub stage: &'static str,
    pub custodian: String,
    pub custodian_name: Option<String>,
    pub onchain_head: String,
    pub onchain_record_count: u16,
    pub recomputed_head: String,
    pub recomputed_record_count: u16,
    /// The event timeline reproduces the on-chain hash chain exactly (nothing missing, reordered or added).
    pub timeline_complete: bool,
    pub records: Vec<RecordVerification>,
}

#[derive(Debug, Serialize)]
pub struct ProvenanceReport {
    pub batch_id: String,
    pub chain_id: u64,
    pub registry: String,
    pub verified: bool,
    pub status: &'static str,
    pub problems: Vec<String>,
    pub farmer: Option<RecordVerification>,
    /// Root ancestor first, requested batch last.
    pub lineage: Vec<BatchProvenance>,
    pub skus: Vec<String>,
    pub verified_at: String,
}

async fn participant_names(state: &AppState) -> Result<HashMap<String, String>, ApiError> {
    let list = state.db.call(db::list_participants).await?;
    let mut names: HashMap<String, String> = list
        .into_iter()
        .map(|p| (p.address.to_lowercase(), p.name))
        .collect();
    names.insert(hexaddr(state.chain_client.admin_address()), "admin".into());
    Ok(names)
}

/// Build and verify the full provenance of a batch.
pub async fn provenance(state: &AppState, batch_id: B256) -> Result<ProvenanceReport, ApiError> {
    let reg = state.chain_client.registry();
    let names = participant_names(state).await?;
    let chain_err = |e: alloy::contract::Error| ApiError::Chain(e.to_string());

    // Walk up to the root ancestor (bounded).
    let mut chain_ids = vec![batch_id];
    let mut batches = Vec::new();
    loop {
        let id = *chain_ids.last().expect("non-empty");
        let b = reg.getBatch(id).call().await.map_err(chain_err)?;
        if b.createdAt == 0 {
            if batches.is_empty() {
                return Err(ApiError::NotFound(format!(
                    "batch {} not found on-chain",
                    hex32(id)
                )));
            }
            return Err(ApiError::Chain(format!(
                "parent batch {} missing on-chain",
                hex32(id)
            )));
        }
        let parent = b.parentBatchId;
        batches.push(b);
        if parent == B256::ZERO || chain_ids.len() >= 8 {
            break;
        }
        chain_ids.push(parent);
    }
    chain_ids.reverse();
    batches.reverse();

    let mut problems = Vec::new();
    let mut lineage = Vec::new();
    let mut parent_processing: Option<B256> = None;
    for (id, b) in chain_ids.iter().zip(batches.iter()) {
        let logs = state
            .chain_client
            .record_logs(*id)
            .await
            .map_err(|e| ApiError::Chain(e.to_string()))?;
        let (mut head, mut count) = match (b.parentBatchId != B256::ZERO, parent_processing) {
            (true, Some(proc_digest)) => (canonical::next_head(B256::ZERO, proc_digest), 1u16),
            (true, None) => {
                problems.push(format!(
                    "batch {}: parent processing record not found",
                    hex32(*id)
                ));
                (B256::ZERO, 0)
            }
            (false, _) => (B256::ZERO, 0),
        };
        let mut records = Vec::new();
        for log in &logs {
            head = canonical::next_head(head, log.record_digest);
            count = count.saturating_add(1);
            let v = verify_log(state, log, &names).await;
            if !v.verified {
                problems.push(format!(
                    "batch {} record {} ({}): {:?}{}",
                    hex32(*id),
                    v.record_digest,
                    v.record_type,
                    v.status,
                    v.detail
                        .as_deref()
                        .map(|d| format!(" — {d}"))
                        .unwrap_or_default()
                ));
            }
            records.push(v);
        }
        parent_processing = logs
            .iter()
            .find(|l| l.record_type == RecordType::Processing.as_u8())
            .map(|l| l.record_digest);
        let complete = head == b.head && count == b.recordCount;
        if !complete {
            problems.push(format!(
                "batch {}: event timeline does not reproduce the on-chain hash chain",
                hex32(*id)
            ));
        }
        let key = hex32(*id);
        let label = state
            .db
            .call(move |c| db::get_batch(c, &key))
            .await?
            .map(|r| r.label);
        let custodian = hexaddr(b.custodian);
        lineage.push(BatchProvenance {
            batch_id: hex32(*id),
            label,
            parent_batch_id: (b.parentBatchId != B256::ZERO).then(|| hex32(b.parentBatchId)),
            farmer_id: hex32(b.farmerId),
            stage: STAGES.get(b.stage as usize).copied().unwrap_or("unknown"),
            custodian_name: names.get(&custodian).cloned(),
            custodian,
            onchain_head: hex32(b.head),
            onchain_record_count: b.recordCount,
            recomputed_head: hex32(head),
            recomputed_record_count: count,
            timeline_complete: complete,
            records,
        });
    }

    // Farmer registration record (encrypted; verified with the server key, never disclosed).
    let farmer_id = batches[0].farmerId;
    let farmer_logs = state
        .chain_client
        .record_logs(farmer_id)
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    let farmer = match farmer_logs
        .iter()
        .find(|l| l.record_type == RecordType::FarmerRegistration.as_u8())
    {
        Some(log) => {
            let v = verify_log(state, log, &names).await;
            if !v.verified {
                problems.push(format!("farmer registration: {:?}", v.status));
            }
            Some(v)
        }
        None => {
            problems.push("farmer registration record not found on-chain".into());
            None
        }
    };

    let key = hex32(batch_id);
    let skus = state
        .db
        .call(move |c| {
            let mut stmt =
                c.prepare("SELECT sku_id FROM skus WHERE batch_id = ?1 ORDER BY created_at")?;
            let rows = stmt.query_map([key], |r| r.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .await?;

    let verified = problems.is_empty();
    Ok(ProvenanceReport {
        batch_id: hex32(batch_id),
        chain_id: state.chain_client.chain_id,
        registry: hexaddr(state.chain_client.registry_address),
        verified,
        status: if verified { "verified" } else { "failed" },
        problems,
        farmer,
        lineage,
        skus,
        verified_at: db::now(),
    })
}

/// GET /api/batches/{id}/provenance (public)
pub async fn batch_provenance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProvenanceReport>, ApiError> {
    let batch_id = parse_id("batch_id", &id)?;
    Ok(Json(provenance(&state, batch_id).await?))
}

/// GET /api/records/{digest}/verify (public)
pub async fn verify_record(
    State(state): State<AppState>,
    Path(digest): Path<String>,
) -> Result<Json<RecordVerification>, ApiError> {
    let digest = parse_id("record_digest", &digest)?;
    let log = state
        .chain_client
        .anchor_log(digest)
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("no RecordAnchored event for this digest".into()))?;
    let names = participant_names(&state).await?;
    Ok(Json(verify_log(&state, &log, &names).await))
}

#[derive(Debug, Deserialize)]
pub struct SkuQuery {
    pub unit_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UnitCheck {
    pub unit_id: String,
    /// `Some(true)` when `verifyUnit` on-chain accepted the proof; `None` if no proof could be built.
    pub included: Option<bool>,
    pub proof: Vec<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SkuVerification {
    pub sku_id: String,
    pub label: Option<String>,
    pub batch_id: String,
    pub farmer_id: String,
    pub units_root: String,
    pub packaged_at: u64,
    pub unit: Option<UnitCheck>,
    pub verified: bool,
    pub provenance: ProvenanceReport,
}

/// GET /api/skus/{id}/verify?unit_id=… (public; what a consumer's QR scan calls)
pub async fn verify_sku(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<SkuQuery>,
) -> Result<Json<SkuVerification>, ApiError> {
    let sku_id = parse_id("sku_id", &id)?;
    let reg = state.chain_client.registry();
    let sku = reg
        .skus(sku_id)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    if sku.packagedAt == 0 {
        return Err(ApiError::NotFound("SKU not found on-chain".into()));
    }
    let batch = reg
        .getBatch(sku.batchId)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    let key = hex32(sku_id);
    let row = state.db.call(move |c| db::get_sku(c, &key)).await?;

    let unit = match &q.unit_id {
        None => None,
        Some(unit_id) => Some(match &row {
            None => UnitCheck {
                unit_id: unit_id.clone(),
                included: None,
                proof: vec![],
                detail: Some("unit list for this SKU is not available on this server".into()),
            },
            Some(r) => match UnitTree::build(sku_id, &r.unit_ids) {
                Err(e) => UnitCheck {
                    unit_id: unit_id.clone(),
                    included: None,
                    proof: vec![],
                    detail: Some(e),
                },
                Ok(tree) if tree.root() != sku.unitsRoot => UnitCheck {
                    unit_id: unit_id.clone(),
                    included: None,
                    proof: vec![],
                    detail: Some("stored unit list does not reproduce the on-chain root".into()),
                },
                Ok(tree) => match tree.proof(sku_id, unit_id) {
                    None => UnitCheck {
                        unit_id: unit_id.clone(),
                        included: Some(false),
                        proof: vec![],
                        detail: Some("unit is not part of this SKU".into()),
                    },
                    Some(proof) => {
                        let ok = reg
                            .verifyUnit(sku_id, unit_id.clone(), proof.clone())
                            .call()
                            .await
                            .map_err(|e| ApiError::Chain(e.to_string()))?;
                        UnitCheck {
                            unit_id: unit_id.clone(),
                            included: Some(ok),
                            proof: proof.into_iter().map(hex32).collect(),
                            detail: (!ok).then(|| "on-chain proof check failed".into()),
                        }
                    }
                },
            },
        }),
    };

    let provenance = provenance(&state, sku.batchId).await?;
    let verified = provenance.verified && unit.as_ref().is_none_or(|u| u.included == Some(true));
    Ok(Json(SkuVerification {
        sku_id: hex32(sku_id),
        label: row.map(|r| r.label),
        batch_id: hex32(sku.batchId),
        farmer_id: hex32(batch.farmerId),
        units_root: hex32(sku.unitsRoot),
        packaged_at: sku.packagedAt,
        unit,
        verified,
        provenance,
    }))
}
