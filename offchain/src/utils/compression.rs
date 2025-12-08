//! Compression Utilities
//!
//! Provides compression and decompression for large IPFS payloads.
//! Features:
//! - Gzip compression with configurable levels
//! - Automatic compression for large payloads
//! - Compression statistics tracking
//! - Support for streaming compression

use flate2::read::{GzDecoder, GzEncoder};
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use tracing::{debug, warn};

/// Compression level presets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompressionLevel {
    /// No compression (level 0)
    None,
    /// Fast compression, lower ratio (level 1)
    Fast,
    /// Balanced compression (level 6, default)
    Default,
    /// Best compression, slower (level 9)
    Best,
    /// Custom level (0-9)
    Custom(u32),
}

impl CompressionLevel {
    pub fn to_flate2(self) -> Compression {
        match self {
            Self::None => Compression::none(),
            Self::Fast => Compression::fast(),
            Self::Default => Compression::default(),
            Self::Best => Compression::best(),
            Self::Custom(level) => Compression::new(level.min(9)),
        }
    }

    pub fn level(&self) -> u32 {
        match self {
            Self::None => 0,
            Self::Fast => 1,
            Self::Default => 6,
            Self::Best => 9,
            Self::Custom(level) => *level,
        }
    }
}

impl Default for CompressionLevel {
    fn default() -> Self {
        Self::Default
    }
}

/// Configuration for compression
#[derive(Debug, Clone)]
pub struct CompressionConfig {
    /// Compression level to use
    pub level: CompressionLevel,
    /// Minimum size in bytes before compression is applied
    pub min_size_threshold: usize,
    /// Maximum size in bytes for input data
    pub max_input_size: usize,
    /// Whether to skip compression if it doesn't reduce size
    pub skip_if_no_benefit: bool,
    /// Minimum compression ratio to accept (e.g., 0.9 means must be at least 10% smaller)
    pub min_compression_ratio: f64,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            level: CompressionLevel::Default,
            min_size_threshold: 1024,        // 1 KB
            max_input_size: 100 * 1024 * 1024, // 100 MB
            skip_if_no_benefit: true,
            min_compression_ratio: 0.95,
        }
    }
}

impl CompressionConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_level(mut self, level: CompressionLevel) -> Self {
        self.level = level;
        self
    }

    pub fn with_min_size(mut self, size: usize) -> Self {
        self.min_size_threshold = size;
        self
    }

    pub fn with_max_input_size(mut self, size: usize) -> Self {
        self.max_input_size = size;
        self
    }

    pub fn fast() -> Self {
        Self {
            level: CompressionLevel::Fast,
            ..Default::default()
        }
    }

    pub fn best() -> Self {
        Self {
            level: CompressionLevel::Best,
            ..Default::default()
        }
    }
}

/// Statistics from a compression operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionStats {
    /// Original size in bytes
    pub original_size: usize,
    /// Compressed size in bytes
    pub compressed_size: usize,
    /// Compression ratio (compressed / original)
    pub ratio: f64,
    /// Space saved in bytes
    pub bytes_saved: i64,
    /// Space saved as percentage
    pub percent_saved: f64,
    /// Whether compression was applied
    pub was_compressed: bool,
    /// Compression level used
    pub level: u32,
    /// Time taken in milliseconds
    pub duration_ms: u64,
}

impl CompressionStats {
    pub fn no_compression(size: usize) -> Self {
        Self {
            original_size: size,
            compressed_size: size,
            ratio: 1.0,
            bytes_saved: 0,
            percent_saved: 0.0,
            was_compressed: false,
            level: 0,
            duration_ms: 0,
        }
    }
}

/// Result of compression operation
#[derive(Debug)]
pub struct CompressionResult {
    /// The output data (compressed or original)
    pub data: Vec<u8>,
    /// Statistics about the compression
    pub stats: CompressionStats,
}

/// Compress data using gzip with default configuration
pub fn compress_data(data: &[u8]) -> Result<CompressionResult, String> {
    compress_with_config(data, &CompressionConfig::default())
}

/// Compress data using gzip with custom configuration
pub fn compress_with_config(data: &[u8], config: &CompressionConfig) -> Result<CompressionResult, String> {
    let start = std::time::Instant::now();
    let original_size = data.len();

    // Check max input size
    if original_size > config.max_input_size {
        return Err(format!(
            "Input size ({} bytes) exceeds maximum allowed ({} bytes)",
            original_size, config.max_input_size
        ));
    }

    // Check if compression should be skipped due to small size
    if original_size < config.min_size_threshold {
        debug!(
            size = original_size,
            threshold = config.min_size_threshold,
            "Skipping compression: below size threshold"
        );
        return Ok(CompressionResult {
            data: data.to_vec(),
            stats: CompressionStats::no_compression(original_size),
        });
    }

    // Perform compression
    let mut encoder = GzEncoder::new(data, config.level.to_flate2());
    let mut compressed = Vec::new();

    encoder
        .read_to_end(&mut compressed)
        .map_err(|e| format!("Compression failed: {}", e))?;

    let compressed_size = compressed.len();
    let ratio = compressed_size as f64 / original_size as f64;
    let duration_ms = start.elapsed().as_millis() as u64;

    // Check if compression provided benefit
    if config.skip_if_no_benefit && ratio >= config.min_compression_ratio {
        debug!(
            original_size = original_size,
            compressed_size = compressed_size,
            ratio = ratio,
            "Skipping compression: no significant size reduction"
        );
        return Ok(CompressionResult {
            data: data.to_vec(),
            stats: CompressionStats {
                original_size,
                compressed_size: original_size,
                ratio: 1.0,
                bytes_saved: 0,
                percent_saved: 0.0,
                was_compressed: false,
                level: config.level.level(),
                duration_ms,
            },
        });
    }

    let bytes_saved = original_size as i64 - compressed_size as i64;
    let percent_saved = (1.0 - ratio) * 100.0;

    debug!(
        original_size = original_size,
        compressed_size = compressed_size,
        ratio = format!("{:.2}", ratio),
        percent_saved = format!("{:.1}%", percent_saved),
        duration_ms = duration_ms,
        "Compression completed"
    );

    Ok(CompressionResult {
        data: compressed,
        stats: CompressionStats {
            original_size,
            compressed_size,
            ratio,
            bytes_saved,
            percent_saved,
            was_compressed: true,
            level: config.level.level(),
            duration_ms,
        },
    })
}

/// Decompress gzip data
pub fn decompress_data(compressed: &[u8]) -> Result<Vec<u8>, String> {
    decompress_with_limit(compressed, 100 * 1024 * 1024) // 100 MB default limit
}

/// Decompress gzip data with a maximum output size limit
pub fn decompress_with_limit(compressed: &[u8], max_output_size: usize) -> Result<Vec<u8>, String> {
    let start = std::time::Instant::now();

    let mut decoder = GzDecoder::new(compressed);
    let mut decompressed = Vec::new();

    // Read in chunks to check size limit
    let mut buffer = [0u8; 8192];
    loop {
        let bytes_read = decoder
            .read(&mut buffer)
            .map_err(|e| format!("Decompression failed: {}", e))?;

        if bytes_read == 0 {
            break;
        }

        decompressed.extend_from_slice(&buffer[..bytes_read]);

        if decompressed.len() > max_output_size {
            return Err(format!(
                "Decompressed size exceeds maximum allowed ({} bytes)",
                max_output_size
            ));
        }
    }

    let duration_ms = start.elapsed().as_millis() as u64;

    debug!(
        compressed_size = compressed.len(),
        decompressed_size = decompressed.len(),
        duration_ms = duration_ms,
        "Decompression completed"
    );

    Ok(decompressed)
}

/// Check if data appears to be gzip compressed
pub fn is_gzip(data: &[u8]) -> bool {
    // Gzip magic number: 0x1f 0x8b
    data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b
}

/// Compress JSON data and return as base64 string
pub fn compress_json_to_base64<T: serde::Serialize>(data: &T) -> Result<String, String> {
    let json = serde_json::to_vec(data).map_err(|e| format!("JSON serialization failed: {}", e))?;

    let result = compress_data(&json)?;

    if result.stats.was_compressed {
        Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &result.data,
        ))
    } else {
        Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &json,
        ))
    }
}

/// Decompress base64-encoded gzip data to JSON
pub fn decompress_base64_to_json<T: serde::de::DeserializeOwned>(encoded: &str) -> Result<T, String> {
    let compressed = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        .map_err(|e| format!("Base64 decode failed: {}", e))?;

    let decompressed = if is_gzip(&compressed) {
        decompress_data(&compressed)?
    } else {
        compressed
    };

    serde_json::from_slice(&decompressed).map_err(|e| format!("JSON deserialization failed: {}", e))
}

/// Streaming compressor for large data
pub struct StreamingCompressor {
    encoder: flate2::write::GzEncoder<Vec<u8>>,
    bytes_written: usize,
}

impl StreamingCompressor {
    pub fn new(level: CompressionLevel) -> Self {
        Self {
            encoder: flate2::write::GzEncoder::new(Vec::new(), level.to_flate2()),
            bytes_written: 0,
        }
    }

    pub fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.encoder
            .write_all(data)
            .map_err(|e| format!("Write failed: {}", e))?;
        self.bytes_written += data.len();
        Ok(())
    }

    pub fn finish(self) -> Result<CompressionResult, String> {
        let compressed = self
            .encoder
            .finish()
            .map_err(|e| format!("Finish failed: {}", e))?;

        let compressed_size = compressed.len();
        let ratio = if self.bytes_written > 0 {
            compressed_size as f64 / self.bytes_written as f64
        } else {
            1.0
        };

        Ok(CompressionResult {
            data: compressed,
            stats: CompressionStats {
                original_size: self.bytes_written,
                compressed_size,
                ratio,
                bytes_saved: self.bytes_written as i64 - compressed_size as i64,
                percent_saved: (1.0 - ratio) * 100.0,
                was_compressed: true,
                level: 6, // Default level
                duration_ms: 0,
            },
        })
    }
}

/// Auto-compress data if beneficial, with metadata header
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressedPayload {
    /// Whether the payload is compressed
    pub compressed: bool,
    /// Compression algorithm used (if any)
    pub algorithm: Option<String>,
    /// Original size before compression
    pub original_size: usize,
    /// The data (base64 encoded if compressed)
    pub data: String,
}

impl CompressedPayload {
    /// Create a new compressed payload from data
    pub fn from_data(data: &[u8], config: &CompressionConfig) -> Result<Self, String> {
        let result = compress_with_config(data, config)?;

        if result.stats.was_compressed {
            Ok(Self {
                compressed: true,
                algorithm: Some("gzip".to_string()),
                original_size: result.stats.original_size,
                data: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &result.data,
                ),
            })
        } else {
            Ok(Self {
                compressed: false,
                algorithm: None,
                original_size: data.len(),
                data: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data),
            })
        }
    }

    /// Extract the original data from the payload
    pub fn to_data(&self) -> Result<Vec<u8>, String> {
        let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &self.data)
            .map_err(|e| format!("Base64 decode failed: {}", e))?;

        if self.compressed {
            decompress_data(&decoded)
        } else {
            Ok(decoded)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_level_conversion() {
        assert_eq!(CompressionLevel::None.level(), 0);
        assert_eq!(CompressionLevel::Fast.level(), 1);
        assert_eq!(CompressionLevel::Default.level(), 6);
        assert_eq!(CompressionLevel::Best.level(), 9);
        assert_eq!(CompressionLevel::Custom(5).level(), 5);
    }

    #[test]
    fn test_compress_decompress_roundtrip() {
        let original = b"Hello, World! This is some test data that should compress well. \
                        Lorem ipsum dolor sit amet, consectetur adipiscing elit. \
                        Repeated text repeated text repeated text repeated text.";

        let result = compress_data(original).unwrap();
        assert!(result.stats.was_compressed);
        assert!(result.stats.compressed_size < result.stats.original_size);

        let decompressed = decompress_data(&result.data).unwrap();
        assert_eq!(decompressed, original);
    }

    #[test]
    fn test_small_data_not_compressed() {
        let small_data = b"tiny";

        let result = compress_data(small_data).unwrap();
        assert!(!result.stats.was_compressed);
        assert_eq!(result.data, small_data);
    }

    #[test]
    fn test_custom_config() {
        let config = CompressionConfig::new()
            .with_level(CompressionLevel::Best)
            .with_min_size(10);

        let data = b"Some data that is larger than 10 bytes for testing";
        let result = compress_with_config(data, &config).unwrap();

        assert!(result.stats.was_compressed);
    }

    #[test]
    fn test_is_gzip() {
        let data = b"not compressed";
        let result = compress_data(data).unwrap();

        // Original data should not be gzip
        assert!(!is_gzip(data));

        // If compression was skipped, the result won't be gzip
        // If compressed, it should be gzip
        if result.stats.was_compressed {
            assert!(is_gzip(&result.data));
        }
    }

    #[test]
    fn test_compressed_payload() {
        let config = CompressionConfig::new().with_min_size(10);
        let data = b"This is test data that should be compressed into a payload";

        let payload = CompressedPayload::from_data(data, &config).unwrap();
        let recovered = payload.to_data().unwrap();

        assert_eq!(recovered, data);
    }

    #[test]
    fn test_streaming_compressor() {
        let mut compressor = StreamingCompressor::new(CompressionLevel::Default);

        compressor.write(b"Part 1 of the data ").unwrap();
        compressor.write(b"Part 2 of the data ").unwrap();
        compressor.write(b"Part 3 of the data").unwrap();

        let result = compressor.finish().unwrap();
        assert!(result.stats.was_compressed);

        let decompressed = decompress_data(&result.data).unwrap();
        assert_eq!(decompressed, b"Part 1 of the data Part 2 of the data Part 3 of the data");
    }

    #[test]
    fn test_max_input_size_limit() {
        let config = CompressionConfig {
            max_input_size: 100,
            min_size_threshold: 10,
            ..Default::default()
        };

        let large_data = vec![0u8; 200];
        let result = compress_with_config(&large_data, &config);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds maximum"));
    }

    #[test]
    fn test_decompress_size_limit() {
        // Create some compressible data
        let original = vec![b'A'; 10000];
        let result = compress_data(&original).unwrap();

        // Try to decompress with a small limit
        let decompressed = decompress_with_limit(&result.data, 100);

        if result.stats.was_compressed {
            assert!(decompressed.is_err());
        }
    }

    #[test]
    fn test_compression_stats() {
        let data = vec![b'X'; 5000];
        let result = compress_data(&data).unwrap();

        assert_eq!(result.stats.original_size, 5000);
        if result.stats.was_compressed {
            assert!(result.stats.compressed_size < 5000);
            assert!(result.stats.ratio < 1.0);
            assert!(result.stats.bytes_saved > 0);
            assert!(result.stats.percent_saved > 0.0);
        }
    }

    #[test]
    fn test_no_compression_stats() {
        let stats = CompressionStats::no_compression(1000);

        assert_eq!(stats.original_size, 1000);
        assert_eq!(stats.compressed_size, 1000);
        assert_eq!(stats.ratio, 1.0);
        assert_eq!(stats.bytes_saved, 0);
        assert!(!stats.was_compressed);
    }

    #[test]
    fn test_config_presets() {
        let fast = CompressionConfig::fast();
        assert_eq!(fast.level, CompressionLevel::Fast);

        let best = CompressionConfig::best();
        assert_eq!(best.level, CompressionLevel::Best);
    }
}
