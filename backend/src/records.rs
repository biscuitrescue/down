//! Record preparation: build → split private fields → canonicalize → hash → encrypt → CID, all before anything
//! leaves the server.
//!
//! For record types that declare private fields (`RecordType::private_fields`), the stored block is
//!
//! ```text
//! committed record = { envelope…, data: {public fields}, private_commitment: keccak256(canonical(private)) }
//! stored block     = committed record + { private_ciphertext: AES-256-GCM(canonical(private)) }
//! contentHash      = keccak256(canonical(committed record))
//! ```
//!
//! where `private = {salt, <private fields>}`. Anyone can verify the public fields against the on-chain
//! commitment; only the key holder can open and check the private part. The salt stops dictionary attacks on
//! the commitment of low-entropy personal data.

use crate::canonical::{self, RecordType, PRIVATE_CIPHERTEXT_FIELD, PRIVATE_COMMITMENT_FIELD};
use crate::cid::Cid;
use crate::crypto;
use crate::error::ApiError;
use crate::ipfs::MAX_BLOCK_SIZE;
use crate::state::AppState;
use alloy::primitives::{Address, B256};
use serde_json::{Map, Value};

#[derive(Debug, Clone)]
pub struct PreparedRecord {
    pub record_type: RecordType,
    pub subject_id: B256,
    pub content_hash: B256,
    pub record_digest: B256,
    /// Exact bytes uploaded to IPFS (canonical JSON; includes the private ciphertext when there is one).
    pub payload: Vec<u8>,
    pub cid: Cid,
    /// Whether the record has an encrypted private part.
    pub encrypted: bool,
    /// Canonical committed record (public fields + private commitment). Safe to cache and display.
    pub public_json: Option<String>,
}

/// AAD of legacy whole-record envelopes (records written before field-level encryption).
pub fn envelope_aad(record_type: RecordType, subject_id: B256) -> String {
    format!("{}:{subject_id:#x}", record_type.name())
}

/// Move the type's declared private fields out of `data`. Returns `(public data, private fields)`.
pub fn split_private(
    record_type: RecordType,
    data: Value,
) -> Result<(Value, Map<String, Value>), ApiError> {
    let Value::Object(mut public) = data else {
        return Err(ApiError::internal("record data must be an object"));
    };
    let mut private = Map::new();
    for key in record_type.private_fields() {
        if let Some(v) = public.remove(*key) {
            private.insert((*key).to_string(), v);
        }
    }
    Ok((Value::Object(public), private))
}

/// Prepare a record whose `data` was already validated by the caller.
pub fn prepare(
    state: &AppState,
    record_type: RecordType,
    subject_id: B256,
    submitter: Address,
    data: Value,
) -> Result<PreparedRecord, ApiError> {
    let (public, mut private) = split_private(record_type, data)?;
    let mut record =
        canonical::build_record(state.domain(), record_type, subject_id, submitter, public);
    if private.is_empty() {
        let bytes =
            canonical::canonical_json(&record).map_err(|e| ApiError::BadRequest(e.to_string()))?;
        return prepare_from_canonical(state, record_type, subject_id, bytes);
    }

    let salt: [u8; 16] = rand::random();
    private.insert("salt".into(), format!("0x{}", hex::encode(salt)).into());
    let private_bytes = canonical::canonical_json(&Value::Object(private))
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let commitment = canonical::content_hash(&private_bytes);
    record[PRIVATE_COMMITMENT_FIELD] = format!("{commitment:#x}").into();
    let committed =
        canonical::canonical_json(&record).map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let ciphertext = crypto::encrypt_envelope_value(
        &state.config.secrets.pii_encryption_key,
        &private_bytes,
        &canonical::private_aad(record_type, subject_id, commitment),
    );
    let mut stored = record;
    stored[PRIVATE_CIPHERTEXT_FIELD] = ciphertext;
    let payload = canonical::canonical_json(&stored).map_err(ApiError::internal)?;
    finish(state, record_type, subject_id, committed, payload, true)
}

/// Prepare from already-canonical bytes of a record with no private part (e.g. revealing a committed AI
/// assessment).
pub fn prepare_from_canonical(
    state: &AppState,
    record_type: RecordType,
    subject_id: B256,
    canonical_bytes: Vec<u8>,
) -> Result<PreparedRecord, ApiError> {
    finish(
        state,
        record_type,
        subject_id,
        canonical_bytes.clone(),
        canonical_bytes,
        false,
    )
}

fn finish(
    state: &AppState,
    record_type: RecordType,
    subject_id: B256,
    committed: Vec<u8>,
    payload: Vec<u8>,
    encrypted: bool,
) -> Result<PreparedRecord, ApiError> {
    let content_hash = canonical::content_hash(&committed);
    let domain = state.domain();
    let record_digest = canonical::record_digest(
        domain.chain_id,
        domain.registry,
        record_type,
        subject_id,
        content_hash,
    );
    if payload.len() > MAX_BLOCK_SIZE {
        return Err(ApiError::BadRequest(format!(
            "record is {} bytes; the limit is {MAX_BLOCK_SIZE}. Upload large documents via /api/ipfs/upload and reference their CIDs",
            payload.len()
        )));
    }
    let cid = Cid::raw_sha256(&payload);
    Ok(PreparedRecord {
        record_type,
        subject_id,
        content_hash,
        record_digest,
        payload,
        cid,
        encrypted,
        public_json: Some(String::from_utf8(committed).map_err(ApiError::internal)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn declared_private_fields_are_split_out() {
        let data = json!({"label": "m1", "crops": ["soybean"], "profile": {"name": "A"}, "wallet_address": "0xabc"});
        let (public, private) = split_private(RecordType::FarmerRegistration, data).unwrap();
        assert_eq!(public, json!({"label": "m1", "crops": ["soybean"]}));
        assert_eq!(
            private.keys().collect::<Vec<_>>(),
            ["profile", "wallet_address"]
        );

        let data = json!({"category": "adulteration", "description": "x", "evidence_cids": []});
        let (public, private) = split_private(RecordType::FraudReport, data).unwrap();
        assert_eq!(public, json!({"category": "adulteration"}));
        assert_eq!(private.len(), 2);

        let data = json!({"crop": "soybean", "delivered_quantity_kg": 10});
        let (public, private) = split_private(RecordType::Procurement, data.clone()).unwrap();
        assert_eq!(public, data);
        assert!(private.is_empty());
    }
}
