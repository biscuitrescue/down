use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

/// Unified error response structure for all API endpoints
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// API error type that can be easily converted to an Axum response
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    #[allow(dead_code)]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    // Convenience constructors for common error cases
    pub fn ipfs_upload_failed(e: impl std::fmt::Display) -> Self {
        Self::internal(format!("IPFS upload failed: {}", e))
    }

    pub fn blockchain_failed(e: impl std::fmt::Display) -> Self {
        Self::internal(format!("Blockchain transaction failed: {}", e))
    }

    pub fn json_failed(e: impl std::fmt::Display) -> Self {
        Self::internal(format!("JSON serialization failed: {}", e))
    }

    pub fn invalid_did(e: impl std::fmt::Display) -> Self {
        Self::bad_request(format!("Invalid DID format: {}", e))
    }

    pub fn invalid_hash(field: &str, e: impl std::fmt::Display) -> Self {
        Self::bad_request(format!("Invalid {}: {}", field, e))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (self.status, body).into_response()
    }
}

// Conversion from anyhow::Error for easy ? usage
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        Self::internal(e.to_string())
    }
}

/// Result type alias for API handlers
pub type ApiResult<T> = Result<Json<T>, ApiError>;

/// Helper to build IPFS gateway URL
pub fn ipfs_gateway_url(cid: &str) -> String {
    format!("https://ipfs.io/ipfs/{}", cid)
}

/// Helper to format transaction hash for response
pub fn format_tx_hash(hash: impl std::fmt::Debug) -> String {
    format!("{:?}", hash)
}

/// Helper to format bytes32 hash for response
pub fn format_hash(hash: impl std::fmt::Debug) -> String {
    format!("{:?}", hash)
}
