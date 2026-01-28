//! Error types for the Slipstream SDK

use std::time::Duration;
use thiserror::Error;

/// SDK error types
#[derive(Debug, Error)]
pub enum SdkError {
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Connection error
    #[error("Connection error: {0}")]
    Connection(String),

    /// Authentication failed
    #[error("Authentication failed: {0}")]
    Auth(String),

    /// Protocol error
    #[error("Protocol error: {0}")]
    Protocol(String),

    /// Transaction submission error
    #[error("Transaction error: {0}")]
    Transaction(String),

    /// Operation timed out
    #[error("Timeout after {0:?}")]
    Timeout(Duration),

    /// All protocol fallbacks failed
    #[error("All protocols failed to connect")]
    AllProtocolsFailed,

    /// Rate limited
    #[error("Rate limited: {0}")]
    RateLimited(String),

    /// Not connected
    #[error("Not connected to server")]
    NotConnected,

    /// Stream closed
    #[error("Stream closed")]
    StreamClosed,

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

impl SdkError {
    /// Create a configuration error
    pub fn config(msg: impl Into<String>) -> Self {
        SdkError::Config(msg.into())
    }

    /// Create a connection error
    pub fn connection(msg: impl Into<String>) -> Self {
        SdkError::Connection(msg.into())
    }

    /// Create an authentication error
    pub fn auth(msg: impl Into<String>) -> Self {
        SdkError::Auth(msg.into())
    }

    /// Create a protocol error
    pub fn protocol(msg: impl Into<String>) -> Self {
        SdkError::Protocol(msg.into())
    }

    /// Create a transaction error
    pub fn transaction(msg: impl Into<String>) -> Self {
        SdkError::Transaction(msg.into())
    }
}

impl From<reqwest::Error> for SdkError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            SdkError::Timeout(Duration::from_secs(30))
        } else if e.is_connect() {
            SdkError::Connection(e.to_string())
        } else {
            SdkError::Protocol(e.to_string())
        }
    }
}

impl From<serde_json::Error> for SdkError {
    fn from(e: serde_json::Error) -> Self {
        SdkError::Protocol(format!("JSON error: {}", e))
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for SdkError {
    fn from(e: tokio_tungstenite::tungstenite::Error) -> Self {
        SdkError::Connection(format!("WebSocket error: {}", e))
    }
}

impl From<std::io::Error> for SdkError {
    fn from(e: std::io::Error) -> Self {
        SdkError::Connection(format!("IO error: {}", e))
    }
}

/// Result type for SDK operations
pub type Result<T> = std::result::Result<T, SdkError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = SdkError::Config("missing api_key".to_string());
        assert_eq!(err.to_string(), "Configuration error: missing api_key");

        let err = SdkError::Timeout(Duration::from_secs(5));
        assert_eq!(err.to_string(), "Timeout after 5s");
    }

    #[test]
    fn test_error_constructors() {
        let err = SdkError::config("test");
        assert!(matches!(err, SdkError::Config(_)));

        let err = SdkError::auth("invalid key");
        assert!(matches!(err, SdkError::Auth(_)));
    }
}
