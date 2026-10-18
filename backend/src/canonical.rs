//! Canonical record serialization and the commitments derived from it. Specification: docs/INTEGRITY.md.
//!
//! * `canonical_json` — UTF-8 JSON, no insignificant whitespace, object keys sorted by byte order,
//!   numbers in serde_json's shortest round-trip form. Integers must be within ±2^53 and floats finite.
//! * `content_hash = keccak256(canonical_json(record))` — commits to the plaintext record.
//! * `record_digest = keccak256(abi.encode(RECORD_TYPEHASH, chainId, registry, recordType, subjectId,
//!   contentHash))` — identical to `OilseedValueChain.computeRecordDigest`; adds domain separation so a
//!   commitment for one record type, subject, chain or registry can never be replayed as another.

use alloy::primitives::{keccak256, Address, B256, U256};
use alloy::sol_types::SolValue;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;

pub const RECORD_SCHEMA: &str = "oilschain.record.v1";
const MAX_SAFE_INTEGER: u64 = 1 << 53;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RecordType {
    FarmerRegistration = 1,
    Procurement = 2,
    CustodyTransfer = 3,
    WarehouseInspection = 4,
    Logistics = 5,
    Processing = 6,
    Packaging = 7,
    FraudReport = 8,
    FraudResolution = 9,
    AiAssessment = 10,
}

impl RecordType {
    pub const ALL: [RecordType; 10] = [
        Self::FarmerRegistration,
        Self::Procurement,
        Self::CustodyTransfer,
        Self::WarehouseInspection,
        Self::Logistics,
        Self::Processing,
        Self::Packaging,
        Self::FraudReport,
        Self::FraudResolution,
        Self::AiAssessment,
    ];

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_u8() == v)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::FarmerRegistration => "farmer_registration",
            Self::Procurement => "procurement",
            Self::CustodyTransfer => "custody_transfer",
            Self::WarehouseInspection => "warehouse_inspection",
            Self::Logistics => "logistics",
            Self::Processing => "processing",
            Self::Packaging => "packaging",
            Self::FraudReport => "fraud_report",
            Self::FraudResolution => "fraud_resolution",
            Self::AiAssessment => "ai_assessment",
        }
    }

    /// Top-level `data` fields that are personal or confidential for this record type. They are moved into an
    /// encrypted, salted private part; everything else in `data` stays public and publicly verifiable.
    /// Declared per type (not detected), so a new field is public only if someone decided it is.
    pub fn private_fields(self) -> &'static [&'static str] {
        match self {
            Self::FarmerRegistration => &["profile", "wallet_address"],
            Self::FraudReport => &["description", "evidence_cids"],
            _ => &[],
        }
    }
}

/// Committed record field holding `keccak256(canonical_json(private_part))`.
pub const PRIVATE_COMMITMENT_FIELD: &str = "private_commitment";
/// Stored-only field holding the encrypted private part. Not covered by `contentHash` (it is covered by the
/// commitment above and by the CID of the stored block).
pub const PRIVATE_CIPHERTEXT_FIELD: &str = "private_ciphertext";

/// AAD for the private part: binds the ciphertext to its record type, subject and commitment.
pub fn private_aad(record_type: RecordType, subject_id: B256, commitment: B256) -> String {
    format!("{}:{subject_id:#x}:{commitment:#x}", record_type.name())
}

impl fmt::Display for RecordType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CanonicalError {
    #[error("integer {0} is outside the interoperable range ±2^53")]
    UnsafeInteger(String),
    #[error("number {0} is not representable")]
    BadNumber(String),
    #[error("payload is not valid JSON: {0}")]
    InvalidJson(String),
    #[error("payload bytes are not in canonical form")]
    NotCanonical,
}

/// Serialize `value` canonically.
pub fn canonical_json(value: &Value) -> Result<Vec<u8>, CanonicalError> {
    let mut out = Vec::with_capacity(256);
    write_canonical(value, &mut out)?;
    Ok(out)
}

fn write_canonical(value: &Value, out: &mut Vec<u8>) -> Result<(), CanonicalError> {
    match value {
        Value::Null | Value::Bool(_) | Value::String(_) => {
            out.extend_from_slice(
                serde_json::to_string(value)
                    .expect("scalar serializes")
                    .as_bytes(),
            );
        }
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i.unsigned_abs() > MAX_SAFE_INTEGER {
                    return Err(CanonicalError::UnsafeInteger(n.to_string()));
                }
            } else if let Some(u) = n.as_u64() {
                if u > MAX_SAFE_INTEGER {
                    return Err(CanonicalError::UnsafeInteger(n.to_string()));
                }
            } else if !n.as_f64().is_some_and(f64::is_finite) {
                return Err(CanonicalError::BadNumber(n.to_string()));
            }
            out.extend_from_slice(n.to_string().as_bytes());
        }
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            out.push(b'{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(
                    serde_json::to_string(key)
                        .expect("key serializes")
                        .as_bytes(),
                );
                out.push(b':');
                write_canonical(&map[key], out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

/// Parse bytes that must already be canonical (as uploaded). Rejects any re-encoded or tampered form.
pub fn parse_canonical(bytes: &[u8]) -> Result<Value, CanonicalError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|e| CanonicalError::InvalidJson(e.to_string()))?;
    if canonical_json(&value)? != bytes {
        return Err(CanonicalError::NotCanonical);
    }
    Ok(value)
}

pub fn content_hash(canonical_bytes: &[u8]) -> B256 {
    keccak256(canonical_bytes)
}

pub fn record_typehash() -> B256 {
    keccak256(
        "OilsChainRecord(uint256 chainId,address registry,uint8 recordType,bytes32 subjectId,bytes32 contentHash)",
    )
}

pub fn ai_commit_typehash() -> B256 {
    keccak256("OilsChainAICommit(bytes32 batchId,bytes32 contentHash,bytes32 salt,address oracle)")
}

/// Mirrors `OilseedValueChain.computeRecordDigest`.
pub fn record_digest(
    chain_id: u64,
    registry: Address,
    record_type: RecordType,
    subject_id: B256,
    content_hash: B256,
) -> B256 {
    keccak256(
        (
            record_typehash(),
            U256::from(chain_id),
            registry,
            U256::from(record_type.as_u8()), // uint8 is ABI-encoded as a 32-byte word
            subject_id,
            content_hash,
        )
            .abi_encode_params(),
    )
}

/// Mirrors the batch hash chain: `head = keccak256(abi.encode(head, recordDigest))`.
pub fn next_head(head: B256, digest: B256) -> B256 {
    keccak256((head, digest).abi_encode_params())
}

/// Mirrors `OilseedValueChain.computeAICommit`.
pub fn ai_commit(batch_id: B256, content_hash: B256, salt: B256, oracle: Address) -> B256 {
    keccak256((ai_commit_typehash(), batch_id, content_hash, salt, oracle).abi_encode_params())
}

/// Context every record is bound to.
#[derive(Debug, Clone, Copy)]
pub struct Domain {
    pub chain_id: u64,
    pub registry: Address,
}

/// Build the record envelope that is canonicalized and hashed.
pub fn build_record(
    domain: Domain,
    record_type: RecordType,
    subject_id: B256,
    submitted_by: Address,
    data: Value,
) -> Value {
    let nonce: [u8; 16] = rand::random();
    let mut m = Map::new();
    m.insert("schema".into(), RECORD_SCHEMA.into());
    m.insert("record_type".into(), record_type.name().into());
    m.insert("subject_id".into(), format!("{subject_id:#x}").into());
    m.insert("chain_id".into(), domain.chain_id.into());
    m.insert("registry".into(), format!("{:#x}", domain.registry).into());
    m.insert("submitted_by".into(), format!("{submitted_by:#x}").into());
    m.insert(
        "created_at".into(),
        chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            .into(),
    );
    // Random salt: makes repeated events distinct and blocks dictionary attacks on the plaintext digest.
    m.insert("nonce".into(), format!("0x{}", hex::encode(nonce)).into());
    m.insert("data".into(), data);
    Value::Object(m)
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum AssociationError {
    #[error("record field `{0}` is missing or malformed")]
    Missing(&'static str),
    #[error("record field `{field}` is {actual}, expected {expected}")]
    Mismatch {
        field: &'static str,
        expected: String,
        actual: String,
    },
}

/// Check that a decoded record belongs to the expected domain, type and subject.
pub fn check_association(
    record: &Value,
    domain: Domain,
    record_type: RecordType,
    subject_id: B256,
) -> Result<(), AssociationError> {
    let field = |name: &'static str| -> Result<String, AssociationError> {
        match record.get(name) {
            Some(Value::String(s)) => Ok(s.to_lowercase()),
            Some(Value::Number(n)) => Ok(n.to_string()),
            _ => Err(AssociationError::Missing(name)),
        }
    };
    let expect = |name: &'static str, expected: String| -> Result<(), AssociationError> {
        let actual = field(name)?;
        if actual != expected {
            return Err(AssociationError::Mismatch {
                field: name,
                expected,
                actual,
            });
        }
        Ok(())
    };
    expect("schema", RECORD_SCHEMA.into())?;
    expect("record_type", record_type.name().into())?;
    expect("subject_id", format!("{subject_id:#x}"))?;
    expect("chain_id", domain.chain_id.to_string())?;
    expect("registry", format!("{:#x}", domain.registry))?;
    if !record.get("data").is_some_and(Value::is_object) {
        return Err(AssociationError::Missing("data"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_sorts_keys_and_strips_whitespace() {
        let v: Value =
            serde_json::from_str(r#"{ "b": 1, "a": {"z": [1, 2.5, "x"], "c": null}, "é": true }"#)
                .unwrap();
        let bytes = canonical_json(&v).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            r#"{"a":{"c":null,"z":[1,2.5,"x"]},"b":1,"é":true}"#
        );
    }

    #[test]
    fn canonical_is_stable_under_reserialization() {
        let v = json!({"q": 95.5, "m": 0.1, "n": -3, "s": "क"});
        let a = canonical_json(&v).unwrap();
        let reparsed = parse_canonical(&a).unwrap();
        assert_eq!(canonical_json(&reparsed).unwrap(), a);
    }

    #[test]
    fn parse_canonical_rejects_non_canonical_bytes() {
        assert_eq!(
            parse_canonical(br#"{"b":1,"a":2}"#),
            Err(CanonicalError::NotCanonical)
        );
        assert_eq!(
            parse_canonical(br#"{"a": 2}"#),
            Err(CanonicalError::NotCanonical)
        );
        assert!(matches!(
            parse_canonical(b"{nope"),
            Err(CanonicalError::InvalidJson(_))
        ));
    }

    #[test]
    fn canonical_rejects_unsafe_integers() {
        let v: Value = serde_json::from_str("9007199254740993").unwrap();
        assert!(matches!(
            canonical_json(&v),
            Err(CanonicalError::UnsafeInteger(_))
        ));
        assert!(canonical_json(&json!(9007199254740992u64)).is_ok());
    }

    #[test]
    fn digest_is_domain_separated() {
        let reg: Address = "0x5FbDB2315678afecb367f032d93F642f64180aa3"
            .parse()
            .unwrap();
        let subject = B256::repeat_byte(1);
        let content = B256::repeat_byte(2);
        let a = record_digest(31337, reg, RecordType::Procurement, subject, content);
        assert_ne!(
            a,
            record_digest(31337, reg, RecordType::Logistics, subject, content)
        );
        assert_ne!(
            a,
            record_digest(1, reg, RecordType::Procurement, subject, content)
        );
        assert_ne!(
            a,
            record_digest(
                31337,
                Address::ZERO,
                RecordType::Procurement,
                subject,
                content
            )
        );
        assert_ne!(
            a,
            record_digest(31337, reg, RecordType::Procurement, B256::ZERO, content)
        );
    }

    #[test]
    fn record_type_roundtrip_matches_contract_constants() {
        for t in RecordType::ALL {
            assert_eq!(RecordType::from_u8(t.as_u8()), Some(t));
        }
        assert_eq!(RecordType::FarmerRegistration.as_u8(), 1);
        assert_eq!(RecordType::AiAssessment.as_u8(), 10);
        assert_eq!(RecordType::from_u8(0), None);
    }

    #[test]
    fn association_detects_wrong_subject_and_type() {
        let domain = Domain {
            chain_id: 31337,
            registry: Address::repeat_byte(9),
        };
        let subject = B256::repeat_byte(7);
        let r = build_record(
            domain,
            RecordType::Logistics,
            subject,
            Address::ZERO,
            json!({"x": 1}),
        );
        assert!(check_association(&r, domain, RecordType::Logistics, subject).is_ok());
        assert!(matches!(
            check_association(&r, domain, RecordType::Processing, subject),
            Err(AssociationError::Mismatch {
                field: "record_type",
                ..
            })
        ));
        assert!(matches!(
            check_association(&r, domain, RecordType::Logistics, B256::ZERO),
            Err(AssociationError::Mismatch {
                field: "subject_id",
                ..
            })
        ));
    }
}
