//! Utilities Module
//!
//! Common utilities for the offchain service:
//! - Compression for large payloads
//! - Hashing utilities
//! - Data transformation helpers

pub mod compression;

pub use compression::{
    compress_data, decompress_data, CompressionConfig, CompressionLevel, CompressionStats,
};

use sha2::{Digest, Sha256};

/// Calculate SHA256 hash of data
pub fn sha256_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("0x{}", hex::encode(hasher.finalize()))
}

/// Calculate SHA256 hash and return as bytes
pub fn sha256_hash_bytes(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Calculate MD5 hash (for content identification, not security)
pub fn md5_hash(data: &[u8]) -> String {
    format!("{:x}", md5::compute(data))
}

/// Generate a unique ID based on content hash and timestamp
pub fn content_id(data: &[u8]) -> String {
    let hash = &sha256_hash(data)[2..18]; // First 16 chars of hash (without 0x)
    let timestamp = chrono::Utc::now().timestamp_millis();
    format!("{}_{}", hash, timestamp)
}

/// Estimate if data should be compressed based on size
pub fn should_compress(data: &[u8], threshold_bytes: usize) -> bool {
    data.len() > threshold_bytes
}

/// Safe JSON serialization with size limit
pub fn safe_json_serialize<T: serde::Serialize>(
    data: &T,
    max_size_bytes: usize,
) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(data).map_err(|e| format!("Serialization error: {}", e))?;

    if json.len() > max_size_bytes {
        return Err(format!(
            "Serialized data ({} bytes) exceeds maximum size ({} bytes)",
            json.len(),
            max_size_bytes
        ));
    }

    Ok(json)
}

/// Truncate string with ellipsis for logging
pub fn truncate_for_log(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Format bytes as human-readable size
pub fn format_bytes(bytes: usize) -> String {
    const KB: usize = 1024;
    const MB: usize = KB * 1024;
    const GB: usize = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Validate CID format
pub fn is_valid_cid(cid: &str) -> bool {
    // CIDv0: Qm followed by 44 base58 characters
    // CIDv1: bafy followed by base32 characters
    if cid.starts_with("Qm") && cid.len() == 46 {
        cid[2..].chars().all(|c| {
            c.is_alphanumeric() && c != '0' && c != 'O' && c != 'I' && c != 'l'
        })
    } else if cid.starts_with("bafy") {
        cid.len() > 50 && cid.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    } else {
        false
    }
}

/// Parse duration from string (e.g., "30s", "5m", "1h")
pub fn parse_duration(s: &str) -> Result<std::time::Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("Empty duration string".to_string());
    }

    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: u64 = num_str
        .parse()
        .map_err(|_| format!("Invalid number in duration: {}", s))?;

    let multiplier = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => return Err(format!("Unknown duration unit: {}", unit)),
    };

    Ok(std::time::Duration::from_secs(num * multiplier))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_hash() {
        let hash = sha256_hash(b"test data");
        assert!(hash.starts_with("0x"));
        assert_eq!(hash.len(), 66); // 0x + 64 hex chars
    }

    #[test]
    fn test_md5_hash() {
        let hash = md5_hash(b"test data");
        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn test_content_id() {
        let id1 = content_id(b"test data");
        let id2 = content_id(b"test data");
        // Same content should produce same hash prefix but different timestamps
        assert_eq!(&id1[..16], &id2[..16]);
    }

    #[test]
    fn test_should_compress() {
        let small_data = vec![0u8; 100];
        let large_data = vec![0u8; 10000];

        assert!(!should_compress(&small_data, 1024));
        assert!(should_compress(&large_data, 1024));
    }

    #[test]
    fn test_truncate_for_log() {
        assert_eq!(truncate_for_log("short", 10), "short");
        assert_eq!(truncate_for_log("this is a very long string", 10), "this is...");
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 bytes");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1572864), "1.50 MB");
    }

    #[test]
    fn test_is_valid_cid() {
        assert!(is_valid_cid("QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"));
        assert!(is_valid_cid("bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi"));
        assert!(!is_valid_cid("invalid"));
        assert!(!is_valid_cid("Qm")); // Too short
    }

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("30s").unwrap(), std::time::Duration::from_secs(30));
        assert_eq!(parse_duration("5m").unwrap(), std::time::Duration::from_secs(300));
        assert_eq!(parse_duration("1h").unwrap(), std::time::Duration::from_secs(3600));
        assert_eq!(parse_duration("2d").unwrap(), std::time::Duration::from_secs(172800));
        assert!(parse_duration("invalid").is_err());
    }

    #[test]
    fn test_safe_json_serialize() {
        #[derive(serde::Serialize)]
        struct TestData {
            value: String,
        }

        let data = TestData {
            value: "test".to_string(),
        };

        let result = safe_json_serialize(&data, 1000);
        assert!(result.is_ok());

        let result = safe_json_serialize(&data, 5);
        assert!(result.is_err());
    }
}
