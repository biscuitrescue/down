//! Bearer-token authentication. The admin token comes from `ADMIN_API_TOKEN`; participant tokens are issued
//! once at creation and stored as SHA-256 hashes. Each participant signs with its own custodial key, so the
//! contract's role and custody checks apply per participant rather than to one shared backend key.

use crate::chain::roles;
use crate::crypto;
use crate::db;
use crate::error::ApiError;
use crate::state::AppState;
use alloy::primitives::Address;
use alloy::signers::local::PrivateKeySigner;
use axum::extract::{FromRequestParts, OptionalFromRequestParts};
use axum::http::request::Parts;

pub const ADMIN_ACTOR_ID: &str = "admin";

#[derive(Debug, Clone)]
pub struct Actor {
    pub id: String,
    pub name: String,
    pub address: Address,
    pub roles: u64,
    pub is_admin: bool,
    sealed_key: Option<String>,
}

impl Actor {
    pub fn require_role(&self, mask: u64, what: &str) -> Result<(), ApiError> {
        if self.roles & mask == 0 {
            return Err(ApiError::Forbidden(format!(
                "{what} requires role {:?}; participant has {:?}",
                roles::names(mask),
                roles::names(self.roles)
            )));
        }
        Ok(())
    }

    pub fn require_admin(&self) -> Result<(), ApiError> {
        if !self.is_admin {
            return Err(ApiError::Forbidden("admin token required".into()));
        }
        Ok(())
    }

    pub fn require_any_role(&self) -> Result<(), ApiError> {
        self.require_role(roles::ALL, "this operation")
    }

    /// The key this actor signs transactions with.
    pub fn signer(&self, state: &AppState) -> Result<PrivateKeySigner, ApiError> {
        if self.is_admin {
            return Ok(state.chain_client.admin_signer());
        }
        let sealed = self
            .sealed_key
            .as_deref()
            .ok_or_else(|| ApiError::internal("participant has no key"))?;
        let key = crypto::open(
            &state.config.secrets.key_encryption_key,
            sealed,
            &wallet_aad(&self.id),
        )
        .map_err(|e| ApiError::internal(format!("cannot unseal participant key: {e}")))?;
        let key = String::from_utf8(key)
            .map_err(|_| ApiError::internal("participant key is not UTF-8"))?;
        key.parse()
            .map_err(|_| ApiError::internal("participant key is invalid"))
    }

    pub fn admin(state: &AppState) -> Self {
        Actor {
            id: ADMIN_ACTOR_ID.into(),
            name: "admin".into(),
            address: state.chain_client.admin_address(),
            roles: roles::ADMIN,
            is_admin: true,
            sealed_key: None,
        }
    }

    pub fn from_participant(p: &db::Participant) -> Result<Self, ApiError> {
        Ok(Actor {
            id: p.id.clone(),
            name: p.name.clone(),
            address: p
                .address
                .parse()
                .map_err(|_| ApiError::internal("stored participant address is invalid"))?,
            roles: p.roles,
            is_admin: false,
            sealed_key: Some(p.encrypted_key.clone()),
        })
    }
}

pub fn wallet_aad(participant_id: &str) -> String {
    format!("oilschain:wallet:{participant_id}")
}

fn bearer(parts: &Parts) -> Option<Result<String, ApiError>> {
    let value = parts.headers.get(axum::http::header::AUTHORIZATION)?;
    Some(
        value
            .to_str()
            .ok()
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .ok_or(ApiError::Unauthenticated),
    )
}

async fn resolve(token: String, state: &AppState) -> Result<Actor, ApiError> {
    let admin = &state.config.secrets.admin_api_token;
    if crypto::constant_time_eq(token.as_bytes(), admin.as_bytes()) {
        return Ok(Actor::admin(state));
    }
    let hash = crypto::token_hash(&token);
    let participant = state
        .db
        .call(move |c| db::participant_by_token_hash(c, &hash))
        .await?
        .ok_or(ApiError::Unauthenticated)?;
    Actor::from_participant(&participant)
}

impl FromRequestParts<AppState> for Actor {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer(parts).ok_or(ApiError::Unauthenticated)??;
        resolve(token, state).await
    }
}

impl OptionalFromRequestParts<AppState> for Actor {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Option<Self>, Self::Rejection> {
        match bearer(parts) {
            None => Ok(None),
            Some(token) => resolve(token?, state).await.map(Some),
        }
    }
}
