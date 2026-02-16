//! # allenhark-slipstream - Rust SDK for Slipstream
//!
//! Slipstream is a sender-agnostic Solana transaction relay system with
//! leader-proximity-aware routing.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use allenhark_slipstream::{Config, SlipstreamClient};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create configuration
//!     let config = Config::builder()
//!         .api_key("sk_test_12345678")
//!         .region("us-west")
//!         .build()?;
//!
//!     // Connect to Slipstream
//!     let client = SlipstreamClient::connect(config).await?;
//!
//!     // Submit a transaction
//!     let tx_bytes = vec![/* signed transaction bytes */];
//!     let result = client.submit_transaction(&tx_bytes).await?;
//!     println!("Transaction signature: {:?}", result.signature);
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Streaming
//!
//! The SDK supports real-time streaming of leader hints, tip instructions,
//! and priority fees:
//!
//! ```rust,no_run
//! use allenhark_slipstream::{Config, SlipstreamClient};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = Config::builder()
//!         .api_key("sk_test_12345678")
//!         .leader_hints(true)
//!         .build()?;
//!
//!     let client = SlipstreamClient::connect(config).await?;
//!
//!     // Subscribe to leader hints
//!     let mut hints = client.subscribe_leader_hints().await?;
//!     while let Some(hint) = hints.recv().await {
//!         println!("Preferred region: {} (confidence: {})",
//!             hint.preferred_region, hint.confidence);
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Protocol Fallback
//!
//! The SDK automatically falls back through protocols if the preferred one
//! is unavailable:
//!
//! 1. **QUIC** (2s timeout) - Primary high-performance protocol
//! 2. **gRPC** (3s timeout) - Streaming fallback
//! 3. **WebSocket** (3s timeout) - Browser-compatible streaming
//! 4. **HTTP** (5s timeout) - Polling fallback
//!
//! ## Features
//!
//! - **Persistent connections** - Single connection reused for all operations
//! - **Auto-reconnect** - Automatic reconnection on connection loss
//! - **Leader hints** - Real-time region recommendations
//! - **Tip instructions** - Dynamic tip wallet selection
//! - **Priority fees** - Compute unit price recommendations

pub mod client;
pub mod config;
pub mod connection;
pub mod discovery;
pub mod error;
pub mod multi_region;
pub mod types;

// Re-export main types for convenience
pub use client::SlipstreamClient;
pub use config::{BackoffStrategy, Config, ConfigBuilder, PriorityFeeConfig, Protocol, ProtocolTimeouts};
pub use error::{Result, SdkError};
pub use multi_region::MultiRegionClient;
pub use types::{
    AlternativeSender, Balance, BundleResult, ConnectionInfo, ConnectionState, ConnectionStatus,
    FallbackStrategy, FreeTierUsage, LandingRateOptions, LandingRatePeriod, LandingRateStats,
    LatestBlockhash, LatestSlot, LeaderHint, LeaderHintMetadata, MultiRegionConfig,
    PerformanceMetrics, PriorityFee, PriorityFeeSpeed, RateLimitInfo, RegionLandingRate,
    RegionStatus, RegisterWebhookRequest, RetryOptions, RoutingInfo, RoutingRecommendation,
    RpcError, RpcResponse, SenderLandingRate, SimulationResult, SubmitOptions, TipInstruction,
    TopUpInfo, TransactionBuilder, TransactionError, TransactionResult, TransactionStatus,
    UsageEntry, UsageHistoryOptions, WebhookConfig, WebhookEvent, WebhookNotificationLevel,
    WorkerEndpoint,
};

/// SDK version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn test_config_builder_accessible() {
        let builder = Config::builder();
        assert!(builder.api_key("sk_test_12345678").build().is_ok());
    }
}
