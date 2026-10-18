//! Backup pinning: every block this server stores on its primary IPFS node is queued (SQLite, survives
//! restarts) for replication to each configured backup target.
//!
//! * `kubo:<url>` — another Kubo-compatible node. The worker reads the block from the primary (CID-checked),
//!   pushes the same bytes with `block/put` (raw, sha2-256, pinned), requires the identical CID back, then reads
//!   the block from the backup and checks it again. Backups never mint new CIDs.
//! * `psa:<url>` — an [IPFS Pinning Service API](https://ipfs.github.io/pinning-services-api-spec/) provider
//!   (Pinata, Filebase, …). The provider fetches the CID from the network itself, so the primary node must be
//!   reachable by it (not the case for the offline dev node). Status is polled until `pinned` or `failed`.
//!
//! Failures back off exponentially and stop after `IPFS_BACKUP_MAX_ATTEMPTS` (status `failed`, visible in
//! `GET /api/admin/pins?status=failed` and in each record's `replicas`).

use crate::auth::Actor;
use crate::cid::Cid;
use crate::config::{IpfsBackupConfig, PinningServiceConfig};
use crate::db::{self, PinJob};
use crate::error::ApiError;
use crate::ipfs::{IpfsClient, IpfsError};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

const REMOTE_POLL_SECS: i64 = 30;
const MAX_BACKOFF_SECS: i64 = 3600;
const BATCH: usize = 25;

pub enum BackupTarget {
    Kubo {
        name: String,
        client: IpfsClient,
    },
    PinningService {
        name: String,
        client: PinningServiceClient,
    },
}

impl BackupTarget {
    pub fn name(&self) -> &str {
        match self {
            Self::Kubo { name, .. } | Self::PinningService { name, .. } => name,
        }
    }
}

pub fn build_targets(cfg: &IpfsBackupConfig) -> anyhow::Result<Vec<BackupTarget>> {
    let mut targets = Vec::new();
    for k in &cfg.kubo {
        targets.push(BackupTarget::Kubo {
            name: format!("kubo:{}", k.api_url),
            client: IpfsClient::new(k.clone())?,
        });
    }
    if let Some(p) = &cfg.pinning_service {
        targets.push(BackupTarget::PinningService {
            name: format!("psa:{}", p.url),
            client: PinningServiceClient::new(p.clone())?,
        });
    }
    Ok(targets)
}

// ======================== PINNING SERVICE API CLIENT ========================

#[derive(Debug, Clone, Deserialize)]
pub struct PinStatus {
    pub requestid: String,
    /// `queued` | `pinning` | `pinned` | `failed`
    pub status: String,
}

pub struct PinningServiceClient {
    http: reqwest::Client,
    config: PinningServiceConfig,
}

impl PinningServiceClient {
    pub fn new(config: PinningServiceConfig) -> anyhow::Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder().timeout(config.timeout).build()?,
            config,
        })
    }

    async fn parse(resp: reqwest::Response) -> Result<PinStatus, IpfsError> {
        let status = resp.status();
        if !status.is_success() {
            let body: String = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(300)
                .collect();
            return Err(IpfsError::Api {
                status: status.as_u16(),
                body,
            });
        }
        resp.json()
            .await
            .map_err(|e| IpfsError::BadResponse(e.to_string()))
    }

    /// `POST /pins` — ask the provider to pin `cid`.
    pub async fn add(&self, cid: &str, name: &str) -> Result<PinStatus, IpfsError> {
        let resp = self
            .http
            .post(format!("{}/pins", self.config.url))
            .bearer_auth(&self.config.token)
            .json(&json!({"cid": cid, "name": name}))
            .send()
            .await
            .map_err(|e| IpfsError::Unavailable(e.to_string()))?;
        Self::parse(resp).await
    }

    /// `GET /pins/{requestid}`
    pub async fn status(&self, request_id: &str) -> Result<PinStatus, IpfsError> {
        let resp = self
            .http
            .get(format!("{}/pins/{request_id}", self.config.url))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .map_err(|e| IpfsError::Unavailable(e.to_string()))?;
        Self::parse(resp).await
    }
}

// ======================== QUEUE ========================

/// Queue a stored block for every backup target. Never fails the caller's operation: a queue error is logged
/// and the block simply stays single-homed until the next store of the same CID.
pub async fn enqueue(state: &AppState, cid: &Cid) {
    if state.backups.is_empty() {
        return;
    }
    let targets: Vec<String> = state.backups.iter().map(|t| t.name().to_string()).collect();
    let cid = cid.to_string();
    let now = chrono::Utc::now().timestamp();
    if let Err(e) = state
        .db
        .call(move |c| db::enqueue_pins(c, &cid, &targets, now))
        .await
    {
        tracing::error!(error = %e, "could not queue backup pins");
    }
}

fn backoff(attempts: u32) -> i64 {
    (30i64 << attempts.min(10)).min(MAX_BACKOFF_SECS)
}

/// Process due jobs once. Returns how many jobs were attempted.
pub async fn run_once(state: &AppState) -> Result<usize, ApiError> {
    if state.backups.is_empty() {
        return Ok(0);
    }
    let now = chrono::Utc::now().timestamp();
    let jobs = state
        .db
        .call(move |c| db::due_pin_jobs(c, now, BATCH))
        .await?;
    let n = jobs.len();
    for job in jobs {
        let Some(target) = state.backups.iter().find(|t| t.name() == job.target) else {
            // Target removed from configuration: leave the job as is.
            continue;
        };
        let updated = process(state, target, job).await;
        state
            .db
            .call(move |c| db::update_pin_job(c, &updated))
            .await?;
    }
    Ok(n)
}

async fn process(state: &AppState, target: &BackupTarget, mut job: PinJob) -> PinJob {
    let now = chrono::Utc::now().timestamp();
    let max = state.config.ipfs_backup.max_attempts.max(1);
    let fail = |mut job: PinJob, err: String| {
        job.attempts += 1;
        job.last_error = Some(err);
        if job.attempts >= max {
            job.status = "failed".into();
        } else {
            job.status = "pending".into();
            job.next_attempt_at = now + backoff(job.attempts);
        }
        job
    };
    let cid = match Cid::parse(&job.cid) {
        Ok(c) => c,
        Err(e) => {
            job.status = "failed".into();
            job.last_error = Some(format!("invalid CID: {e}"));
            return job;
        }
    };
    match target {
        BackupTarget::Kubo { client, .. } => {
            let bytes = match state.ipfs_client.get_block(&cid).await {
                Ok(b) => b,
                Err(e) => return fail(job, format!("read from primary: {e}")),
            };
            // put_block computes the CID locally and rejects a node that answers with a different one.
            if let Err(e) = client.put_block(&bytes).await {
                return fail(job, format!("push to backup: {e}"));
            }
            // Read back from the backup; get_block checks sha256 against the CID.
            if let Err(e) = client.get_block(&cid).await {
                return fail(job, format!("read back from backup: {e}"));
            }
            job.status = "pinned".into();
            job.last_error = None;
            job.verified_at = Some(db::now());
            tracing::info!(cid = %job.cid, target = %job.target, "backup pin verified");
            job
        }
        BackupTarget::PinningService { client, .. } => {
            let result = match &job.request_id {
                None => {
                    client
                        .add(&job.cid, &format!("oilschain-{}", job.cid))
                        .await
                }
                Some(id) => client.status(id).await,
            };
            match result {
                Ok(s) => match s.status.as_str() {
                    "pinned" => {
                        job.status = "pinned".into();
                        job.request_id = Some(s.requestid);
                        job.last_error = None;
                        // Reported by the provider; content is not re-downloaded from it.
                        job.verified_at = Some(db::now());
                        job
                    }
                    "failed" => {
                        job.request_id = None; // re-submit on the next attempt
                        fail(job, "pinning service reported `failed`".into())
                    }
                    _ => {
                        job.status = "remote_pending".into();
                        job.request_id = Some(s.requestid);
                        job.next_attempt_at = now + REMOTE_POLL_SECS;
                        job
                    }
                },
                Err(e) => fail(job, format!("pinning service: {e}")),
            }
        }
    }
}

// ======================== HTTP ========================

#[derive(Debug, Serialize)]
pub struct PinsView {
    pub cid: String,
    pub primary: &'static str,
    pub backups: Vec<PinJob>,
    pub configured_targets: Vec<String>,
}

/// GET /api/ipfs/pins/{cid} (public) — replication state of one block.
pub async fn pins_for_cid(
    State(state): State<AppState>,
    Path(cid): Path<String>,
) -> Result<Json<PinsView>, ApiError> {
    Cid::parse(&cid).map_err(|e| ApiError::BadRequest(format!("cid: {e}")))?;
    let c = cid.clone();
    let backups = state.db.call(move |db| db::pins_for_cid(db, &c)).await?;
    Ok(Json(PinsView {
        cid,
        primary: "pinned on this server's IPFS node at upload time",
        backups,
        configured_targets: state.backups.iter().map(|t| t.name().to_string()).collect(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct PinListQuery {
    pub status: Option<String>,
    pub limit: Option<usize>,
}

/// GET /api/admin/pins?status=failed (admin)
pub async fn list_pins(
    State(state): State<AppState>,
    actor: Actor,
    Query(q): Query<PinListQuery>,
) -> Result<Json<Value>, ApiError> {
    actor.require_admin()?;
    if let Some(s) = &q.status {
        if !["pending", "remote_pending", "pinned", "failed"].contains(&s.as_str()) {
            return Err(ApiError::bad_request(
                "status must be pending, remote_pending, pinned or failed",
            ));
        }
    }
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let status = q.status.clone();
    let jobs = state
        .db
        .call(move |c| db::pin_jobs_with_status(c, status.as_deref(), limit))
        .await?;
    Ok(Json(json!({
        "targets": state.backups.iter().map(|t| t.name().to_string()).collect::<Vec<_>>(),
        "jobs": jobs,
    })))
}

/// For tests and the reconciler: wait-free single pass helper with a short delay between passes.
pub async fn drain(state: &AppState, passes: usize) -> Result<(), ApiError> {
    for _ in 0..passes {
        if run_once(state).await? == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}
