//! Supply Chain Workflow Orchestration
//!
//! `/api/workflow/execute` runs the full chain (farmer → FPO → warehouse → logistics → processor → packager,
//! plus the provenance NFT and an optional AI commitment) by calling the same stage services as the
//! individual endpoints, each signed by the participant who performs it.
//!
//! It is not atomic and does not pretend to be: every stage is its own operation with key
//! `wf:<workflow_key>:<stage>`. The run stops at the first stage that is not confirmed and reports per-stage
//! status; calling again with the same `workflow_key` resumes from there without repeating confirmed stages.

use crate::auth::Actor;
use crate::db::{self, OpStatus, Operation};
use crate::error::ApiError;
use crate::nft;
use crate::ops::{self, OperationView};
use crate::state::AppState;
use crate::supply_chain_handlers as sc;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowParticipants {
    pub fpo: String,
    pub warehouse: String,
    pub logistics: String,
    pub processor: String,
    /// Defaults to the processor (it must then also hold the packager role).
    #[serde(default)]
    pub packager: Option<String>,
    #[serde(default)]
    pub ai_oracle: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRequest {
    pub workflow_key: String,
    pub participants: WorkflowParticipants,
    /// Either an existing farmer id or a registration request.
    #[serde(default)]
    pub farmer_id: Option<String>,
    #[serde(default)]
    pub farmer: Option<Value>,
    /// Stage payloads use the same schema as the individual endpoints, minus the ids the workflow fills in.
    pub procurement: Value,
    pub warehouse: Value,
    #[serde(default)]
    pub logistics: Vec<Value>,
    pub processing: Value,
    pub packaging: Value,
    /// Which processing output to package (default 0).
    #[serde(default)]
    pub package_output_index: usize,
    #[serde(default = "yes")]
    pub mint_provenance_nft: bool,
    #[serde(default)]
    pub ai_assessment: Option<Value>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct StageResult {
    pub stage: String,
    pub status: String,
    pub operation: Option<OperationView>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Default)]
pub struct WorkflowIds {
    pub farmer_id: Option<String>,
    pub batch_id: Option<String>,
    pub output_batch_ids: Vec<String>,
    pub sku_id: Option<String>,
    pub provenance_record_digest: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WorkflowResponse {
    pub workflow_key: String,
    /// `completed`, `in_progress` (a stage is pending confirmation; call again to resume) or `failed`.
    pub status: &'static str,
    pub stages: Vec<StageResult>,
    pub ids: WorkflowIds,
    pub note: &'static str,
}

fn with_fields(mut v: Value, fields: &[(&str, Value)]) -> Result<Value, ApiError> {
    let obj = v
        .as_object_mut()
        .ok_or_else(|| ApiError::bad_request("stage payloads must be JSON objects"))?;
    for (k, val) in fields {
        obj.insert((*k).into(), val.clone());
    }
    Ok(v)
}

fn parse<T: serde::de::DeserializeOwned>(stage: &str, v: Value) -> Result<T, ApiError> {
    serde_json::from_value(v).map_err(|e| ApiError::BadRequest(format!("{stage}: {e}")))
}

async fn participant(state: &AppState, id: &str) -> Result<Actor, ApiError> {
    let id_owned = id.to_string();
    let p = state
        .db
        .call(move |c| db::participant_by_id(c, &id_owned))
        .await?
        .ok_or_else(|| ApiError::BadRequest(format!("participant {id} not found")))?;
    Actor::from_participant(&p)
}

fn result_str(op: &Operation, path: &[&str]) -> Option<String> {
    let ctx: Value = serde_json::from_str(&op.context).ok()?;
    let mut v = &ctx["result"];
    for p in path {
        v = v.get(p)?;
    }
    v.as_str().map(str::to_string)
}

struct Runner<'a> {
    state: &'a AppState,
    key: String,
    stages: Vec<StageResult>,
    halted: Option<&'static str>,
}

impl Runner<'_> {
    /// Record a stage outcome; returns the op only when it is confirmed.
    fn record(&mut self, stage: &str, res: Result<Operation, ApiError>) -> Option<Operation> {
        match res {
            Ok(op) => {
                let status = op.status;
                self.stages.push(StageResult {
                    stage: stage.into(),
                    status: status.as_str().into(),
                    operation: Some(ops::view(self.state, &op)),
                    error: op.error.clone(),
                });
                match status {
                    OpStatus::Confirmed => Some(op),
                    OpStatus::Failed | OpStatus::Reverted => {
                        self.halted = Some("failed");
                        None
                    }
                    _ => {
                        self.halted = Some("in_progress");
                        None
                    }
                }
            }
            Err(e) => {
                self.stages.push(StageResult {
                    stage: stage.into(),
                    status: "error".into(),
                    operation: None,
                    error: Some(e.to_string()),
                });
                // Infrastructure errors are retryable; validation errors are not.
                self.halted = Some(
                    if matches!(
                        e,
                        ApiError::Ipfs(_) | ApiError::Chain(_) | ApiError::Unavailable(_)
                    ) {
                        "in_progress"
                    } else {
                        "failed"
                    },
                );
                None
            }
        }
    }

    fn idem(&self, stage: &str) -> String {
        format!("wf:{}:{stage}", self.key)
    }
}

/// POST /api/workflow/execute (admin)
pub async fn execute_workflow(
    State(state): State<AppState>,
    actor: Actor,
    Json(req): Json<WorkflowRequest>,
) -> Result<Response, ApiError> {
    actor.require_admin()?;
    let key = req.workflow_key.trim().to_string();
    if key.is_empty()
        || key.len() > 64
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::bad_request(
            "workflow_key must be 1-64 characters [A-Za-z0-9_-]",
        ));
    }
    let p = &req.participants;
    let fpo = participant(&state, &p.fpo).await?;
    let warehouse = participant(&state, &p.warehouse).await?;
    let logistics = participant(&state, &p.logistics).await?;
    let processor = participant(&state, &p.processor).await?;
    let packager = match &p.packager {
        Some(id) => participant(&state, id).await?,
        None => processor.clone(),
    };

    let mut r = Runner {
        state: &state,
        key,
        stages: Vec::new(),
        halted: None,
    };
    let mut ids = WorkflowIds::default();
    let response = |r: Runner, ids: WorkflowIds| -> Response {
        let status = r.halted.unwrap_or("completed");
        let code = match status {
            "completed" => StatusCode::OK,
            "in_progress" => StatusCode::ACCEPTED,
            _ => StatusCode::CONFLICT,
        };
        (
            code,
            Json(WorkflowResponse {
                workflow_key: r.key,
                status,
                stages: r.stages,
                ids,
                note: "Stages are separate operations; this workflow is not atomic. Re-run with the same workflow_key to resume.",
            }),
        )
            .into_response()
    };

    // 1. Farmer
    let farmer_id = match (&req.farmer_id, &req.farmer) {
        (Some(id), None) => id.clone(),
        (None, Some(f)) => {
            let reg = parse::<sc::RegisterFarmerRequest>("farmer", f.clone())?;
            let res = sc::register_farmer_op(&state, &fpo, r.idem("farmer"), reg).await;
            match r.record("farmer_registration", res) {
                Some(op) => result_str(&op, &["farmer_id"])
                    .ok_or_else(|| ApiError::internal("farmer id"))?,
                None => return Ok(response(r, ids)),
            }
        }
        _ => {
            return Err(ApiError::bad_request(
                "provide exactly one of farmer_id or farmer",
            ))
        }
    };
    ids.farmer_id = Some(farmer_id.clone());

    // 2. Procurement (creates the batch)
    let proc_req = parse(
        "procurement",
        with_fields(req.procurement.clone(), &[("farmer_id", json!(farmer_id))])?,
    )?;
    let Some(op) = r.record(
        "procurement",
        sc::fpo_purchase_op(&state, &fpo, r.idem("procurement"), proc_req).await,
    ) else {
        return Ok(response(r, ids));
    };
    let batch_id = result_str(&op, &["batch_id"]).ok_or_else(|| ApiError::internal("batch id"))?;
    ids.batch_id = Some(batch_id.clone());
    ids.provenance_record_digest = op.record_digest.clone();

    // 3. Provenance NFT for the procurement (minted to the FPO, follows custody afterwards)
    if req.mint_provenance_nft {
        let mint = nft::MintFromTxRequest {
            tx_hash: None,
            record_digest: op.record_digest.clone(),
        };
        if r.record(
            "provenance_nft",
            nft::mint_from_tx_op(&state, &fpo, mint).await,
        )
        .is_none()
        {
            return Ok(response(r, ids));
        }
    }

    let transfer = |to: &Actor| sc::CustodyTransferRequest {
        batch_id: batch_id.clone(),
        to_participant_id: to.id.clone(),
        quantity_kg: None,
        handover_at: None,
        notes: None,
    };

    // 4. Warehouse
    if r.record(
        "custody_to_warehouse",
        sc::transfer_custody_op(
            &state,
            &fpo,
            r.idem("custody-warehouse"),
            transfer(&warehouse),
        )
        .await,
    )
    .is_none()
    {
        return Ok(response(r, ids));
    }
    let wh = parse(
        "warehouse",
        with_fields(req.warehouse.clone(), &[("batch_id", json!(batch_id))])?,
    )?;
    if r.record(
        "warehouse_inspection",
        sc::warehouse_inspection_op(&state, &warehouse, r.idem("warehouse"), wh).await,
    )
    .is_none()
    {
        return Ok(response(r, ids));
    }

    // 5. Logistics
    if r.record(
        "custody_to_logistics",
        sc::transfer_custody_op(
            &state,
            &warehouse,
            r.idem("custody-logistics"),
            transfer(&logistics),
        )
        .await,
    )
    .is_none()
    {
        return Ok(response(r, ids));
    }
    for (i, l) in req.logistics.iter().enumerate() {
        let lr = parse(
            "logistics",
            with_fields(l.clone(), &[("batch_id", json!(batch_id))])?,
        )?;
        let stage = format!("logistics_{i}");
        if r.record(
            &stage,
            sc::record_logistics_op(&state, &logistics, r.idem(&stage), lr).await,
        )
        .is_none()
        {
            return Ok(response(r, ids));
        }
    }

    // 6. Processing
    if r.record(
        "custody_to_processor",
        sc::transfer_custody_op(
            &state,
            &logistics,
            r.idem("custody-processor"),
            transfer(&processor),
        )
        .await,
    )
    .is_none()
    {
        return Ok(response(r, ids));
    }
    let pr = parse(
        "processing",
        with_fields(
            req.processing.clone(),
            &[("input_batch_id", json!(batch_id))],
        )?,
    )?;
    let Some(op) = r.record(
        "processing",
        sc::process_batch_op(&state, &processor, r.idem("processing"), pr).await,
    ) else {
        return Ok(response(r, ids));
    };
    let ctx: Value = serde_json::from_str(&op.context)?;
    ids.output_batch_ids = ctx["result"]["outputs"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|o| o["batch_id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let output = ids
        .output_batch_ids
        .get(req.package_output_index)
        .cloned()
        .ok_or_else(|| ApiError::bad_request("package_output_index is out of range"))?;

    // 7. Packaging
    if packager.id != processor.id {
        let t = sc::CustodyTransferRequest {
            batch_id: output.clone(),
            to_participant_id: packager.id.clone(),
            quantity_kg: None,
            handover_at: None,
            notes: None,
        };
        if r.record(
            "custody_to_packager",
            sc::transfer_custody_op(&state, &processor, r.idem("custody-packager"), t).await,
        )
        .is_none()
        {
            return Ok(response(r, ids));
        }
    }
    let pk = parse(
        "packaging",
        with_fields(req.packaging.clone(), &[("batch_id", json!(output))])?,
    )?;
    let Some(op) = r.record(
        "packaging",
        sc::create_sku_op(&state, &packager, r.idem("packaging"), pk).await,
    ) else {
        return Ok(response(r, ids));
    };
    ids.sku_id = result_str(&op, &["sku_id"]);

    // 8. Optional AI assessment commitment (revealed later by the reconciler once the window opens)
    if let (Some(a), Some(oracle_id)) = (&req.ai_assessment, &p.ai_oracle) {
        let oracle = participant(&state, oracle_id).await?;
        let ar = parse(
            "ai_assessment",
            json!({"batch_id": batch_id, "assessment": a}),
        )?;
        r.record(
            "ai_commit",
            sc::commit_ai_score_op(&state, &oracle, r.idem("ai-commit"), ar).await,
        );
    }

    Ok(response(r, ids))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifySkuBody {
    pub sku_id: String,
    #[serde(default)]
    pub unit_id: Option<String>,
}

/// POST /api/workflow/verify-sku — kept for existing clients; same as GET /api/skus/{id}/verify.
pub async fn verify_sku_handler(
    State(state): State<AppState>,
    Json(req): Json<VerifySkuBody>,
) -> Result<Json<crate::verify::SkuVerification>, ApiError> {
    crate::verify::verify_sku(
        State(state),
        Path(req.sku_id),
        axum::extract::Query(crate::verify::SkuQuery {
            unit_id: req.unit_id,
        }),
    )
    .await
}
