//! Supply-chain stage endpoints. Each one validates its input, builds a typed record, and submits it through
//! the operation journal (`ops::submit`): IPFS first, then the contract call signed by the caller's own key.
//! Responses carry the operation status; nothing is reported as done before the transaction is confirmed.

use crate::auth::Actor;
use crate::canonical::{self, RecordType};
use crate::chain::{generate_id, hex32, hexaddr, roles, ChainAction, STAGES};
use crate::cid::Cid;
use crate::crypto;
use crate::db::{self, Operation};
use crate::error::ApiError;
use crate::merkle::UnitTree;
use crate::ops::{self, NewOperation};
use crate::records;
use crate::state::AppState;
use alloy::primitives::{Address, B256};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

// ======================== VALIDATION HELPERS ========================

pub const OILSEED_CROPS: [&str; 9] = [
    "groundnut",
    "soybean",
    "rapeseed_mustard",
    "sunflower",
    "sesame",
    "safflower",
    "niger",
    "castor",
    "linseed",
];

pub fn parse_id(field: &str, s: &str) -> Result<B256, ApiError> {
    let ok = s.len() == 66 && s.starts_with("0x") && s[2..].chars().all(|c| c.is_ascii_hexdigit());
    if !ok {
        return Err(ApiError::BadRequest(format!(
            "{field} must be a 0x-prefixed 32-byte hex id"
        )));
    }
    let id: B256 = s
        .parse()
        .map_err(|_| ApiError::BadRequest(format!("{field} is invalid")))?;
    if id == B256::ZERO {
        return Err(ApiError::BadRequest(format!("{field} must not be zero")));
    }
    Ok(id)
}

fn text(field: &str, s: &str, max: usize) -> Result<String, ApiError> {
    let t = s.trim();
    if t.is_empty() || t.chars().count() > max {
        return Err(ApiError::BadRequest(format!(
            "{field} must be 1-{max} characters"
        )));
    }
    if t.chars().any(|c| c.is_control()) {
        return Err(ApiError::BadRequest(format!(
            "{field} contains control characters"
        )));
    }
    Ok(t.to_string())
}

fn opt_text(field: &str, s: &Option<String>, max: usize) -> Result<Option<String>, ApiError> {
    s.as_deref().map(|v| text(field, v, max)).transpose()
}

fn range(field: &str, v: f64, min: f64, max: f64) -> Result<f64, ApiError> {
    if !v.is_finite() || v < min || v > max {
        return Err(ApiError::BadRequest(format!(
            "{field} must be between {min} and {max}"
        )));
    }
    Ok(v)
}

fn opt_range(field: &str, v: Option<f64>, min: f64, max: f64) -> Result<Option<f64>, ApiError> {
    v.map(|x| range(field, x, min, max)).transpose()
}

fn date(field: &str, s: &str) -> Result<chrono::NaiveDate, ApiError> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| ApiError::BadRequest(format!("{field} must be a YYYY-MM-DD date")))
}

fn datetime(field: &str, s: &str) -> Result<String, ApiError> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| {
            d.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .map_err(|_| ApiError::BadRequest(format!("{field} must be an RFC 3339 timestamp")))
}

fn crop(field: &str, s: &str) -> Result<String, ApiError> {
    let c = s.trim().to_lowercase();
    if !OILSEED_CROPS.contains(&c.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "{field} must be one of {OILSEED_CROPS:?}"
        )));
    }
    Ok(c)
}

fn one_of(field: &str, s: &str, allowed: &[&str]) -> Result<String, ApiError> {
    if !allowed.contains(&s) {
        return Err(ApiError::BadRequest(format!(
            "{field} must be one of {allowed:?}"
        )));
    }
    Ok(s.to_string())
}

fn cid_ref(field: &str, s: &str) -> Result<String, ApiError> {
    Cid::parse(s).map_err(|e| ApiError::BadRequest(format!("{field}: {e}")))?;
    Ok(s.to_string())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    pub cid: String,
    pub media_type: String,
    #[serde(default)]
    pub description: Option<String>,
}

fn attachments(list: &[Attachment]) -> Result<Value, ApiError> {
    if list.len() > 20 {
        return Err(ApiError::bad_request("at most 20 attachments"));
    }
    let mut out = Vec::new();
    for (i, a) in list.iter().enumerate() {
        let mut m = Map::new();
        m.insert(
            "cid".into(),
            cid_ref(&format!("attachments[{i}].cid"), &a.cid)?.into(),
        );
        m.insert(
            "media_type".into(),
            text("media_type", &a.media_type, 100)?.into(),
        );
        if let Some(d) = opt_text("description", &a.description, 300)? {
            m.insert("description".into(), d.into());
        }
        out.push(Value::Object(m));
    }
    Ok(Value::Array(out))
}

/// Insert `key: value` only when present, so optional fields are omitted rather than null.
fn put<T: Into<Value>>(m: &mut Map<String, Value>, key: &str, v: Option<T>) {
    if let Some(v) = v {
        m.insert(key.into(), v.into());
    }
}

/// Batch must exist on-chain and be held by the caller (the contract enforces this too; this gives a clear error).
async fn require_custody(
    state: &AppState,
    actor: &Actor,
    batch_id: B256,
) -> Result<crate::chain::OilseedValueChain::Batch, ApiError> {
    let b = state
        .chain_client
        .registry()
        .getBatch(batch_id)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    if b.createdAt == 0 {
        return Err(ApiError::NotFound(format!(
            "batch {} not found on-chain",
            hex32(batch_id)
        )));
    }
    if b.custodian != actor.address {
        return Err(ApiError::Conflict(format!(
            "caller is not the custodian of batch {} (custodian is {})",
            hex32(batch_id),
            hexaddr(b.custodian)
        )));
    }
    if STAGES.get(b.stage as usize) == Some(&"consumed") {
        return Err(ApiError::Conflict(
            "batch was consumed by processing".into(),
        ));
    }
    Ok(b)
}

#[allow(clippy::too_many_arguments)]
async fn run(
    state: &AppState,
    actor: &Actor,
    idem_key: String,
    kind: &'static str,
    request_hash: String,
    record: Option<records::PreparedRecord>,
    action: ChainAction,
    context: Value,
) -> Result<Operation, ApiError> {
    ops::submit(
        state,
        actor,
        NewOperation {
            idem_key,
            kind,
            request_hash,
            record,
            action,
            context,
        },
    )
    .await
}

fn ctx_str(ctx: &Value, path: &[&str]) -> Option<String> {
    let mut v = ctx;
    for p in path {
        v = v.get(p)?;
    }
    v.as_str().map(str::to_string)
}

// ======================== STAGE 1: FARMER REGISTRATION ========================

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub state: String,
    pub district: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FarmerProfile {
    pub name: String,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub village: Option<String>,
    #[serde(default)]
    pub land_area_ha: Option<f64>,
    /// Agri-Stack / state registry farmer ID, if available (kept encrypted).
    #[serde(default)]
    pub external_farmer_id: Option<String>,
    #[serde(default)]
    pub crops: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterFarmerRequest {
    /// Pseudonymous display label (e.g. FPO member number). Not the farmer's name.
    pub label: String,
    #[serde(default)]
    pub region: Option<Region>,
    #[serde(default)]
    pub primary_crop: Option<String>,
    /// Farmer-controlled wallet, only needed for the soulbound credential.
    #[serde(default)]
    pub wallet_address: Option<String>,
    pub profile: FarmerProfile,
}

/// POST /api/farmer/register (FPO). The whole registration record is encrypted before upload.
pub async fn register_farmer(
    State(state): State<AppState>,
    actor: Actor,
    headers: HeaderMap,
    Json(req): Json<RegisterFarmerRequest>,
) -> Result<Response, ApiError> {
    let op = register_farmer_op(&state, &actor, ops::idempotency_key(&headers)?, req).await?;
    Ok(ops::respond(&state, &op))
}

pub async fn register_farmer_op(
    state: &AppState,
    actor: &Actor,
    idem_key: String,
    req: RegisterFarmerRequest,
) -> Result<Operation, ApiError> {
    actor.require_role(roles::FPO, "farmer registration")?;
    let request_hash = ops::request_hash("register_farmer", &req)?;
    if let Some(op) = ops::resume_existing(state, actor, &idem_key, &request_hash).await? {
        return Ok(op);
    }
    let label = text("label", &req.label, 64)?;
    let region = req
        .region
        .as_ref()
        .map(|r| -> Result<Value, ApiError> {
            Ok(json!({"state": text("region.state", &r.state, 64)?, "district": text("region.district", &r.district, 64)?}))
        })
        .transpose()?;
    let primary_crop = req
        .primary_crop
        .as_deref()
        .map(|c| crop("primary_crop", c))
        .transpose()?;
    let wallet = req
        .wallet_address
        .as_deref()
        .map(|w| {
            w.parse::<Address>()
                .ok()
                .filter(|a| *a != Address::ZERO)
                .map(hexaddr)
                .ok_or_else(|| ApiError::bad_request("wallet_address is not a valid address"))
        })
        .transpose()?;
    let p = &req.profile;
    let mut profile = Map::new();
    profile.insert("name".into(), text("profile.name", &p.name, 120)?.into());
    put(
        &mut profile,
        "phone",
        opt_text("profile.phone", &p.phone, 20)?,
    );
    put(
        &mut profile,
        "village",
        opt_text("profile.village", &p.village, 120)?,
    );
    put(
        &mut profile,
        "land_area_ha",
        opt_range("profile.land_area_ha", p.land_area_ha, 0.0, 10_000.0)?,
    );
    put(
        &mut profile,
        "external_farmer_id",
        opt_text("profile.external_farmer_id", &p.external_farmer_id, 64)?,
    );
    let crops = p
        .crops
        .iter()
        .map(|c| crop("profile.crops", c))
        .collect::<Result<Vec<_>, _>>()?;

    let farmer_id = generate_id();
    let mut data = Map::new();
    data.insert("label".into(), label.clone().into());
    put(&mut data, "region", region.clone());
    put(&mut data, "primary_crop", primary_crop.clone());
    put(&mut data, "wallet_address", wallet.clone());
    // Public: label, region, crops. Private (declared in RecordType::private_fields): profile, wallet_address.
    data.insert("crops".into(), json!(crops));
    data.insert("profile".into(), Value::Object(profile));

    let record = records::prepare(
        state,
        RecordType::FarmerRegistration,
        farmer_id,
        actor.address,
        Value::Object(data),
    )?;
    let action = ChainAction::RegisterFarmer {
        farmer_id: hex32(farmer_id),
        content_hash: hex32(record.content_hash),
        cid: record.cid.to_string(),
    };
    let region_text = region.map(|r| {
        format!(
            "{}, {}",
            r["district"].as_str().unwrap_or(""),
            r["state"].as_str().unwrap_or("")
        )
    });
    let context = json!({
        "result": {"farmer_id": hex32(farmer_id)},
        "label": label, "region": region_text, "primary_crop": primary_crop, "wallet_address": wallet,
    });
    run(
        state,
        actor,
        idem_key,
        "register_farmer",
        request_hash,
        Some(record),
        action,
        context,
    )
    .await
}

pub async fn finalize_farmer(
    state: &AppState,
    op: &Operation,
    ctx: &Value,
) -> Result<(), ApiError> {
    let row = db::FarmerRow {
        farmer_id: ctx_str(ctx, &["result", "farmer_id"])
            .ok_or_else(|| ApiError::internal("ctx.farmer_id"))?,
        registered_by: op.actor_id.clone(),
        label: ctx_str(ctx, &["label"]).unwrap_or_default(),
        region: ctx_str(ctx, &["region"]),
        primary_crop: ctx_str(ctx, &["primary_crop"]),
        wallet_address: ctx_str(ctx, &["wallet_address"]),
        registration_digest: op.record_digest.clone().unwrap_or_default(),
        cid: op.cid.clone().unwrap_or_default(),
        op_id: op.id.clone(),
        created_at: db::now(),
    };
    state.db.call(move |c| db::upsert_farmer(c, &row)).await
}

// ======================== STAGE 2: FPO PROCUREMENT ========================

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcurementRequest {
    pub farmer_id: String,
    /// FPO's own lot label.
    pub label: String,
    pub crop: String,
    #[serde(default)]
    pub variety: Option<String>,
    #[serde(default)]
    pub season: Option<String>,
    pub delivered_quantity_kg: f64,
    /// Quantity the farmer agreed to supply, if there was a prior commitment (used for fulfilment rate).
    #[serde(default)]
    pub committed_quantity_kg: Option<f64>,
    #[serde(default)]
    pub promised_delivery_date: Option<String>,
    pub delivery_date: String,
    #[serde(default)]
    pub price_per_quintal_inr: Option<f64>,
    #[serde(default)]
    pub moisture_pct: Option<f64>,
    #[serde(default)]
    pub payment_reference: Option<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

/// POST /api/fpo/purchase (FPO) — records the farmer's sale and creates the batch.
pub async fn fpo_purchase(
    State(state): State<AppState>,
    actor: Actor,
    headers: HeaderMap,
    Json(req): Json<ProcurementRequest>,
) -> Result<Response, ApiError> {
    let op = fpo_purchase_op(&state, &actor, ops::idempotency_key(&headers)?, req).await?;
    Ok(ops::respond(&state, &op))
}

pub async fn fpo_purchase_op(
    state: &AppState,
    actor: &Actor,
    idem_key: String,
    req: ProcurementRequest,
) -> Result<Operation, ApiError> {
    actor.require_role(roles::FPO, "procurement")?;
    let request_hash = ops::request_hash("procurement", &req)?;
    if let Some(op) = ops::resume_existing(state, actor, &idem_key, &request_hash).await? {
        return Ok(op);
    }
    let farmer_id = parse_id("farmer_id", &req.farmer_id)?;
    let farmer = state
        .chain_client
        .registry()
        .farmers(farmer_id)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    if farmer.registeredAt == 0 {
        return Err(ApiError::NotFound(
            "farmer is not registered on-chain".into(),
        ));
    }
    let label = text("label", &req.label, 64)?;
    let crop_name = crop("crop", &req.crop)?;
    let delivered = range(
        "delivered_quantity_kg",
        req.delivered_quantity_kg,
        0.001,
        10_000_000.0,
    )?;
    let committed = opt_range(
        "committed_quantity_kg",
        req.committed_quantity_kg,
        0.001,
        10_000_000.0,
    )?;
    let delivery = date("delivery_date", &req.delivery_date)?;
    if delivery > chrono::Utc::now().date_naive() + chrono::Days::new(1) {
        return Err(ApiError::bad_request("delivery_date is in the future"));
    }
    let promised = req
        .promised_delivery_date
        .as_deref()
        .map(|d| date("promised_delivery_date", d))
        .transpose()?;

    let mut data = Map::new();
    data.insert("farmer_id".into(), hex32(farmer_id).into());
    data.insert("label".into(), label.clone().into());
    data.insert("crop".into(), crop_name.clone().into());
    put(&mut data, "variety", opt_text("variety", &req.variety, 64)?);
    put(
        &mut data,
        "season",
        req.season
            .as_deref()
            .map(|s| one_of("season", s, &["kharif", "rabi", "zaid"]))
            .transpose()?,
    );
    data.insert("delivered_quantity_kg".into(), json!(delivered));
    put(&mut data, "committed_quantity_kg", committed);
    put(
        &mut data,
        "promised_delivery_date",
        promised.map(|d| d.to_string()),
    );
    data.insert("delivery_date".into(), delivery.to_string().into());
    put(
        &mut data,
        "price_per_quintal_inr",
        opt_range(
            "price_per_quintal_inr",
            req.price_per_quintal_inr,
            0.0,
            1_000_000.0,
        )?,
    );
    put(
        &mut data,
        "moisture_pct",
        opt_range("moisture_pct", req.moisture_pct, 0.0, 100.0)?,
    );
    put(
        &mut data,
        "payment_reference",
        opt_text("payment_reference", &req.payment_reference, 64)?,
    );
    data.insert("attachments".into(), attachments(&req.attachments)?);

    let batch_id = generate_id();
    let record = records::prepare(
        state,
        RecordType::Procurement,
        batch_id,
        actor.address,
        Value::Object(data),
    )?;
    let action = ChainAction::RecordProcurement {
        batch_id: hex32(batch_id),
        farmer_id: hex32(farmer_id),
        content_hash: hex32(record.content_hash),
        cid: record.cid.to_string(),
    };
    let context = json!({
        "result": {"batch_id": hex32(batch_id), "farmer_id": hex32(farmer_id)},
        "label": label, "crop": crop_name,
    });
    run(
        state,
        actor,
        idem_key,
        "procurement",
        request_hash,
        Some(record),
        action,
        context,
    )
    .await
}

pub async fn finalize_procurement(
    state: &AppState,
    op: &Operation,
    ctx: &Value,
) -> Result<(), ApiError> {
    let row = db::BatchRow {
        batch_id: ctx_str(ctx, &["result", "batch_id"])
            .ok_or_else(|| ApiError::internal("ctx.batch_id"))?,
        farmer_id: ctx_str(ctx, &["result", "farmer_id"])
            .ok_or_else(|| ApiError::internal("ctx.farmer_id"))?,
        parent_batch_id: None,
        label: ctx_str(ctx, &["label"]).unwrap_or_default(),
        crop: ctx_str(ctx, &["crop"]),
        op_id: op.id.clone(),
        created_at: db::now(),
    };
    state.db.call(move |c| db::upsert_batch(c, &row)).await
}

// ======================== CUSTODY TRANSFER ========================

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CustodyTransferRequest {
    pub batch_id: String,
    pub to_participant_id: String,
    #[serde(default)]
    pub quantity_kg: Option<f64>,
    #[serde(default)]
    pub handover_at: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// POST /api/custody/transfer (current custodian)
pub async fn transfer_custody(
    State(state): State<AppState>,
    actor: Actor,
    headers: HeaderMap,
    Json(req): Json<CustodyTransferRequest>,
) -> Result<Response, ApiError> {
    let op = transfer_custody_op(&state, &actor, ops::idempotency_key(&headers)?, req).await?;
    Ok(ops::respond(&state, &op))
}

pub async fn transfer_custody_op(
    state: &AppState,
    actor: &Actor,
    idem_key: String,
    req: CustodyTransferRequest,
) -> Result<Operation, ApiError> {
    actor.require_role(roles::CUSTODIAN, "custody transfer")?;
    let request_hash = ops::request_hash("custody_transfer", &req)?;
    if let Some(op) = ops::resume_existing(state, actor, &idem_key, &request_hash).await? {
        return Ok(op);
    }
    let batch_id = parse_id("batch_id", &req.batch_id)?;
    require_custody(state, actor, batch_id).await?;
    let to_id = req.to_participant_id.clone();
    let to = state
        .db
        .call(move |c| db::participant_by_id(c, &to_id))
        .await?
        .ok_or_else(|| ApiError::NotFound("recipient participant not found".into()))?;
    if to.roles & roles::CUSTODIAN == 0 {
        return Err(ApiError::bad_request("recipient has no custodian role"));
    }
    if to.address.eq_ignore_ascii_case(&hexaddr(actor.address)) {
        return Err(ApiError::bad_request("recipient is already the custodian"));
    }
    let mut data = Map::new();
    data.insert("from".into(), hexaddr(actor.address).into());
    data.insert("to".into(), to.address.to_lowercase().into());
    data.insert("to_participant".into(), to.name.clone().into());
    put(
        &mut data,
        "quantity_kg",
        opt_range("quantity_kg", req.quantity_kg, 0.001, 10_000_000.0)?,
    );
    put(
        &mut data,
        "handover_at",
        req.handover_at
            .as_deref()
            .map(|d| datetime("handover_at", d))
            .transpose()?,
    );
    put(&mut data, "notes", opt_text("notes", &req.notes, 500)?);

    let record = records::prepare(
        state,
        RecordType::CustodyTransfer,
        batch_id,
        actor.address,
        Value::Object(data),
    )?;
    let action = ChainAction::TransferCustody {
        batch_id: hex32(batch_id),
        to: to.address.clone(),
        content_hash: hex32(record.content_hash),
        cid: record.cid.to_string(),
    };
    let context = json!({"result": {"batch_id": hex32(batch_id), "to": to.address, "to_participant_id": to.id}});
    run(
        state,
        actor,
        idem_key,
        "custody_transfer",
        request_hash,
        Some(record),
        action,
        context,
    )
    .await
}

// ======================== STAGE 3: WAREHOUSE INSPECTION ========================

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Inspection {
    pub passed: bool,
    #[serde(default)]
    pub grade: Option<String>,
    #[serde(default)]
    pub moisture_pct: Option<f64>,
    #[serde(default)]
    pub foreign_matter_pct: Option<f64>,
    #[serde(default)]
    pub oil_content_pct: Option<f64>,
    #[serde(default)]
    pub aflatoxin_ppb: Option<f64>,
    pub inspected_at: String,
    #[serde(default)]
    pub inspector_ref: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConditions {
    #[serde(default)]
    pub temperature_c: Option<f64>,
    #[serde(default)]
    pub humidity_pct: Option<f64>,
    /// CID of the IoT sensor roll-up (uploaded via /api/ipfs/upload).
    #[serde(default)]
    pub iot_rollup_cid: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WarehouseInspectionRequest {
    pub batch_id: String,
    pub warehouse_ref: String,
    pub inspection: Inspection,
    #[serde(default)]
    pub storage: Option<StorageConditions>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

/// POST /api/warehouse/inspection (WAREHOUSE custodian)
pub async fn warehouse_inspection(
    State(state): State<AppState>,
    actor: Actor,
    headers: HeaderMap,
    Json(req): Json<WarehouseInspectionRequest>,
) -> Result<Response, ApiError> {
    let op = warehouse_inspection_op(&state, &actor, ops::idempotency_key(&headers)?, req).await?;
    Ok(ops::respond(&state, &op))
}

pub async fn warehouse_inspection_op(
    state: &AppState,
    actor: &Actor,
    idem_key: String,
    req: WarehouseInspectionRequest,
) -> Result<Operation, ApiError> {
    actor.require_role(roles::WAREHOUSE, "warehouse inspection")?;
    let request_hash = ops::request_hash("warehouse_inspection", &req)?;
    if let Some(op) = ops::resume_existing(state, actor, &idem_key, &request_hash).await? {
        return Ok(op);
    }
    let batch_id = parse_id("batch_id", &req.batch_id)?;
    require_custody(state, actor, batch_id).await?;
    let i = &req.inspection;
    let mut inspection = Map::new();
    inspection.insert("passed".into(), i.passed.into());
    put(
        &mut inspection,
        "grade",
        opt_text("inspection.grade", &i.grade, 32)?,
    );
    put(
        &mut inspection,
        "moisture_pct",
        opt_range("inspection.moisture_pct", i.moisture_pct, 0.0, 100.0)?,
    );
    put(
        &mut inspection,
        "foreign_matter_pct",
        opt_range(
            "inspection.foreign_matter_pct",
            i.foreign_matter_pct,
            0.0,
            100.0,
        )?,
    );
    put(
        &mut inspection,
        "oil_content_pct",
        opt_range("inspection.oil_content_pct", i.oil_content_pct, 0.0, 100.0)?,
    );
    put(
        &mut inspection,
        "aflatoxin_ppb",
        opt_range("inspection.aflatoxin_ppb", i.aflatoxin_ppb, 0.0, 100_000.0)?,
    );
    inspection.insert(
        "inspected_at".into(),
        datetime("inspection.inspected_at", &i.inspected_at)?.into(),
    );
    put(
        &mut inspection,
        "inspector_ref",
        opt_text("inspection.inspector_ref", &i.inspector_ref, 64)?,
    );

    let mut data = Map::new();
    data.insert(
        "warehouse_ref".into(),
        text("warehouse_ref", &req.warehouse_ref, 64)?.into(),
    );
    data.insert("inspection".into(), Value::Object(inspection));
    if let Some(s) = &req.storage {
        let mut storage = Map::new();
        put(
            &mut storage,
            "temperature_c",
            opt_range("storage.temperature_c", s.temperature_c, -40.0, 80.0)?,
        );
        put(
            &mut storage,
            "humidity_pct",
            opt_range("storage.humidity_pct", s.humidity_pct, 0.0, 100.0)?,
        );
        put(
            &mut storage,
            "iot_rollup_cid",
            s.iot_rollup_cid
                .as_deref()
                .map(|c| cid_ref("storage.iot_rollup_cid", c))
                .transpose()?,
        );
        data.insert("storage".into(), Value::Object(storage));
    }
    data.insert("attachments".into(), attachments(&req.attachments)?);

    let record = records::prepare(
        state,
        RecordType::WarehouseInspection,
        batch_id,
        actor.address,
        Value::Object(data),
    )?;
    let action = ChainAction::RecordWarehouseInspection {
        batch_id: hex32(batch_id),
        content_hash: hex32(record.content_hash),
        cid: record.cid.to_string(),
    };
    let context = json!({"result": {"batch_id": hex32(batch_id), "passed": i.passed}});
    run(
        state,
        actor,
        idem_key,
        "warehouse_inspection",
        request_hash,
        Some(record),
        action,
        context,
    )
    .await
}

// ======================== STAGE 4: LOGISTICS ========================

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub name: String,
    pub at: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogisticsRequest {
    pub batch_id: String,
    pub shipment_ref: String,
    pub checkpoint: Checkpoint,
    pub delivered: bool,
    #[serde(default)]
    pub expected_arrival: Option<String>,
    #[serde(default)]
    pub actual_arrival: Option<String>,
    /// CID of the full GPS trace (uploaded via /api/ipfs/upload). The trace itself never goes on-chain.
    #[serde(default)]
    pub gps_trace_cid: Option<String>,
    #[serde(default)]
    pub vehicle_ref: Option<String>,
    #[serde(default)]
    pub distance_km: Option<f64>,
}

/// POST /api/logistics/record (LOGISTICS custodian)
pub async fn record_logistics(
    State(state): State<AppState>,
    actor: Actor,
    headers: HeaderMap,
    Json(req): Json<LogisticsRequest>,
) -> Result<Response, ApiError> {
    let op = record_logistics_op(&state, &actor, ops::idempotency_key(&headers)?, req).await?;
    Ok(ops::respond(&state, &op))
}

pub async fn record_logistics_op(
    state: &AppState,
    actor: &Actor,
    idem_key: String,
    req: LogisticsRequest,
) -> Result<Operation, ApiError> {
    actor.require_role(roles::LOGISTICS, "logistics milestone")?;
    let request_hash = ops::request_hash("logistics", &req)?;
    if let Some(op) = ops::resume_existing(state, actor, &idem_key, &request_hash).await? {
        return Ok(op);
    }
    let batch_id = parse_id("batch_id", &req.batch_id)?;
    require_custody(state, actor, batch_id).await?;
    let mut data = Map::new();
    data.insert(
        "shipment_ref".into(),
        text("shipment_ref", &req.shipment_ref, 64)?.into(),
    );
    data.insert(
        "checkpoint".into(),
        json!({"name": text("checkpoint.name", &req.checkpoint.name, 120)?, "at": datetime("checkpoint.at", &req.checkpoint.at)?}),
    );
    data.insert("delivered".into(), req.delivered.into());
    put(
        &mut data,
        "expected_arrival",
        req.expected_arrival
            .as_deref()
            .map(|d| datetime("expected_arrival", d))
            .transpose()?,
    );
    put(
        &mut data,
        "actual_arrival",
        req.actual_arrival
            .as_deref()
            .map(|d| datetime("actual_arrival", d))
            .transpose()?,
    );
    put(
        &mut data,
        "gps_trace_cid",
        req.gps_trace_cid
            .as_deref()
            .map(|c| cid_ref("gps_trace_cid", c))
            .transpose()?,
    );
    put(
        &mut data,
        "vehicle_ref",
        opt_text("vehicle_ref", &req.vehicle_ref, 32)?,
    );
    put(
        &mut data,
        "distance_km",
        opt_range("distance_km", req.distance_km, 0.0, 10_000.0)?,
    );

    let record = records::prepare(
        state,
        RecordType::Logistics,
        batch_id,
        actor.address,
        Value::Object(data),
    )?;
    let action = ChainAction::RecordLogistics {
        batch_id: hex32(batch_id),
        content_hash: hex32(record.content_hash),
        cid: record.cid.to_string(),
    };
    let context = json!({"result": {"batch_id": hex32(batch_id), "delivered": req.delivered}});
    run(
        state,
        actor,
        idem_key,
        "logistics",
        request_hash,
        Some(record),
        action,
        context,
    )
    .await
}

// ======================== STAGE 5: PROCESSING ========================

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessingOutput {
    pub label: String,
    pub product: String,
    pub quantity_kg: f64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LabReport {
    #[serde(default)]
    pub oil_yield_pct: Option<f64>,
    #[serde(default)]
    pub free_fatty_acid_pct: Option<f64>,
    #[serde(default)]
    pub peroxide_value_meq_kg: Option<f64>,
    #[serde(default)]
    pub lab_ref: Option<String>,
    #[serde(default)]
    pub report_cid: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessBatchRequest {
    pub input_batch_id: String,
    pub method: String,
    pub outputs: Vec<ProcessingOutput>,
    #[serde(default)]
    pub lab_report: Option<LabReport>,
    #[serde(default)]
    pub completed_at: Option<String>,
}

/// POST /api/processing/batch (PROCESSOR custodian) — consumes the input batch, creates output batches.
pub async fn process_batch(
    State(state): State<AppState>,
    actor: Actor,
    headers: HeaderMap,
    Json(req): Json<ProcessBatchRequest>,
) -> Result<Response, ApiError> {
    let op = process_batch_op(&state, &actor, ops::idempotency_key(&headers)?, req).await?;
    Ok(ops::respond(&state, &op))
}

pub async fn process_batch_op(
    state: &AppState,
    actor: &Actor,
    idem_key: String,
    req: ProcessBatchRequest,
) -> Result<Operation, ApiError> {
    actor.require_role(roles::PROCESSOR, "processing")?;
    let request_hash = ops::request_hash("processing", &req)?;
    if let Some(op) = ops::resume_existing(state, actor, &idem_key, &request_hash).await? {
        return Ok(op);
    }
    let input = parse_id("input_batch_id", &req.input_batch_id)?;
    let batch = require_custody(state, actor, input).await?;
    if req.outputs.is_empty() || req.outputs.len() > 16 {
        return Err(ApiError::bad_request("outputs must contain 1-16 products"));
    }
    let mut outputs = Vec::new();
    let mut result_outputs = Vec::new();
    for (i, o) in req.outputs.iter().enumerate() {
        let id = generate_id();
        let label = text(&format!("outputs[{i}].label"), &o.label, 64)?;
        let product = one_of(
            &format!("outputs[{i}].product"),
            &o.product,
            &[
                "crude_oil",
                "refined_oil",
                "filtered_oil",
                "oilcake",
                "meal",
                "seed_for_sowing",
                "other",
            ],
        )?;
        let qty = range(
            &format!("outputs[{i}].quantity_kg"),
            o.quantity_kg,
            0.001,
            10_000_000.0,
        )?;
        outputs.push(
            json!({"batch_id": hex32(id), "label": label, "product": product, "quantity_kg": qty}),
        );
        result_outputs.push(json!({"batch_id": hex32(id), "label": label, "product": product}));
    }
    let mut data = Map::new();
    data.insert("method".into(), text("method", &req.method, 120)?.into());
    data.insert("outputs".into(), Value::Array(outputs));
    put(
        &mut data,
        "completed_at",
        req.completed_at
            .as_deref()
            .map(|d| datetime("completed_at", d))
            .transpose()?,
    );
    if let Some(l) = &req.lab_report {
        let mut lab = Map::new();
        put(
            &mut lab,
            "oil_yield_pct",
            opt_range("lab_report.oil_yield_pct", l.oil_yield_pct, 0.0, 100.0)?,
        );
        put(
            &mut lab,
            "free_fatty_acid_pct",
            opt_range(
                "lab_report.free_fatty_acid_pct",
                l.free_fatty_acid_pct,
                0.0,
                100.0,
            )?,
        );
        put(
            &mut lab,
            "peroxide_value_meq_kg",
            opt_range(
                "lab_report.peroxide_value_meq_kg",
                l.peroxide_value_meq_kg,
                0.0,
                1000.0,
            )?,
        );
        put(
            &mut lab,
            "lab_ref",
            opt_text("lab_report.lab_ref", &l.lab_ref, 64)?,
        );
        put(
            &mut lab,
            "report_cid",
            l.report_cid
                .as_deref()
                .map(|c| cid_ref("lab_report.report_cid", c))
                .transpose()?,
        );
        data.insert("lab_report".into(), Value::Object(lab));
    }
    let record = records::prepare(
        state,
        RecordType::Processing,
        input,
        actor.address,
        Value::Object(data),
    )?;
    let action = ChainAction::ProcessBatch {
        input_batch_id: hex32(input),
        output_batch_ids: result_outputs
            .iter()
            .map(|o| o["batch_id"].as_str().unwrap_or_default().to_string())
            .collect(),
        content_hash: hex32(record.content_hash),
        cid: record.cid.to_string(),
    };
    let context = json!({
        "result": {"input_batch_id": hex32(input), "outputs": result_outputs},
        "farmer_id": hex32(batch.farmerId),
    });
    run(
        state,
        actor,
        idem_key,
        "processing",
        request_hash,
        Some(record),
        action,
        context,
    )
    .await
}

pub async fn finalize_processing(
    state: &AppState,
    op: &Operation,
    ctx: &Value,
) -> Result<(), ApiError> {
    let parent = ctx_str(ctx, &["result", "input_batch_id"])
        .ok_or_else(|| ApiError::internal("ctx.input"))?;
    let farmer = ctx_str(ctx, &["farmer_id"]).unwrap_or_default();
    let parent_crop = {
        let p = parent.clone();
        state
            .db
            .call(move |c| db::get_batch(c, &p))
            .await?
            .and_then(|b| b.crop)
    };
    let outputs = ctx["result"]["outputs"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for o in outputs {
        let row = db::BatchRow {
            batch_id: o["batch_id"].as_str().unwrap_or_default().to_string(),
            farmer_id: farmer.clone(),
            parent_batch_id: Some(parent.clone()),
            label: o["label"].as_str().unwrap_or_default().to_string(),
            crop: parent_crop.clone(),
            op_id: op.id.clone(),
            created_at: db::now(),
        };
        state.db.call(move |c| db::upsert_batch(c, &row)).await?;
    }
    Ok(())
}

// ======================== STAGE 6: PACKAGING ========================

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageInfo {
    #[serde(default)]
    pub pack_size: Option<String>,
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub best_before: Option<String>,
    #[serde(default)]
    pub fssai_license_ref: Option<String>,
    #[serde(default)]
    pub label_artwork_cid: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSkuRequest {
    pub batch_id: String,
    pub label: String,
    pub unit_ids: Vec<String>,
    #[serde(default)]
    pub package: Option<PackageInfo>,
}

/// POST /api/packaging/sku (PACKAGER custodian of a product batch)
pub async fn create_sku(
    State(state): State<AppState>,
    actor: Actor,
    headers: HeaderMap,
    Json(req): Json<CreateSkuRequest>,
) -> Result<Response, ApiError> {
    let op = create_sku_op(&state, &actor, ops::idempotency_key(&headers)?, req).await?;
    Ok(ops::respond(&state, &op))
}

pub async fn create_sku_op(
    state: &AppState,
    actor: &Actor,
    idem_key: String,
    req: CreateSkuRequest,
) -> Result<Operation, ApiError> {
    actor.require_role(roles::PACKAGER, "packaging")?;
    let request_hash = ops::request_hash("packaging", &req)?;
    if let Some(op) = ops::resume_existing(state, actor, &idem_key, &request_hash).await? {
        return Ok(op);
    }
    let batch_id = parse_id("batch_id", &req.batch_id)?;
    let batch = require_custody(state, actor, batch_id).await?;
    if batch.parentBatchId == B256::ZERO {
        return Err(ApiError::Conflict(
            "only processed product batches can be packaged".into(),
        ));
    }
    if req.unit_ids.len() > 10_000 {
        return Err(ApiError::bad_request("at most 10000 units per SKU"));
    }
    let sku_id = generate_id();
    let tree = UnitTree::build(sku_id, &req.unit_ids).map_err(ApiError::BadRequest)?;
    let label = text("label", &req.label, 64)?;

    let mut data = Map::new();
    data.insert("sku_id".into(), hex32(sku_id).into());
    data.insert("label".into(), label.clone().into());
    data.insert("unit_count".into(), json!(req.unit_ids.len()));
    data.insert("units_root".into(), hex32(tree.root()).into());
    data.insert("unit_ids".into(), json!(req.unit_ids));
    if let Some(p) = &req.package {
        let mut pkg = Map::new();
        put(
            &mut pkg,
            "pack_size",
            opt_text("package.pack_size", &p.pack_size, 32)?,
        );
        put(
            &mut pkg,
            "material",
            opt_text("package.material", &p.material, 64)?,
        );
        put(
            &mut pkg,
            "best_before",
            p.best_before
                .as_deref()
                .map(|d| date("package.best_before", d).map(|d| d.to_string()))
                .transpose()?,
        );
        put(
            &mut pkg,
            "fssai_license_ref",
            opt_text("package.fssai_license_ref", &p.fssai_license_ref, 32)?,
        );
        put(
            &mut pkg,
            "label_artwork_cid",
            p.label_artwork_cid
                .as_deref()
                .map(|c| cid_ref("package.label_artwork_cid", c))
                .transpose()?,
        );
        data.insert("package".into(), Value::Object(pkg));
    }
    let record = records::prepare(
        state,
        RecordType::Packaging,
        batch_id,
        actor.address,
        Value::Object(data),
    )?;
    let action = ChainAction::CreateSku {
        sku_id: hex32(sku_id),
        batch_id: hex32(batch_id),
        units_root: hex32(tree.root()),
        unit_count: req.unit_ids.len() as u32,
        content_hash: hex32(record.content_hash),
        cid: record.cid.to_string(),
    };
    let context = json!({
        "result": {"sku_id": hex32(sku_id), "batch_id": hex32(batch_id), "units_root": hex32(tree.root()), "unit_count": req.unit_ids.len()},
        "label": label, "unit_ids": req.unit_ids,
    });
    run(
        state,
        actor,
        idem_key,
        "packaging",
        request_hash,
        Some(record),
        action,
        context,
    )
    .await
}

pub async fn finalize_packaging(
    state: &AppState,
    op: &Operation,
    ctx: &Value,
) -> Result<(), ApiError> {
    let row = db::SkuRow {
        sku_id: ctx_str(ctx, &["result", "sku_id"])
            .ok_or_else(|| ApiError::internal("ctx.sku_id"))?,
        batch_id: ctx_str(ctx, &["result", "batch_id"]).unwrap_or_default(),
        label: ctx_str(ctx, &["label"]).unwrap_or_default(),
        units_root: ctx_str(ctx, &["result", "units_root"]).unwrap_or_default(),
        unit_ids: serde_json::from_value(ctx["unit_ids"].clone()).unwrap_or_default(),
        op_id: op.id.clone(),
        created_at: db::now(),
    };
    state.db.call(move |c| db::upsert_sku(c, &row)).await
}

// ======================== STAGE 7: FRAUD REPORTING & REVIEW ========================

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportFraudRequest {
    #[serde(default)]
    pub batch_id: Option<String>,
    #[serde(default)]
    pub sku_id: Option<String>,
    pub category: String,
    pub description: String,
    #[serde(default)]
    pub evidence_cids: Vec<String>,
}

/// POST /api/fraud/report (any participant). Evidence is encrypted: reports can name people.
pub async fn report_fraud(
    State(state): State<AppState>,
    actor: Actor,
    headers: HeaderMap,
    Json(req): Json<ReportFraudRequest>,
) -> Result<Response, ApiError> {
    let op = report_fraud_op(&state, &actor, ops::idempotency_key(&headers)?, req).await?;
    Ok(ops::respond(&state, &op))
}

pub async fn report_fraud_op(
    state: &AppState,
    actor: &Actor,
    idem_key: String,
    req: ReportFraudRequest,
) -> Result<Operation, ApiError> {
    if !actor.is_admin {
        actor.require_any_role()?;
    }
    let request_hash = ops::request_hash("fraud_report", &req)?;
    if let Some(op) = ops::resume_existing(state, actor, &idem_key, &request_hash).await? {
        return Ok(op);
    }
    let batch_id = match (&req.batch_id, &req.sku_id) {
        (Some(b), None) => parse_id("batch_id", b)?,
        (None, Some(s)) => {
            let sku = parse_id("sku_id", s)?;
            let origin = state
                .chain_client
                .registry()
                .skus(sku)
                .call()
                .await
                .map_err(|e| ApiError::Chain(e.to_string()))?;
            if origin.packagedAt == 0 {
                return Err(ApiError::NotFound("SKU not found on-chain".into()));
            }
            origin.batchId
        }
        _ => {
            return Err(ApiError::bad_request(
                "provide exactly one of batch_id or sku_id",
            ))
        }
    };
    let category = one_of(
        "category",
        &req.category,
        &[
            "adulteration",
            "mislabeling",
            "quantity_shortfall",
            "counterfeit",
            "quality_misrepresentation",
            "other",
        ],
    )?;
    if req.evidence_cids.len() > 20 {
        return Err(ApiError::bad_request("at most 20 evidence CIDs"));
    }
    let evidence = req
        .evidence_cids
        .iter()
        .map(|c| cid_ref("evidence_cids", c))
        .collect::<Result<Vec<_>, _>>()?;
    let mut data = Map::new();
    data.insert("category".into(), category.clone().into());
    data.insert(
        "description".into(),
        text("description", &req.description, 2000)?.into(),
    );
    data.insert("evidence_cids".into(), json!(evidence));
    put(&mut data, "sku_id", req.sku_id.clone());

    let record = records::prepare(
        state,
        RecordType::FraudReport,
        batch_id,
        actor.address,
        Value::Object(data),
    )?;
    let report_digest = hex32(record.record_digest);
    let action = ChainAction::ReportFraud {
        batch_id: hex32(batch_id),
        content_hash: hex32(record.content_hash),
        cid: record.cid.to_string(),
    };
    let context = json!({"result": {"report_digest": report_digest, "batch_id": hex32(batch_id), "category": category}});
    run(
        state,
        actor,
        idem_key,
        "fraud_report",
        request_hash,
        Some(record),
        action,
        context,
    )
    .await
}

pub async fn finalize_fraud_report(
    state: &AppState,
    _op: &Operation,
    ctx: &Value,
) -> Result<(), ApiError> {
    let row = db::FraudRow {
        report_digest: ctx_str(ctx, &["result", "report_digest"])
            .ok_or_else(|| ApiError::internal("ctx.report"))?,
        batch_id: ctx_str(ctx, &["result", "batch_id"]).unwrap_or_default(),
        status: "open".into(),
        attribution: None,
        resolution_digest: None,
        updated_at: db::now(),
    };
    state.db.call(move |c| db::upsert_fraud(c, &row)).await
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveFraudRequest {
    pub report_digest: String,
    pub upheld: bool,
    /// Who the upheld finding is attributed to. Only `farmer` affects farmer rankings.
    pub attribution: String,
    pub notes: String,
}

pub const FRAUD_ATTRIBUTIONS: [&str; 7] = [
    "farmer",
    "fpo",
    "warehouse",
    "logistics",
    "processor",
    "packager",
    "unattributed",
];

/// POST /api/fraud/resolve (admin, not the reporter)
pub async fn resolve_fraud(
    State(state): State<AppState>,
    actor: Actor,
    headers: HeaderMap,
    Json(req): Json<ResolveFraudRequest>,
) -> Result<Response, ApiError> {
    let op = resolve_fraud_op(&state, &actor, ops::idempotency_key(&headers)?, req).await?;
    Ok(ops::respond(&state, &op))
}

pub async fn resolve_fraud_op(
    state: &AppState,
    actor: &Actor,
    idem_key: String,
    req: ResolveFraudRequest,
) -> Result<Operation, ApiError> {
    actor.require_admin()?;
    let request_hash = ops::request_hash("fraud_resolution", &req)?;
    if let Some(op) = ops::resume_existing(state, actor, &idem_key, &request_hash).await? {
        return Ok(op);
    }
    let report = parse_id("report_digest", &req.report_digest)?;
    let status = state
        .chain_client
        .registry()
        .fraudStatus(report)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    if status != 1 {
        return Err(ApiError::Conflict("report is not open on-chain".into()));
    }
    let anchor = state
        .chain_client
        .registry()
        .anchors(report)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    let attribution = one_of("attribution", &req.attribution, &FRAUD_ATTRIBUTIONS)?;
    if !req.upheld && attribution != "unattributed" {
        return Err(ApiError::bad_request(
            "a dismissed report must be `unattributed`",
        ));
    }
    let data = json!({
        "report_digest": hex32(report),
        "upheld": req.upheld,
        "attribution": attribution,
        "notes": text("notes", &req.notes, 2000)?,
    });
    let record = records::prepare(
        state,
        RecordType::FraudResolution,
        anchor.subjectId,
        actor.address,
        data,
    )?;
    let action = ChainAction::ResolveFraud {
        report_digest: hex32(report),
        upheld: req.upheld,
        content_hash: hex32(record.content_hash),
        cid: record.cid.to_string(),
    };
    let context = json!({
        "result": {"report_digest": hex32(report), "upheld": req.upheld, "attribution": attribution},
        "batch_id": hex32(anchor.subjectId), "resolution_digest": hex32(record.record_digest),
    });
    run(
        state,
        actor,
        idem_key,
        "fraud_resolution",
        request_hash,
        Some(record),
        action,
        context,
    )
    .await
}

pub async fn finalize_fraud_resolution(
    state: &AppState,
    _op: &Operation,
    ctx: &Value,
) -> Result<(), ApiError> {
    let upheld = ctx["result"]["upheld"].as_bool().unwrap_or(false);
    let row = db::FraudRow {
        report_digest: ctx_str(ctx, &["result", "report_digest"])
            .ok_or_else(|| ApiError::internal("ctx.report"))?,
        batch_id: ctx_str(ctx, &["batch_id"]).unwrap_or_default(),
        status: if upheld {
            "upheld".into()
        } else {
            "dismissed".into()
        },
        attribution: ctx_str(ctx, &["result", "attribution"]),
        resolution_digest: ctx_str(ctx, &["resolution_digest"]),
        updated_at: db::now(),
    };
    state.db.call(move |c| db::upsert_fraud(c, &row)).await
}

// ======================== STAGE 8: AI ASSESSMENT (COMMIT–REVEAL) ========================

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AiInput {
    pub cid: String,
    pub description: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AiAssessment {
    pub model_id: String,
    pub model_version: String,
    pub method: String,
    #[serde(default)]
    pub inputs: Vec<AiInput>,
    /// Model outputs as produced by the oracle (e.g. predicted grade and per-attribute scores).
    pub outputs: Map<String, Value>,
    #[serde(default)]
    pub limitations: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommitAiRequest {
    pub batch_id: String,
    pub assessment: AiAssessment,
}

/// POST /api/ai/commit (AI_ORACLE). The assessment stays sealed in the database (not on IPFS) until reveal.
pub async fn commit_ai_score(
    State(state): State<AppState>,
    actor: Actor,
    headers: HeaderMap,
    Json(req): Json<CommitAiRequest>,
) -> Result<Response, ApiError> {
    let op = commit_ai_score_op(&state, &actor, ops::idempotency_key(&headers)?, req).await?;
    Ok(ops::respond(&state, &op))
}

pub async fn commit_ai_score_op(
    state: &AppState,
    actor: &Actor,
    idem_key: String,
    req: CommitAiRequest,
) -> Result<Operation, ApiError> {
    actor.require_role(roles::AI_ORACLE, "AI assessment")?;
    let request_hash = ops::request_hash("ai_commit", &req)?;
    if let Some(op) = ops::resume_existing(state, actor, &idem_key, &request_hash).await? {
        return Ok(op);
    }
    let batch_id = parse_id("batch_id", &req.batch_id)?;
    let batch = state
        .chain_client
        .registry()
        .getBatch(batch_id)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    if batch.createdAt == 0 {
        return Err(ApiError::NotFound("batch not found on-chain".into()));
    }
    let a = &req.assessment;
    if a.outputs.is_empty() || a.inputs.len() > 50 {
        return Err(ApiError::bad_request(
            "assessment.outputs must be non-empty; at most 50 inputs",
        ));
    }
    let inputs = a
        .inputs
        .iter()
        .map(|i| -> Result<Value, ApiError> {
            Ok(json!({"cid": cid_ref("inputs.cid", &i.cid)?, "description": text("inputs.description", &i.description, 300)?}))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut data = Map::new();
    data.insert("model_id".into(), text("model_id", &a.model_id, 64)?.into());
    data.insert(
        "model_version".into(),
        text("model_version", &a.model_version, 32)?.into(),
    );
    data.insert("method".into(), text("method", &a.method, 500)?.into());
    data.insert("inputs".into(), Value::Array(inputs));
    data.insert("outputs".into(), Value::Object(a.outputs.clone()));
    put(
        &mut data,
        "limitations",
        opt_text("limitations", &a.limitations, 1000)?,
    );

    let record = canonical::build_record(
        state.domain(),
        RecordType::AiAssessment,
        batch_id,
        actor.address,
        Value::Object(data),
    );
    let canonical_bytes =
        canonical::canonical_json(&record).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    if canonical_bytes.len() > crate::ipfs::MAX_BLOCK_SIZE {
        return Err(ApiError::bad_request("assessment is too large"));
    }
    let content_hash = canonical::content_hash(&canonical_bytes);
    let salt = generate_id();
    let commit_hash = canonical::ai_commit(batch_id, content_hash, salt, actor.address);

    // The sealed assessment and salt are stored in the operation row itself (atomically with the op), so a
    // crash after the commit transaction can never lose what is needed to reveal it.
    let key = state.config.secrets.pii_encryption_key;
    let aad = format!("ai:{}", hex32(commit_hash));
    let action = ChainAction::CommitAi {
        batch_id: hex32(batch_id),
        commit_hash: hex32(commit_hash),
    };
    let context = json!({
        "result": {"batch_id": hex32(batch_id), "commit_hash": hex32(commit_hash)},
        "oracle_id": actor.id,
        "content_hash": hex32(content_hash),
        "sealed_payload": crypto::seal(&key, &canonical_bytes, &aad),
        "sealed_salt": crypto::seal(&key, salt.as_slice(), &aad),
    });
    run(
        state,
        actor,
        idem_key,
        "ai_commit",
        request_hash,
        None,
        action,
        context,
    )
    .await
}

pub async fn finalize_ai_commit(
    state: &AppState,
    op: &Operation,
    ctx: &Value,
) -> Result<(), ApiError> {
    let commit_hash =
        ctx_str(ctx, &["result", "commit_hash"]).ok_or_else(|| ApiError::internal("ctx.commit"))?;
    let batch_hex =
        ctx_str(ctx, &["result", "batch_id"]).ok_or_else(|| ApiError::internal("ctx.batch"))?;
    let row = db::AiCommitRow {
        commit_hash: commit_hash.clone(),
        batch_id: batch_hex.clone(),
        oracle_id: ctx_str(ctx, &["oracle_id"]).unwrap_or_default(),
        sealed_payload: ctx_str(ctx, &["sealed_payload"])
            .ok_or_else(|| ApiError::internal("ctx.sealed_payload"))?,
        sealed_salt: ctx_str(ctx, &["sealed_salt"])
            .ok_or_else(|| ApiError::internal("ctx.sealed_salt"))?,
        content_hash: ctx_str(ctx, &["content_hash"]).unwrap_or_default(),
        status: "pending_commit".into(),
        committed_at: None,
        commit_op_id: op.id.clone(),
        reveal_op_id: None,
        record_digest: None,
        created_at: db::now(),
        updated_at: db::now(),
    };
    state
        .db
        .call(move |c| db::insert_ai_commit(c, &row))
        .await?;
    let op_id = op.id.clone();
    let Some(mut row) = state
        .db
        .call(move |c| db::ai_commit_by_op(c, &op_id))
        .await?
    else {
        return Err(ApiError::internal("AI commit row missing after insert"));
    };
    let batch =
        crate::chain::parse_b256(&batch_hex).map_err(|e| ApiError::internal(e.to_string()))?;
    let onchain = state
        .chain_client
        .registry()
        .aiCommits(batch)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    if hex32(onchain.commitHash) == commit_hash && row.status == "pending_commit" {
        row.status = "committed".into();
        row.committed_at = Some(onchain.committedAt as i64);
        state
            .db
            .call(move |c| db::update_ai_commit(c, &row))
            .await?;
    }
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevealAiRequest {
    pub batch_id: String,
}

#[derive(Debug, Serialize)]
pub struct AiStatusView {
    pub batch_id: String,
    pub commit_hash: String,
    pub status: String,
    pub committed_at: Option<i64>,
    pub reveal_not_before: Option<i64>,
    pub reveal_deadline: Option<i64>,
    pub chain_time: u64,
    pub record_digest: Option<String>,
}

pub const MIN_REVEAL_DELAY: i64 = 3600;
pub const MAX_REVEAL_DELAY: i64 = 24 * 3600;

/// Reveal a committed assessment: upload it to IPFS and anchor it. Used by the API and the reconciler.
pub async fn reveal_commit(state: &AppState, row: &db::AiCommitRow) -> Result<Operation, ApiError> {
    let key = state.config.secrets.pii_encryption_key;
    let aad = format!("ai:{}", row.commit_hash);
    let canonical_bytes =
        crypto::open(&key, &row.sealed_payload, &aad).map_err(ApiError::internal)?;
    let salt_bytes = crypto::open(&key, &row.sealed_salt, &aad).map_err(ApiError::internal)?;
    let salt =
        B256::try_from(salt_bytes.as_slice()).map_err(|_| ApiError::internal("salt length"))?;
    let batch_id =
        crate::chain::parse_b256(&row.batch_id).map_err(|e| ApiError::internal(e.to_string()))?;
    let record = records::prepare_from_canonical(
        state,
        RecordType::AiAssessment,
        batch_id,
        canonical_bytes,
    )?;
    if hex32(record.content_hash) != row.content_hash {
        return Err(ApiError::internal(
            "sealed assessment does not match its commitment",
        ));
    }
    let oracle = ops::actor_for(state, &row.oracle_id).await?;
    let action = ChainAction::RevealAi {
        batch_id: row.batch_id.clone(),
        content_hash: row.content_hash.clone(),
        salt: hex32(salt),
        cid: record.cid.to_string(),
    };
    let op = ops::submit(
        state,
        &oracle,
        NewOperation {
            idem_key: format!("ai-reveal:{}", row.commit_hash),
            kind: "ai_reveal",
            request_hash: format!("ai-reveal:{}", row.commit_hash),
            record: Some(record),
            action,
            context: json!({"result": {"batch_id": row.batch_id, "commit_hash": row.commit_hash}}),
        },
    )
    .await?;
    Ok(op)
}

/// POST /api/ai/reveal (the committing AI_ORACLE). Fails clearly before the reveal window opens.
pub async fn reveal_ai_score(
    State(state): State<AppState>,
    actor: Actor,
    Json(req): Json<RevealAiRequest>,
) -> Result<Response, ApiError> {
    actor.require_role(roles::AI_ORACLE, "AI assessment reveal")?;
    let batch_id = hex32(parse_id("batch_id", &req.batch_id)?);
    let b = batch_id.clone();
    let row = state
        .db
        .call(move |c| db::latest_ai_commit_for_batch(c, &b))
        .await?
        .ok_or_else(|| ApiError::NotFound("no AI commitment for this batch".into()))?;
    if row.oracle_id != actor.id {
        return Err(ApiError::Forbidden(
            "only the committing oracle can reveal".into(),
        ));
    }
    if row.status == "pending_commit" {
        return Err(ApiError::Conflict(
            "commit transaction is not confirmed yet".into(),
        ));
    }
    let now = state
        .chain_client
        .latest_timestamp()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))? as i64;
    let committed = row.committed_at.unwrap_or(now);
    if row.status == "committed" && now < committed + MIN_REVEAL_DELAY {
        return Err(ApiError::Conflict(format!(
            "reveal window opens at chain time {} (in {}s)",
            committed + MIN_REVEAL_DELAY,
            committed + MIN_REVEAL_DELAY - now
        )));
    }
    let op = reveal_commit(&state, &row).await?;
    Ok(ops::respond(&state, &op))
}

pub async fn finalize_ai_reveal(
    state: &AppState,
    op: &Operation,
    ctx: &Value,
) -> Result<(), ApiError> {
    let batch = ctx_str(ctx, &["result", "batch_id"]).unwrap_or_default();
    let commit = ctx_str(ctx, &["result", "commit_hash"]).unwrap_or_default();
    if let Some(mut row) = state
        .db
        .call(move |c| db::latest_ai_commit_for_batch(c, &batch))
        .await?
    {
        if row.commit_hash == commit {
            row.status = "revealed".into();
            row.reveal_op_id = Some(op.id.clone());
            row.record_digest = op.record_digest.clone();
            state
                .db
                .call(move |c| db::update_ai_commit(c, &row))
                .await?;
        }
    }
    Ok(())
}

/// GET /api/ai/{batch_id}
pub async fn ai_status(
    State(state): State<AppState>,
    _actor: Actor,
    Path(batch_id): Path<String>,
) -> Result<Json<AiStatusView>, ApiError> {
    let batch_id = hex32(parse_id("batch_id", &batch_id)?);
    let b = batch_id.clone();
    let row = state
        .db
        .call(move |c| db::latest_ai_commit_for_batch(c, &b))
        .await?
        .ok_or_else(|| ApiError::NotFound("no AI commitment for this batch".into()))?;
    let chain_time = state
        .chain_client
        .latest_timestamp()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    Ok(Json(AiStatusView {
        batch_id,
        commit_hash: row.commit_hash,
        status: row.status,
        committed_at: row.committed_at,
        reveal_not_before: row.committed_at.map(|t| t + MIN_REVEAL_DELAY),
        reveal_deadline: row.committed_at.map(|t| t + MAX_REVEAL_DELAY),
        chain_time,
        record_digest: row.record_digest,
    }))
}

// ======================== READ ENDPOINTS ========================

#[derive(Debug, Serialize)]
pub struct FarmerView {
    pub farmer_id: String,
    pub label: String,
    pub region: Option<String>,
    pub primary_crop: Option<String>,
    pub registered_by: String,
    pub registration_digest: String,
    pub onchain_registered_at: u64,
    pub batches: Vec<db::BatchRow>,
}

/// GET /api/farmers — participants only (farmer list is not public).
pub async fn list_farmers(
    State(state): State<AppState>,
    actor: Actor,
) -> Result<Json<Vec<db::FarmerRow>>, ApiError> {
    if !actor.is_admin {
        actor.require_any_role()?;
    }
    Ok(Json(state.db.call(db::list_farmers).await?))
}

/// GET /api/farmers/{id}
pub async fn get_farmer(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<String>,
) -> Result<Json<FarmerView>, ApiError> {
    if !actor.is_admin {
        actor.require_any_role()?;
    }
    let farmer_id = parse_id("farmer_id", &id)?;
    let key = hex32(farmer_id);
    let k2 = key.clone();
    let row = state
        .db
        .call(move |c| db::get_farmer(c, &k2))
        .await?
        .ok_or_else(|| ApiError::NotFound("farmer not found".into()))?;
    let onchain = state
        .chain_client
        .registry()
        .farmers(farmer_id)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    let batches = state
        .db
        .call(move |c| db::list_batches(c, Some(&key)))
        .await?;
    Ok(Json(FarmerView {
        farmer_id: row.farmer_id,
        label: row.label,
        region: row.region,
        primary_crop: row.primary_crop,
        registered_by: row.registered_by,
        registration_digest: row.registration_digest,
        onchain_registered_at: onchain.registeredAt,
        batches,
    }))
}

/// GET /api/batches
pub async fn list_batches(
    State(state): State<AppState>,
    actor: Actor,
) -> Result<Json<Vec<db::BatchRow>>, ApiError> {
    if !actor.is_admin {
        actor.require_any_role()?;
    }
    Ok(Json(state.db.call(|c| db::list_batches(c, None)).await?))
}

#[derive(Debug, Serialize)]
pub struct BatchView {
    pub batch_id: String,
    pub label: Option<String>,
    pub crop: Option<String>,
    pub farmer_id: String,
    pub parent_batch_id: Option<String>,
    pub custodian: String,
    pub custodian_participant: Option<String>,
    pub stage: &'static str,
    pub record_count: u16,
    pub head: String,
    pub created_at: u64,
}

/// GET /api/batches/{id} — on-chain state (public).
pub async fn get_batch(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<BatchView>, ApiError> {
    let batch_id = parse_id("batch_id", &id)?;
    let b = state
        .chain_client
        .registry()
        .getBatch(batch_id)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    if b.createdAt == 0 {
        return Err(ApiError::NotFound("batch not found on-chain".into()));
    }
    let key = hex32(batch_id);
    let row = state.db.call(move |c| db::get_batch(c, &key)).await?;
    let custodian = hexaddr(b.custodian);
    let cust = custodian.clone();
    let custodian_participant = state
        .db
        .call(move |c| db::participant_by_address(c, &cust))
        .await?
        .map(|p| p.name);
    Ok(Json(BatchView {
        batch_id: hex32(batch_id),
        label: row.as_ref().map(|r| r.label.clone()),
        crop: row.and_then(|r| r.crop),
        farmer_id: hex32(b.farmerId),
        parent_batch_id: (b.parentBatchId != B256::ZERO).then(|| hex32(b.parentBatchId)),
        custodian,
        custodian_participant,
        stage: STAGES.get(b.stage as usize).copied().unwrap_or("unknown"),
        record_count: b.recordCount,
        head: hex32(b.head),
        created_at: b.createdAt,
    }))
}
