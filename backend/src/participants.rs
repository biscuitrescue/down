//! Participant onboarding. Each participant gets its own custodial wallet (key sealed with
//! KEY_ENCRYPTION_KEY) and API token; the admin grants its on-chain roles.

use crate::auth::Actor;
use crate::chain::{hexaddr, roles, ChainAction};
use crate::crypto;
use crate::db::{self, Participant};
use crate::error::ApiError;
use crate::ops::{self, NewOperation, OperationView};
use crate::state::AppState;
use alloy::primitives::U256;
use alloy::signers::local::PrivateKeySigner;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateParticipantRequest {
    pub name: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ParticipantView {
    pub id: String,
    pub name: String,
    pub roles: Vec<&'static str>,
    pub address: String,
    pub created_at: String,
}

impl From<&Participant> for ParticipantView {
    fn from(p: &Participant) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            roles: roles::names(p.roles),
            address: p.address.clone(),
            created_at: p.created_at.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CreateParticipantResponse {
    pub participant: ParticipantView,
    /// Shown exactly once; only its hash is stored.
    pub api_token: String,
    pub operations: Vec<OperationView>,
}

/// POST /api/admin/participants
pub async fn create_participant(
    State(state): State<AppState>,
    actor: Actor,
    Json(req): Json<CreateParticipantRequest>,
) -> Result<Response, ApiError> {
    actor.require_admin()?;
    let name = req.name.trim().to_string();
    if name.is_empty() || name.len() > 80 {
        return Err(ApiError::bad_request("name must be 1-80 characters"));
    }
    if req.roles.is_empty() {
        return Err(ApiError::bad_request("at least one role is required"));
    }
    let mask = roles::parse(&req.roles).map_err(ApiError::BadRequest)?;
    if mask & roles::ADMIN != 0 {
        return Err(ApiError::bad_request(
            "admin access is configured through ADMIN_API_TOKEN, not granted via the API",
        ));
    }

    let signer = PrivateKeySigner::random();
    let id = db::new_id("p");
    let token = crypto::new_api_token();
    let key_hex = format!("0x{}", hex::encode(signer.to_bytes()));
    let participant = Participant {
        id: id.clone(),
        name,
        roles: mask,
        address: hexaddr(signer.address()),
        encrypted_key: crypto::seal(
            &state.config.secrets.key_encryption_key,
            key_hex.as_bytes(),
            &crate::auth::wallet_aad(&id),
        ),
        token_hash: crypto::token_hash(&token),
        created_at: db::now(),
    };
    let row = participant.clone();
    state
        .db
        .call(move |c| db::insert_participant(c, &row))
        .await?;
    tracing::info!(participant = %id, address = %participant.address, roles = ?roles::names(mask), "participant created");

    let admin = Actor::admin(&state);
    let mut operations = Vec::new();
    let grant = ops::submit(
        &state,
        &admin,
        NewOperation {
            idem_key: format!("participant:{id}:grant"),
            kind: "grant_role",
            request_hash: format!("grant:{id}:{mask}"),
            record: None,
            action: ChainAction::GrantRole {
                account: participant.address.clone(),
                role: mask,
            },
            context: json!({"result": {"participant_id": id}}),
        },
    )
    .await?;
    let granted = grant.status == db::OpStatus::Confirmed;
    operations.push(ops::view(&state, &grant));

    if state.chain_client.funding_wei > U256::ZERO {
        let fund = ops::submit(
            &state,
            &admin,
            NewOperation {
                idem_key: format!("participant:{id}:fund"),
                kind: "fund",
                request_hash: format!("fund:{id}"),
                record: None,
                action: ChainAction::Fund {
                    to: participant.address.clone(),
                    wei: state.chain_client.funding_wei.to_string(),
                },
                context: json!({"result": {"participant_id": id}}),
            },
        )
        .await?;
        operations.push(ops::view(&state, &fund));
    }

    let status = if granted {
        StatusCode::CREATED
    } else {
        StatusCode::ACCEPTED
    };
    Ok((
        status,
        Json(CreateParticipantResponse {
            participant: ParticipantView::from(&participant),
            api_token: token,
            operations,
        }),
    )
        .into_response())
}

/// GET /api/participants — directory for choosing custody recipients. No secrets.
pub async fn list_participants(
    State(state): State<AppState>,
    actor: Actor,
) -> Result<Json<Vec<ParticipantView>>, ApiError> {
    if !actor.is_admin {
        actor.require_any_role()?;
    }
    let list = state.db.call(db::list_participants).await?;
    Ok(Json(list.iter().map(ParticipantView::from).collect()))
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub id: String,
    pub name: String,
    pub is_admin: bool,
    pub address: String,
    pub roles: Vec<&'static str>,
    /// Roles actually held on-chain right now (may lag while a grant is pending).
    pub onchain_roles: Vec<&'static str>,
    pub balance_wei: String,
}

/// GET /api/me
pub async fn me(State(state): State<AppState>, actor: Actor) -> Result<Json<MeResponse>, ApiError> {
    let chain = &state.chain_client;
    let onchain = chain
        .registry()
        .getRoles(actor.address)
        .call()
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    let onchain_mask: u64 = onchain.try_into().unwrap_or(0);
    let balance = alloy::providers::Provider::get_balance(chain.provider(), actor.address)
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    Ok(Json(MeResponse {
        id: actor.id.clone(),
        name: actor.name.clone(),
        is_admin: actor.is_admin,
        address: hexaddr(actor.address),
        roles: roles::names(actor.roles),
        onchain_roles: roles::names(onchain_mask),
        balance_wei: balance.to_string(),
    }))
}
