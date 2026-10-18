//! Minimal CID parsing/validation (CIDv0 and CIDv1 in base32 / base58btc multibase).
//!
//! Records are stored as raw IPFS blocks (`cid-codec=raw`, `sha2-256`), so their CID is
//! `bafkrei…` = CIDv1(raw, sha256(bytes)) and can be computed and checked locally without trusting the node.

use std::fmt;

pub const CODEC_RAW: u64 = 0x55;
pub const CODEC_DAG_PB: u64 = 0x70;
pub const HASH_SHA2_256: u64 = 0x12;
pub const MAX_CID_LENGTH: usize = 128; // mirrors the contracts' MAX_CID_LENGTH

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cid {
    pub version: u8,
    pub codec: u64,
    pub hash_code: u64,
    pub digest: Vec<u8>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CidError {
    #[error("CID is empty or longer than {MAX_CID_LENGTH} characters")]
    Length,
    #[error("unsupported multibase prefix")]
    Multibase,
    #[error("invalid base encoding")]
    Encoding,
    #[error("malformed CID bytes")]
    Malformed,
    #[error("unsupported CID version {0}")]
    Version(u64),
}

fn read_varint(bytes: &[u8], pos: &mut usize) -> Result<u64, CidError> {
    let mut value: u64 = 0;
    for shift in (0..64).step_by(7) {
        let b = *bytes.get(*pos).ok_or(CidError::Malformed)?;
        *pos += 1;
        value |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(CidError::Malformed)
}

fn write_varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn base32_lower_decode(s: &str) -> Result<Vec<u8>, CidError> {
    data_encoding::BASE32_NOPAD
        .decode(s.to_ascii_uppercase().as_bytes())
        .map_err(|_| CidError::Encoding)
}

impl Cid {
    /// CIDv1 for a raw block hashed with sha2-256.
    pub fn raw_sha256(bytes: &[u8]) -> Self {
        Cid {
            version: 1,
            codec: CODEC_RAW,
            hash_code: HASH_SHA2_256,
            digest: crate::crypto::sha256(bytes).to_vec(),
        }
    }

    pub fn parse(s: &str) -> Result<Self, CidError> {
        if s.is_empty() || s.len() > MAX_CID_LENGTH {
            return Err(CidError::Length);
        }
        if s.len() == 46 && s.starts_with("Qm") {
            let raw = bs58::decode(s).into_vec().map_err(|_| CidError::Encoding)?;
            if raw.len() != 34 || raw[0] != 0x12 || raw[1] != 0x20 {
                return Err(CidError::Malformed);
            }
            return Ok(Cid {
                version: 0,
                codec: CODEC_DAG_PB,
                hash_code: HASH_SHA2_256,
                digest: raw[2..].to_vec(),
            });
        }
        let (prefix, rest) = s.split_at(1);
        let raw = match prefix {
            "b" => base32_lower_decode(rest)?,
            "z" => bs58::decode(rest)
                .into_vec()
                .map_err(|_| CidError::Encoding)?,
            _ => return Err(CidError::Multibase),
        };
        let mut pos = 0;
        let version = read_varint(&raw, &mut pos)?;
        if version != 1 {
            return Err(CidError::Version(version));
        }
        let codec = read_varint(&raw, &mut pos)?;
        let hash_code = read_varint(&raw, &mut pos)?;
        let len = read_varint(&raw, &mut pos)? as usize;
        let digest = raw.get(pos..).ok_or(CidError::Malformed)?;
        if digest.len() != len || len == 0 {
            return Err(CidError::Malformed);
        }
        if hash_code == HASH_SHA2_256 && len != 32 {
            return Err(CidError::Malformed);
        }
        Ok(Cid {
            version: 1,
            codec,
            hash_code,
            digest: digest.to_vec(),
        })
    }

    /// True if the CID commits directly to the block bytes with sha2-256 (so it is checkable locally).
    pub fn is_locally_verifiable(&self) -> bool {
        self.version == 1 && self.codec == CODEC_RAW && self.hash_code == HASH_SHA2_256
    }

    /// Check fetched bytes against the CID. `None` when the CID type cannot be checked locally.
    pub fn verify_bytes(&self, bytes: &[u8]) -> Option<bool> {
        self.is_locally_verifiable()
            .then(|| crate::crypto::sha256(bytes).as_slice() == self.digest.as_slice())
    }
}

impl fmt::Display for Cid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.version == 0 {
            let mut raw = vec![0x12, 0x20];
            raw.extend_from_slice(&self.digest);
            return f.write_str(&bs58::encode(raw).into_string());
        }
        let mut raw = Vec::with_capacity(4 + self.digest.len());
        write_varint(1, &mut raw);
        write_varint(self.codec, &mut raw);
        write_varint(self.hash_code, &mut raw);
        write_varint(self.digest.len() as u64, &mut raw);
        raw.extend_from_slice(&self.digest);
        write!(
            f,
            "b{}",
            data_encoding::BASE32_NOPAD
                .encode(&raw)
                .to_ascii_lowercase()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_cid_matches_known_vector() {
        // `echo -n hello | ipfs block put --cid-codec raw --mhtype sha2-256`
        let cid = Cid::raw_sha256(b"hello");
        assert_eq!(
            cid.to_string(),
            "bafkreibm6jg3ux5qumhcn2b3flc3tyu6dmlb4xa7u5bf44yegnrjhc4yeq"
        );
        assert_eq!(Cid::parse(&cid.to_string()).unwrap(), cid);
        assert_eq!(cid.verify_bytes(b"hello"), Some(true));
        assert_eq!(cid.verify_bytes(b"hellO"), Some(false));
    }

    #[test]
    fn parses_cidv0() {
        let cid = Cid::parse("QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG").unwrap();
        assert_eq!(cid.version, 0);
        assert_eq!(cid.codec, CODEC_DAG_PB);
        assert_eq!(cid.verify_bytes(b"x"), None);
        assert_eq!(
            cid.to_string(),
            "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"
        );
    }

    #[test]
    fn rejects_garbage() {
        // The legacy mock server produced "Qm" + 44 hex chars; '0' is outside the base58 alphabet.
        let fake = format!("Qm{}", "0a".repeat(22));
        assert!(Cid::parse(&fake).is_err());
        assert_eq!(Cid::parse(""), Err(CidError::Length));
        assert_eq!(Cid::parse("xabc"), Err(CidError::Multibase));
        assert!(Cid::parse("bafkrei").is_err());
        assert!(Cid::parse(&"b".repeat(200)).is_err());
    }
}
