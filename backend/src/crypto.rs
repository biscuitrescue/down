//! AES-256-GCM encryption for confidential IPFS records and secrets at rest, plus API token hashing.
//! Keys come from the environment and never touch the chain or IPFS.

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const ENVELOPE_SCHEMA: &str = "oilschain.encrypted.v1";

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CryptoError {
    #[error("not an encrypted envelope")]
    NotEnvelope,
    #[error("envelope was encrypted with a different key (kid {0})")]
    WrongKey(String),
    #[error("envelope is malformed: {0}")]
    Malformed(&'static str),
    #[error("decryption failed (tampered ciphertext or wrong associated data)")]
    DecryptFailed,
}

/// Short, non-secret key identifier so a verifier can tell which key an envelope needs.
pub fn key_id(key: &[u8; 32]) -> String {
    hex::encode(&Sha256::digest(key)[..8])
}

fn cipher(key: &[u8; 32]) -> Aes256Gcm {
    Aes256Gcm::new_from_slice(key).expect("32-byte key")
}

/// Encrypt `plaintext` into a canonical JSON envelope suitable for IPFS. `aad` binds the ciphertext to its
/// record type and subject so it cannot be swapped into another record.
pub fn encrypt_envelope(key: &[u8; 32], plaintext: &[u8], aad: &str) -> Vec<u8> {
    crate::canonical::canonical_json(&encrypt_envelope_value(key, plaintext, aad))
        .expect("envelope is canonicalizable")
}

/// Same as `encrypt_envelope`, returned as a JSON value (for embedding in a record).
pub fn encrypt_envelope_value(key: &[u8; 32], plaintext: &[u8], aad: &str) -> Value {
    let nonce: [u8; 12] = rand::random();
    let ciphertext = cipher(key)
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: plaintext,
                aad: aad.as_bytes(),
            },
        )
        .expect("AES-GCM encryption cannot fail for valid input");
    json!({
        "schema": ENVELOPE_SCHEMA,
        "alg": "A256GCM",
        "kid": key_id(key),
        "aad": aad,
        "nonce": B64.encode(nonce),
        "ciphertext": B64.encode(ciphertext),
    })
}

pub fn is_envelope(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|v| {
            v.get("schema")
                .and_then(Value::as_str)
                .map(|s| s == ENVELOPE_SCHEMA)
        })
        .unwrap_or(false)
}

/// Decrypt an envelope, returning `(plaintext, aad)`.
pub fn decrypt_envelope(key: &[u8; 32], bytes: &[u8]) -> Result<(Vec<u8>, String), CryptoError> {
    let v: Value = serde_json::from_slice(bytes).map_err(|_| CryptoError::NotEnvelope)?;
    if v.get("schema").and_then(Value::as_str) != Some(ENVELOPE_SCHEMA) {
        return Err(CryptoError::NotEnvelope);
    }
    if v.get("alg").and_then(Value::as_str) != Some("A256GCM") {
        return Err(CryptoError::Malformed("alg"));
    }
    let kid = v
        .get("kid")
        .and_then(Value::as_str)
        .ok_or(CryptoError::Malformed("kid"))?;
    if kid != key_id(key) {
        return Err(CryptoError::WrongKey(kid.to_string()));
    }
    let aad = v
        .get("aad")
        .and_then(Value::as_str)
        .ok_or(CryptoError::Malformed("aad"))?;
    let nonce = v
        .get("nonce")
        .and_then(Value::as_str)
        .and_then(|s| B64.decode(s).ok())
        .and_then(|n| <[u8; 12]>::try_from(n.as_slice()).ok())
        .ok_or(CryptoError::Malformed("nonce"))?;
    let ct = v
        .get("ciphertext")
        .and_then(Value::as_str)
        .and_then(|s| B64.decode(s).ok())
        .ok_or(CryptoError::Malformed("ciphertext"))?;
    let plaintext = cipher(key)
        .decrypt(
            &Nonce::from(nonce),
            Payload {
                msg: &ct,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| CryptoError::DecryptFailed)?;
    Ok((plaintext, aad.to_string()))
}

/// Encrypt a secret for storage in the database: base64(nonce || ciphertext).
pub fn seal(key: &[u8; 32], plaintext: &[u8], aad: &str) -> String {
    let nonce: [u8; 12] = rand::random();
    let ct = cipher(key)
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: plaintext,
                aad: aad.as_bytes(),
            },
        )
        .expect("AES-GCM encryption cannot fail for valid input");
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    B64.encode(out)
}

pub fn open(key: &[u8; 32], sealed: &str, aad: &str) -> Result<Vec<u8>, CryptoError> {
    let raw = B64
        .decode(sealed)
        .map_err(|_| CryptoError::Malformed("base64"))?;
    if raw.len() < 12 + 16 {
        return Err(CryptoError::Malformed("length"));
    }
    let (nonce, ct) = raw.split_at(12);
    let nonce: [u8; 12] = nonce
        .try_into()
        .map_err(|_| CryptoError::Malformed("nonce"))?;
    cipher(key)
        .decrypt(
            &Nonce::from(nonce),
            Payload {
                msg: ct,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| CryptoError::DecryptFailed)
}

/// New participant API token. Only its SHA-256 is stored.
pub fn new_api_token() -> String {
    let bytes: [u8; 32] = rand::random();
    format!("oc_{}", hex::encode(bytes))
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7u8; 32];

    #[test]
    fn envelope_roundtrip_and_aad_binding() {
        let env = encrypt_envelope(&KEY, b"secret profile", "farmer_registration:0xabc");
        assert!(is_envelope(&env));
        let text = String::from_utf8(env.clone()).unwrap();
        assert!(!text.contains("secret profile"));
        let (pt, aad) = decrypt_envelope(&KEY, &env).unwrap();
        assert_eq!(pt, b"secret profile");
        assert_eq!(aad, "farmer_registration:0xabc");

        // Changing the AAD inside the envelope must break authentication.
        let tampered = text.replace("0xabc", "0xdef");
        assert_eq!(
            decrypt_envelope(&KEY, tampered.as_bytes()),
            Err(CryptoError::DecryptFailed)
        );
    }

    #[test]
    fn envelope_rejects_wrong_key() {
        let env = encrypt_envelope(&KEY, b"x", "a");
        assert!(matches!(
            decrypt_envelope(&[8u8; 32], &env),
            Err(CryptoError::WrongKey(_))
        ));
        assert_eq!(decrypt_envelope(&KEY, b"{}"), Err(CryptoError::NotEnvelope));
    }

    #[test]
    fn seal_open_roundtrip() {
        let sealed = seal(&KEY, b"0xprivatekey", "wallet:p1");
        assert_eq!(open(&KEY, &sealed, "wallet:p1").unwrap(), b"0xprivatekey");
        assert_eq!(
            open(&KEY, &sealed, "wallet:p2"),
            Err(CryptoError::DecryptFailed)
        );
    }

    #[test]
    fn tokens_are_hashed() {
        let t = new_api_token();
        assert!(t.starts_with("oc_") && t.len() == 67);
        assert_eq!(token_hash(&t), token_hash(&t));
        assert_ne!(token_hash(&t), token_hash("other"));
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
    }
}
