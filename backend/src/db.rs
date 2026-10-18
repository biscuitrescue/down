//! SQLite persistence (application index + operation journal). The chain and IPFS stay the source of truth
//! for provenance; this database records what the API did so retries, recovery and rankings work.

use crate::error::ApiError;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE participants (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    roles         INTEGER NOT NULL,
    address       TEXT NOT NULL UNIQUE,
    encrypted_key TEXT NOT NULL,
    token_hash    TEXT NOT NULL UNIQUE,
    created_at    TEXT NOT NULL
);

-- Journal of every write: one row per (actor, idempotency key). Status machine:
-- prepared -> stored (IPFS) -> submitted (tx hash known) -> confirmed | reverted ; failed = rejected before send.
CREATE TABLE operations (
    id             TEXT PRIMARY KEY,
    idem_key       TEXT NOT NULL,
    actor_id       TEXT NOT NULL,
    kind           TEXT NOT NULL,
    request_hash   TEXT NOT NULL,
    status         TEXT NOT NULL,
    subject_id     TEXT,
    record_type    INTEGER,
    content_hash   TEXT,
    record_digest  TEXT,
    cid            TEXT,
    payload        BLOB,
    encrypted      INTEGER NOT NULL DEFAULT 0,
    public_json    TEXT,
    action         TEXT NOT NULL,
    context        TEXT NOT NULL,
    tx_hash        TEXT,
    block_number   INTEGER,
    error          TEXT,
    attempts       INTEGER NOT NULL DEFAULT 0,
    submitted_at   INTEGER,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    UNIQUE (actor_id, idem_key)
);
CREATE INDEX operations_status ON operations(status);

CREATE TABLE farmers (
    farmer_id           TEXT PRIMARY KEY,
    registered_by       TEXT NOT NULL,
    label               TEXT NOT NULL,
    region              TEXT,
    primary_crop        TEXT,
    wallet_address      TEXT,
    registration_digest TEXT NOT NULL,
    cid                 TEXT NOT NULL,
    op_id               TEXT NOT NULL,
    created_at          TEXT NOT NULL
);

CREATE TABLE batches (
    batch_id        TEXT PRIMARY KEY,
    farmer_id       TEXT NOT NULL,
    parent_batch_id TEXT,
    label           TEXT NOT NULL,
    crop            TEXT,
    op_id           TEXT NOT NULL,
    created_at      TEXT NOT NULL
);
CREATE INDEX batches_farmer ON batches(farmer_id);

CREATE TABLE records (
    record_digest     TEXT PRIMARY KEY,
    subject_id        TEXT NOT NULL,
    record_type       INTEGER NOT NULL,
    content_hash      TEXT NOT NULL,
    cid               TEXT NOT NULL,
    encrypted         INTEGER NOT NULL,
    public_json       TEXT,
    submitter_id      TEXT NOT NULL,
    submitter_address TEXT NOT NULL,
    tx_hash           TEXT,
    block_number      INTEGER,
    op_id             TEXT NOT NULL,
    created_at        TEXT NOT NULL
);
CREATE INDEX records_subject ON records(subject_id);

CREATE TABLE skus (
    sku_id     TEXT PRIMARY KEY,
    batch_id   TEXT NOT NULL,
    label      TEXT NOT NULL,
    units_root TEXT NOT NULL,
    unit_ids   TEXT NOT NULL,
    op_id      TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE fraud_reports (
    report_digest     TEXT PRIMARY KEY,
    batch_id          TEXT NOT NULL,
    status            TEXT NOT NULL,
    attribution       TEXT,
    resolution_digest TEXT,
    updated_at        TEXT NOT NULL
);

CREATE TABLE ai_commits (
    commit_hash       TEXT PRIMARY KEY,
    batch_id          TEXT NOT NULL,
    oracle_id         TEXT NOT NULL,
    sealed_payload    TEXT NOT NULL,
    sealed_salt       TEXT NOT NULL,
    content_hash      TEXT NOT NULL,
    status            TEXT NOT NULL,
    committed_at      INTEGER,
    commit_op_id      TEXT NOT NULL,
    reveal_op_id      TEXT,
    record_digest     TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);

CREATE TABLE nfts (
    kind             TEXT NOT NULL,
    source_ref       TEXT NOT NULL,
    token_id         INTEGER,
    contract_address TEXT NOT NULL,
    chain_id         INTEGER NOT NULL,
    batch_id         TEXT,
    farmer_id        TEXT,
    owner            TEXT,
    token_uri        TEXT NOT NULL,
    metadata_cid     TEXT NOT NULL,
    op_id            TEXT NOT NULL,
    status           TEXT NOT NULL,
    tx_hash          TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL,
    PRIMARY KEY (kind, source_ref)
);
"#,
    r#"
-- Backup pinning queue: one row per (block CID, backup target). Survives restarts.
CREATE TABLE pin_jobs (
    cid             TEXT NOT NULL,
    target          TEXT NOT NULL,
    status          TEXT NOT NULL,   -- pending | remote_pending | pinned | failed
    attempts        INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER NOT NULL,
    request_id      TEXT,
    last_error      TEXT,
    verified_at     TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    PRIMARY KEY (cid, target)
);
CREATE INDEX pin_jobs_due ON pin_jobs(status, next_attempt_at);
"#,
];

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn new_id(prefix: &str) -> String {
    let bytes: [u8; 12] = rand::random();
    format!("{prefix}_{}", hex::encode(bytes))
}

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> anyhow::Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY);",
        )?;
        let current: i64 = conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )?;
        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let version = i as i64 + 1;
            if version > current {
                conn.execute_batch(&format!("BEGIN; {sql} INSERT INTO schema_migrations(version) VALUES ({version}); COMMIT;"))?;
            }
        }
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Run a closure against the connection on the blocking pool.
    pub async fn call<F, R>(&self, f: F) -> Result<R, ApiError>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn
                .lock()
                .map_err(|_| ApiError::internal("database mutex poisoned"))?;
            f(&guard).map_err(ApiError::from)
        })
        .await
        .map_err(|e| ApiError::internal(format!("database task: {e}")))?
    }
}

// ======================== PARTICIPANTS ========================

#[derive(Debug, Clone, Serialize)]
pub struct Participant {
    pub id: String,
    pub name: String,
    pub roles: u64,
    pub address: String,
    #[serde(skip)]
    pub encrypted_key: String,
    #[serde(skip)]
    pub token_hash: String,
    pub created_at: String,
}

fn participant_from_row(r: &Row) -> rusqlite::Result<Participant> {
    Ok(Participant {
        id: r.get(0)?,
        name: r.get(1)?,
        roles: r.get::<_, i64>(2)? as u64,
        address: r.get(3)?,
        encrypted_key: r.get(4)?,
        token_hash: r.get(5)?,
        created_at: r.get(6)?,
    })
}

const PARTICIPANT_COLS: &str = "id, name, roles, address, encrypted_key, token_hash, created_at";

pub fn insert_participant(c: &Connection, p: &Participant) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO participants (id, name, roles, address, encrypted_key, token_hash, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![p.id, p.name, p.roles as i64, p.address, p.encrypted_key, p.token_hash, p.created_at],
    )?;
    Ok(())
}

pub fn participant_by_token_hash(
    c: &Connection,
    hash: &str,
) -> rusqlite::Result<Option<Participant>> {
    c.query_row(
        &format!("SELECT {PARTICIPANT_COLS} FROM participants WHERE token_hash = ?1"),
        [hash],
        participant_from_row,
    )
    .optional()
}

pub fn participant_by_id(c: &Connection, id: &str) -> rusqlite::Result<Option<Participant>> {
    c.query_row(
        &format!("SELECT {PARTICIPANT_COLS} FROM participants WHERE id = ?1"),
        [id],
        participant_from_row,
    )
    .optional()
}

pub fn participant_by_address(
    c: &Connection,
    address: &str,
) -> rusqlite::Result<Option<Participant>> {
    c.query_row(
        &format!("SELECT {PARTICIPANT_COLS} FROM participants WHERE lower(address) = lower(?1)"),
        [address],
        participant_from_row,
    )
    .optional()
}

pub fn list_participants(c: &Connection) -> rusqlite::Result<Vec<Participant>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {PARTICIPANT_COLS} FROM participants ORDER BY created_at, id"
    ))?;
    let rows = stmt.query_map([], participant_from_row)?;
    rows.collect()
}

// ======================== OPERATIONS ========================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpStatus {
    Prepared,
    Stored,
    Submitted,
    Confirmed,
    Reverted,
    Failed,
}

impl OpStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Stored => "stored",
            Self::Submitted => "submitted",
            Self::Confirmed => "confirmed",
            Self::Reverted => "reverted",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "prepared" => Self::Prepared,
            "stored" => Self::Stored,
            "submitted" => Self::Submitted,
            "confirmed" => Self::Confirmed,
            "reverted" => Self::Reverted,
            _ => Self::Failed,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Confirmed | Self::Reverted | Self::Failed)
    }
}

#[derive(Debug, Clone)]
pub struct Operation {
    pub id: String,
    pub idem_key: String,
    pub actor_id: String,
    pub kind: String,
    pub request_hash: String,
    pub status: OpStatus,
    pub subject_id: Option<String>,
    pub record_type: Option<u8>,
    pub content_hash: Option<String>,
    pub record_digest: Option<String>,
    pub cid: Option<String>,
    pub payload: Option<Vec<u8>>,
    pub encrypted: bool,
    pub public_json: Option<String>,
    pub action: String,
    pub context: String,
    pub tx_hash: Option<String>,
    pub block_number: Option<u64>,
    pub error: Option<String>,
    pub attempts: u32,
    pub submitted_at: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

const OP_COLS: &str = "id, idem_key, actor_id, kind, request_hash, status, subject_id, record_type, content_hash, record_digest, cid, payload, encrypted, public_json, action, context, tx_hash, block_number, error, attempts, submitted_at, created_at, updated_at";

fn op_from_row(r: &Row) -> rusqlite::Result<Operation> {
    Ok(Operation {
        id: r.get(0)?,
        idem_key: r.get(1)?,
        actor_id: r.get(2)?,
        kind: r.get(3)?,
        request_hash: r.get(4)?,
        status: OpStatus::parse(&r.get::<_, String>(5)?),
        subject_id: r.get(6)?,
        record_type: r.get::<_, Option<i64>>(7)?.map(|v| v as u8),
        content_hash: r.get(8)?,
        record_digest: r.get(9)?,
        cid: r.get(10)?,
        payload: r.get(11)?,
        encrypted: r.get::<_, i64>(12)? != 0,
        public_json: r.get(13)?,
        action: r.get(14)?,
        context: r.get(15)?,
        tx_hash: r.get(16)?,
        block_number: r.get::<_, Option<i64>>(17)?.map(|v| v as u64),
        error: r.get(18)?,
        attempts: r.get::<_, i64>(19)? as u32,
        submitted_at: r.get(20)?,
        created_at: r.get(21)?,
        updated_at: r.get(22)?,
    })
}

/// Insert `op` unless the (actor, idempotency key) already exists; returns the stored row and whether it is new.
pub fn insert_or_get_operation(
    c: &Connection,
    op: &Operation,
) -> rusqlite::Result<(Operation, bool)> {
    let inserted = c.execute(
        &format!("INSERT INTO operations ({OP_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23) ON CONFLICT(actor_id, idem_key) DO NOTHING"),
        params![
            op.id, op.idem_key, op.actor_id, op.kind, op.request_hash, op.status.as_str(), op.subject_id,
            op.record_type.map(i64::from), op.content_hash, op.record_digest, op.cid, op.payload,
            op.encrypted as i64, op.public_json, op.action, op.context, op.tx_hash,
            op.block_number.map(|b| b as i64), op.error, op.attempts as i64, op.submitted_at,
            op.created_at, op.updated_at
        ],
    )?;
    let stored = c.query_row(
        &format!("SELECT {OP_COLS} FROM operations WHERE actor_id = ?1 AND idem_key = ?2"),
        params![op.actor_id, op.idem_key],
        op_from_row,
    )?;
    Ok((stored, inserted == 1))
}

pub fn operation_by_key(
    c: &Connection,
    actor_id: &str,
    idem_key: &str,
) -> rusqlite::Result<Option<Operation>> {
    c.query_row(
        &format!("SELECT {OP_COLS} FROM operations WHERE actor_id = ?1 AND idem_key = ?2"),
        params![actor_id, idem_key],
        op_from_row,
    )
    .optional()
}

pub fn get_operation(c: &Connection, id: &str) -> rusqlite::Result<Option<Operation>> {
    c.query_row(
        &format!("SELECT {OP_COLS} FROM operations WHERE id = ?1"),
        [id],
        op_from_row,
    )
    .optional()
}

pub fn operations_with_status(
    c: &Connection,
    statuses: &[OpStatus],
) -> rusqlite::Result<Vec<Operation>> {
    let list = statuses
        .iter()
        .map(|s| format!("'{}'", s.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let mut stmt = c.prepare(&format!(
        "SELECT {OP_COLS} FROM operations WHERE status IN ({list}) ORDER BY created_at, id"
    ))?;
    let rows = stmt.query_map([], op_from_row)?;
    rows.collect()
}

pub fn operation_by_tx_hash(c: &Connection, tx_hash: &str) -> rusqlite::Result<Option<Operation>> {
    c.query_row(
        &format!("SELECT {OP_COLS} FROM operations WHERE lower(tx_hash) = lower(?1)"),
        [tx_hash],
        op_from_row,
    )
    .optional()
}

pub fn operation_by_record_digest(
    c: &Connection,
    digest: &str,
) -> rusqlite::Result<Option<Operation>> {
    c.query_row(
        &format!(
            "SELECT {OP_COLS} FROM operations WHERE record_digest = ?1 ORDER BY created_at LIMIT 1"
        ),
        [digest],
        op_from_row,
    )
    .optional()
}

pub fn update_operation(c: &Connection, op: &Operation) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE operations SET status=?2, cid=?3, tx_hash=?4, block_number=?5, error=?6, attempts=?7, submitted_at=?8, updated_at=?9 WHERE id=?1",
        params![
            op.id, op.status.as_str(), op.cid, op.tx_hash, op.block_number.map(|b| b as i64), op.error,
            op.attempts as i64, op.submitted_at, now()
        ],
    )?;
    Ok(())
}

// ======================== DOMAIN ROWS ========================

#[derive(Debug, Clone, Serialize)]
pub struct FarmerRow {
    pub farmer_id: String,
    pub registered_by: String,
    pub label: String,
    pub region: Option<String>,
    pub primary_crop: Option<String>,
    pub wallet_address: Option<String>,
    pub registration_digest: String,
    pub cid: String,
    pub op_id: String,
    pub created_at: String,
}

fn farmer_from_row(r: &Row) -> rusqlite::Result<FarmerRow> {
    Ok(FarmerRow {
        farmer_id: r.get(0)?,
        registered_by: r.get(1)?,
        label: r.get(2)?,
        region: r.get(3)?,
        primary_crop: r.get(4)?,
        wallet_address: r.get(5)?,
        registration_digest: r.get(6)?,
        cid: r.get(7)?,
        op_id: r.get(8)?,
        created_at: r.get(9)?,
    })
}

const FARMER_COLS: &str =
    "farmer_id, registered_by, label, region, primary_crop, wallet_address, registration_digest, cid, op_id, created_at";

pub fn upsert_farmer(c: &Connection, f: &FarmerRow) -> rusqlite::Result<()> {
    c.execute(
        &format!("INSERT INTO farmers ({FARMER_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(farmer_id) DO NOTHING"),
        params![f.farmer_id, f.registered_by, f.label, f.region, f.primary_crop, f.wallet_address, f.registration_digest, f.cid, f.op_id, f.created_at],
    )?;
    Ok(())
}

pub fn get_farmer(c: &Connection, farmer_id: &str) -> rusqlite::Result<Option<FarmerRow>> {
    c.query_row(
        &format!("SELECT {FARMER_COLS} FROM farmers WHERE farmer_id = ?1"),
        [farmer_id],
        farmer_from_row,
    )
    .optional()
}

pub fn list_farmers(c: &Connection) -> rusqlite::Result<Vec<FarmerRow>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {FARMER_COLS} FROM farmers ORDER BY farmer_id"
    ))?;
    let rows = stmt.query_map([], farmer_from_row)?;
    rows.collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchRow {
    pub batch_id: String,
    pub farmer_id: String,
    pub parent_batch_id: Option<String>,
    pub label: String,
    pub crop: Option<String>,
    pub op_id: String,
    pub created_at: String,
}

fn batch_from_row(r: &Row) -> rusqlite::Result<BatchRow> {
    Ok(BatchRow {
        batch_id: r.get(0)?,
        farmer_id: r.get(1)?,
        parent_batch_id: r.get(2)?,
        label: r.get(3)?,
        crop: r.get(4)?,
        op_id: r.get(5)?,
        created_at: r.get(6)?,
    })
}

const BATCH_COLS: &str = "batch_id, farmer_id, parent_batch_id, label, crop, op_id, created_at";

pub fn upsert_batch(c: &Connection, b: &BatchRow) -> rusqlite::Result<()> {
    c.execute(
        &format!("INSERT INTO batches ({BATCH_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(batch_id) DO NOTHING"),
        params![b.batch_id, b.farmer_id, b.parent_batch_id, b.label, b.crop, b.op_id, b.created_at],
    )?;
    Ok(())
}

pub fn get_batch(c: &Connection, batch_id: &str) -> rusqlite::Result<Option<BatchRow>> {
    c.query_row(
        &format!("SELECT {BATCH_COLS} FROM batches WHERE batch_id = ?1"),
        [batch_id],
        batch_from_row,
    )
    .optional()
}

pub fn list_batches(c: &Connection, farmer_id: Option<&str>) -> rusqlite::Result<Vec<BatchRow>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {BATCH_COLS} FROM batches WHERE (?1 IS NULL OR farmer_id = ?1) ORDER BY created_at, batch_id"
    ))?;
    let rows = stmt.query_map([farmer_id], batch_from_row)?;
    rows.collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordRow {
    pub record_digest: String,
    pub subject_id: String,
    pub record_type: u8,
    pub content_hash: String,
    pub cid: String,
    pub encrypted: bool,
    #[serde(skip)]
    pub public_json: Option<String>,
    pub submitter_id: String,
    pub submitter_address: String,
    pub tx_hash: Option<String>,
    pub block_number: Option<u64>,
    pub op_id: String,
    pub created_at: String,
}

fn record_from_row(r: &Row) -> rusqlite::Result<RecordRow> {
    Ok(RecordRow {
        record_digest: r.get(0)?,
        subject_id: r.get(1)?,
        record_type: r.get::<_, i64>(2)? as u8,
        content_hash: r.get(3)?,
        cid: r.get(4)?,
        encrypted: r.get::<_, i64>(5)? != 0,
        public_json: r.get(6)?,
        submitter_id: r.get(7)?,
        submitter_address: r.get(8)?,
        tx_hash: r.get(9)?,
        block_number: r.get::<_, Option<i64>>(10)?.map(|v| v as u64),
        op_id: r.get(11)?,
        created_at: r.get(12)?,
    })
}

const RECORD_COLS: &str = "record_digest, subject_id, record_type, content_hash, cid, encrypted, public_json, submitter_id, submitter_address, tx_hash, block_number, op_id, created_at";

pub fn upsert_record(c: &Connection, r: &RecordRow) -> rusqlite::Result<()> {
    c.execute(
        &format!("INSERT INTO records ({RECORD_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(record_digest) DO UPDATE SET tx_hash = COALESCE(excluded.tx_hash, records.tx_hash), block_number = COALESCE(excluded.block_number, records.block_number)"),
        params![
            r.record_digest, r.subject_id, r.record_type as i64, r.content_hash, r.cid, r.encrypted as i64,
            r.public_json, r.submitter_id, r.submitter_address, r.tx_hash, r.block_number.map(|b| b as i64),
            r.op_id, r.created_at
        ],
    )?;
    Ok(())
}

pub fn get_record(c: &Connection, digest: &str) -> rusqlite::Result<Option<RecordRow>> {
    c.query_row(
        &format!("SELECT {RECORD_COLS} FROM records WHERE record_digest = ?1"),
        [digest],
        record_from_row,
    )
    .optional()
}

pub fn all_records(c: &Connection) -> rusqlite::Result<Vec<RecordRow>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {RECORD_COLS} FROM records ORDER BY COALESCE(block_number, 0), created_at, record_digest"
    ))?;
    let rows = stmt.query_map([], record_from_row)?;
    rows.collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct SkuRow {
    pub sku_id: String,
    pub batch_id: String,
    pub label: String,
    pub units_root: String,
    pub unit_ids: Vec<String>,
    pub op_id: String,
    pub created_at: String,
}

pub fn upsert_sku(c: &Connection, s: &SkuRow) -> rusqlite::Result<()> {
    let units = serde_json::to_string(&s.unit_ids).expect("strings serialize");
    c.execute(
        "INSERT INTO skus (sku_id, batch_id, label, units_root, unit_ids, op_id, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(sku_id) DO NOTHING",
        params![s.sku_id, s.batch_id, s.label, s.units_root, units, s.op_id, s.created_at],
    )?;
    Ok(())
}

pub fn get_sku(c: &Connection, sku_id: &str) -> rusqlite::Result<Option<SkuRow>> {
    c.query_row(
        "SELECT sku_id, batch_id, label, units_root, unit_ids, op_id, created_at FROM skus WHERE sku_id = ?1",
        [sku_id],
        |r| {
            let units: String = r.get(4)?;
            Ok(SkuRow {
                sku_id: r.get(0)?,
                batch_id: r.get(1)?,
                label: r.get(2)?,
                units_root: r.get(3)?,
                unit_ids: serde_json::from_str(&units).unwrap_or_default(),
                op_id: r.get(5)?,
                created_at: r.get(6)?,
            })
        },
    )
    .optional()
}

#[derive(Debug, Clone, Serialize)]
pub struct FraudRow {
    pub report_digest: String,
    pub batch_id: String,
    pub status: String,
    pub attribution: Option<String>,
    pub resolution_digest: Option<String>,
    pub updated_at: String,
}

pub fn upsert_fraud(c: &Connection, f: &FraudRow) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO fraud_reports (report_digest, batch_id, status, attribution, resolution_digest, updated_at) VALUES (?1,?2,?3,?4,?5,?6)
         ON CONFLICT(report_digest) DO UPDATE SET status = excluded.status, attribution = excluded.attribution, resolution_digest = excluded.resolution_digest, updated_at = excluded.updated_at",
        params![f.report_digest, f.batch_id, f.status, f.attribution, f.resolution_digest, f.updated_at],
    )?;
    Ok(())
}

pub fn get_fraud(c: &Connection, digest: &str) -> rusqlite::Result<Option<FraudRow>> {
    c.query_row(
        "SELECT report_digest, batch_id, status, attribution, resolution_digest, updated_at FROM fraud_reports WHERE report_digest = ?1",
        [digest],
        |r| {
            Ok(FraudRow {
                report_digest: r.get(0)?,
                batch_id: r.get(1)?,
                status: r.get(2)?,
                attribution: r.get(3)?,
                resolution_digest: r.get(4)?,
                updated_at: r.get(5)?,
            })
        },
    )
    .optional()
}

pub fn all_fraud(c: &Connection) -> rusqlite::Result<Vec<FraudRow>> {
    let mut stmt = c.prepare(
        "SELECT report_digest, batch_id, status, attribution, resolution_digest, updated_at FROM fraud_reports ORDER BY report_digest",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(FraudRow {
            report_digest: r.get(0)?,
            batch_id: r.get(1)?,
            status: r.get(2)?,
            attribution: r.get(3)?,
            resolution_digest: r.get(4)?,
            updated_at: r.get(5)?,
        })
    })?;
    rows.collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct AiCommitRow {
    pub commit_hash: String,
    pub batch_id: String,
    pub oracle_id: String,
    #[serde(skip)]
    pub sealed_payload: String,
    #[serde(skip)]
    pub sealed_salt: String,
    pub content_hash: String,
    pub status: String,
    pub committed_at: Option<i64>,
    pub commit_op_id: String,
    pub reveal_op_id: Option<String>,
    pub record_digest: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

const AI_COLS: &str = "commit_hash, batch_id, oracle_id, sealed_payload, sealed_salt, content_hash, status, committed_at, commit_op_id, reveal_op_id, record_digest, created_at, updated_at";

fn ai_from_row(r: &Row) -> rusqlite::Result<AiCommitRow> {
    Ok(AiCommitRow {
        commit_hash: r.get(0)?,
        batch_id: r.get(1)?,
        oracle_id: r.get(2)?,
        sealed_payload: r.get(3)?,
        sealed_salt: r.get(4)?,
        content_hash: r.get(5)?,
        status: r.get(6)?,
        committed_at: r.get(7)?,
        commit_op_id: r.get(8)?,
        reveal_op_id: r.get(9)?,
        record_digest: r.get(10)?,
        created_at: r.get(11)?,
        updated_at: r.get(12)?,
    })
}

pub fn insert_ai_commit(c: &Connection, a: &AiCommitRow) -> rusqlite::Result<()> {
    c.execute(
        &format!("INSERT INTO ai_commits ({AI_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(commit_hash) DO NOTHING"),
        params![
            a.commit_hash, a.batch_id, a.oracle_id, a.sealed_payload, a.sealed_salt, a.content_hash, a.status,
            a.committed_at, a.commit_op_id, a.reveal_op_id, a.record_digest, a.created_at, a.updated_at
        ],
    )?;
    Ok(())
}

pub fn update_ai_commit(c: &Connection, a: &AiCommitRow) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE ai_commits SET status=?2, committed_at=?3, reveal_op_id=?4, record_digest=?5, updated_at=?6 WHERE commit_hash=?1",
        params![a.commit_hash, a.status, a.committed_at, a.reveal_op_id, a.record_digest, now()],
    )?;
    Ok(())
}

pub fn ai_commit_by_op(c: &Connection, op_id: &str) -> rusqlite::Result<Option<AiCommitRow>> {
    c.query_row(
        &format!("SELECT {AI_COLS} FROM ai_commits WHERE commit_op_id = ?1"),
        [op_id],
        ai_from_row,
    )
    .optional()
}

pub fn latest_ai_commit_for_batch(
    c: &Connection,
    batch_id: &str,
) -> rusqlite::Result<Option<AiCommitRow>> {
    c.query_row(
        &format!(
            "SELECT {AI_COLS} FROM ai_commits WHERE batch_id = ?1 ORDER BY created_at DESC LIMIT 1"
        ),
        [batch_id],
        ai_from_row,
    )
    .optional()
}

pub fn ai_commits_with_status(c: &Connection, status: &str) -> rusqlite::Result<Vec<AiCommitRow>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {AI_COLS} FROM ai_commits WHERE status = ?1 ORDER BY created_at"
    ))?;
    let rows = stmt.query_map([status], ai_from_row)?;
    rows.collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct NftRow {
    pub kind: String,
    pub source_ref: String,
    pub token_id: Option<u64>,
    pub contract_address: String,
    pub chain_id: u64,
    pub batch_id: Option<String>,
    pub farmer_id: Option<String>,
    pub owner: Option<String>,
    pub token_uri: String,
    pub metadata_cid: String,
    pub op_id: String,
    pub status: String,
    pub tx_hash: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

const NFT_COLS: &str = "kind, source_ref, token_id, contract_address, chain_id, batch_id, farmer_id, owner, token_uri, metadata_cid, op_id, status, tx_hash, created_at, updated_at";

fn nft_from_row(r: &Row) -> rusqlite::Result<NftRow> {
    Ok(NftRow {
        kind: r.get(0)?,
        source_ref: r.get(1)?,
        token_id: r.get::<_, Option<i64>>(2)?.map(|v| v as u64),
        contract_address: r.get(3)?,
        chain_id: r.get::<_, i64>(4)? as u64,
        batch_id: r.get(5)?,
        farmer_id: r.get(6)?,
        owner: r.get(7)?,
        token_uri: r.get(8)?,
        metadata_cid: r.get(9)?,
        op_id: r.get(10)?,
        status: r.get(11)?,
        tx_hash: r.get(12)?,
        created_at: r.get(13)?,
        updated_at: r.get(14)?,
    })
}

pub fn upsert_nft(c: &Connection, n: &NftRow) -> rusqlite::Result<()> {
    c.execute(
        &format!("INSERT INTO nfts ({NFT_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
         ON CONFLICT(kind, source_ref) DO UPDATE SET token_id = excluded.token_id, owner = excluded.owner, status = excluded.status, tx_hash = excluded.tx_hash, updated_at = excluded.updated_at"),
        params![
            n.kind, n.source_ref, n.token_id.map(|t| t as i64), n.contract_address, n.chain_id as i64, n.batch_id,
            n.farmer_id, n.owner, n.token_uri, n.metadata_cid, n.op_id, n.status, n.tx_hash, n.created_at, n.updated_at
        ],
    )?;
    Ok(())
}

pub fn get_nft(c: &Connection, kind: &str, source_ref: &str) -> rusqlite::Result<Option<NftRow>> {
    c.query_row(
        &format!("SELECT {NFT_COLS} FROM nfts WHERE kind = ?1 AND source_ref = ?2"),
        params![kind, source_ref],
        nft_from_row,
    )
    .optional()
}

pub fn nft_by_token(c: &Connection, kind: &str, token_id: u64) -> rusqlite::Result<Option<NftRow>> {
    c.query_row(
        &format!("SELECT {NFT_COLS} FROM nfts WHERE kind = ?1 AND token_id = ?2"),
        params![kind, token_id as i64],
        nft_from_row,
    )
    .optional()
}

pub fn list_nfts(c: &Connection) -> rusqlite::Result<Vec<NftRow>> {
    let mut stmt = c.prepare(&format!("SELECT {NFT_COLS} FROM nfts ORDER BY created_at"))?;
    let rows = stmt.query_map([], nft_from_row)?;
    rows.collect()
}

// ======================== PIN JOBS ========================

#[derive(Debug, Clone, Serialize)]
pub struct PinJob {
    pub cid: String,
    pub target: String,
    pub status: String,
    pub attempts: u32,
    pub next_attempt_at: i64,
    #[serde(skip)]
    pub request_id: Option<String>,
    pub last_error: Option<String>,
    pub verified_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

const PIN_COLS: &str =
    "cid, target, status, attempts, next_attempt_at, request_id, last_error, verified_at, created_at, updated_at";

fn pin_from_row(r: &Row) -> rusqlite::Result<PinJob> {
    Ok(PinJob {
        cid: r.get(0)?,
        target: r.get(1)?,
        status: r.get(2)?,
        attempts: r.get::<_, i64>(3)? as u32,
        next_attempt_at: r.get(4)?,
        request_id: r.get(5)?,
        last_error: r.get(6)?,
        verified_at: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

/// Queue `cid` for every target; existing jobs are left untouched (idempotent).
pub fn enqueue_pins(
    c: &Connection,
    cid: &str,
    targets: &[String],
    now_ts: i64,
) -> rusqlite::Result<usize> {
    let mut n = 0;
    for t in targets {
        n += c.execute(
            &format!("INSERT INTO pin_jobs ({PIN_COLS}) VALUES (?1,?2,'pending',0,?3,NULL,NULL,NULL,?4,?4) ON CONFLICT(cid, target) DO NOTHING"),
            params![cid, t, now_ts, now()],
        )?;
    }
    Ok(n)
}

pub fn due_pin_jobs(c: &Connection, now_ts: i64, limit: usize) -> rusqlite::Result<Vec<PinJob>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {PIN_COLS} FROM pin_jobs WHERE status IN ('pending','remote_pending') AND next_attempt_at <= ?1 ORDER BY next_attempt_at, cid LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![now_ts, limit as i64], pin_from_row)?;
    rows.collect()
}

pub fn update_pin_job(c: &Connection, j: &PinJob) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE pin_jobs SET status=?3, attempts=?4, next_attempt_at=?5, request_id=?6, last_error=?7, verified_at=?8, updated_at=?9 WHERE cid=?1 AND target=?2",
        params![j.cid, j.target, j.status, j.attempts as i64, j.next_attempt_at, j.request_id, j.last_error, j.verified_at, now()],
    )?;
    Ok(())
}

pub fn pins_for_cid(c: &Connection, cid: &str) -> rusqlite::Result<Vec<PinJob>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {PIN_COLS} FROM pin_jobs WHERE cid = ?1 ORDER BY target"
    ))?;
    let rows = stmt.query_map([cid], pin_from_row)?;
    rows.collect()
}

pub fn pin_jobs_with_status(
    c: &Connection,
    status: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<PinJob>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {PIN_COLS} FROM pin_jobs WHERE (?1 IS NULL OR status = ?1) ORDER BY updated_at DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![status, limit as i64], pin_from_row)?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(idem: &str) -> Operation {
        Operation {
            id: new_id("op"),
            idem_key: idem.into(),
            actor_id: "p1".into(),
            kind: "procurement".into(),
            request_hash: "h".into(),
            status: OpStatus::Prepared,
            subject_id: None,
            record_type: Some(2),
            content_hash: None,
            record_digest: None,
            cid: None,
            payload: Some(vec![1, 2, 3]),
            encrypted: false,
            public_json: None,
            action: "{}".into(),
            context: "{}".into(),
            tx_hash: None,
            block_number: None,
            error: None,
            attempts: 0,
            submitted_at: None,
            created_at: now(),
            updated_at: now(),
        }
    }

    #[tokio::test]
    async fn idempotency_key_returns_existing_operation() {
        let db = Db::open_in_memory().unwrap();
        let first = op("key-1");
        let first_id = first.id.clone();
        let (stored, new) = db
            .call(move |c| insert_or_get_operation(c, &first))
            .await
            .unwrap();
        assert!(new);
        assert_eq!(stored.id, first_id);

        let retry = op("key-1");
        let (again, new) = db
            .call(move |c| insert_or_get_operation(c, &retry))
            .await
            .unwrap();
        assert!(!new);
        assert_eq!(
            again.id, first_id,
            "retry must resolve to the original operation"
        );

        let other = op("key-2");
        let (_, new) = db
            .call(move |c| insert_or_get_operation(c, &other))
            .await
            .unwrap();
        assert!(new);
    }

    #[tokio::test]
    async fn migrations_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        Db::open(&path).unwrap();
        Db::open(&path).unwrap();
    }
}
