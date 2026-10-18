//! Background recovery loop:
//! * checks receipts of `submitted` operations (slow confirmations, dropped transactions);
//! * resumes `prepared` / `stored` operations whose request was interrupted (IPFS or RPC outage);
//! * re-runs failed database finalization of confirmed operations;
//! * reveals committed AI assessments once their window opens (chain time), and marks expired ones;
//! * replicates stored blocks to backup pinning targets (`pinning.rs`).

use crate::db::{self, OpStatus};
use crate::ops;
use crate::state::AppState;
use crate::supply_chain_handlers::{reveal_commit, MAX_REVEAL_DELAY, MIN_REVEAL_DELAY};
use std::time::Duration;

/// Operations younger than this are assumed to still be handled by their HTTP request.
const RESUME_AFTER_SECS: i64 = 30;

pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(state.config.reconcile_interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if let Err(e) = run_once(&state).await {
                tracing::warn!(error = %e, "reconciler pass failed");
            }
        }
    })
}

pub async fn run_once(state: &AppState) -> Result<(), crate::error::ApiError> {
    let ops_list = state
        .db
        .call(|c| {
            db::operations_with_status(
                c,
                &[OpStatus::Prepared, OpStatus::Stored, OpStatus::Submitted],
            )
        })
        .await?;
    let now = chrono::Utc::now();
    for op in ops_list {
        let age = chrono::DateTime::parse_from_rfc3339(&op.updated_at)
            .map(|t| (now - t.with_timezone(&chrono::Utc)).num_seconds())
            .unwrap_or(i64::MAX);
        if op.status != OpStatus::Submitted && age < RESUME_AFTER_SECS {
            continue;
        }
        let id = op.id.clone();
        match ops::drive(state, op, Duration::ZERO).await {
            Ok(op) if op.status != OpStatus::Submitted => {
                tracing::info!(op = %id, status = op.status.as_str(), "reconciled operation")
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(op = %id, error = %e, "operation still blocked"),
        }
    }

    let needs_finalize = state
        .db
        .call(|c| db::operations_with_status(c, &[OpStatus::Confirmed]))
        .await?
        .into_iter()
        .filter(|o| {
            o.error
                .as_deref()
                .is_some_and(|e| e.starts_with("finalize"))
        });
    for mut op in needs_finalize {
        ops::finalize(state, &mut op).await;
    }

    let committed = state
        .db
        .call(|c| db::ai_commits_with_status(c, "committed"))
        .await?;
    if !committed.is_empty() {
        let chain_now = state
            .chain_client
            .latest_timestamp()
            .await
            .map_err(|e| crate::error::ApiError::Chain(e.to_string()))?
            as i64;
        for mut row in committed {
            let Some(at) = row.committed_at else { continue };
            if chain_now > at + MAX_REVEAL_DELAY {
                tracing::warn!(commit = %row.commit_hash, "AI commitment expired before reveal");
                row.status = "expired".into();
                state
                    .db
                    .call(move |c| db::update_ai_commit(c, &row))
                    .await?;
            } else if state.config.ai_auto_reveal && chain_now >= at + MIN_REVEAL_DELAY {
                match reveal_commit(state, &row).await {
                    Ok(op) => {
                        tracing::info!(commit = %row.commit_hash, status = op.status.as_str(), "auto-reveal")
                    }
                    Err(e) => {
                        tracing::warn!(commit = %row.commit_hash, error = %e, "auto-reveal failed")
                    }
                }
            }
        }
    }
    match crate::pinning::run_once(state).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(jobs = n, "backup pinning pass"),
        Err(e) => tracing::warn!(error = %e, "backup pinning pass failed"),
    }
    Ok(())
}
