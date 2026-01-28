//! Connection management and protocol implementations
//!
//! This module provides the transport abstraction and protocol-specific implementations.

pub mod http;

use crate::config::{Config, Protocol, ProtocolTimeouts};
use crate::error::{Result, SdkError};
use crate::types::{
    ConnectionInfo, LeaderHint, PriorityFee, SubmitOptions, TipInstruction, TransactionResult,
};
use async_trait::async_trait;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Re-export async_trait for Transport implementors
pub use async_trait::async_trait as transport_trait;

/// Transport abstraction for different protocol backends
#[async_trait]
pub trait Transport: Send + Sync {
    /// Connect to the server
    async fn connect(&mut self, config: &Config) -> Result<ConnectionInfo>;

    /// Disconnect from the server
    async fn disconnect(&mut self) -> Result<()>;

    /// Check if connected
    fn is_connected(&self) -> bool;

    /// Get the protocol type
    fn protocol(&self) -> Protocol;

    /// Submit a transaction
    async fn submit_transaction(
        &self,
        transaction: &[u8],
        options: &SubmitOptions,
    ) -> Result<TransactionResult>;

    /// Subscribe to leader hints stream
    async fn subscribe_leader_hints(&self) -> Result<mpsc::Receiver<LeaderHint>>;

    /// Subscribe to tip instructions stream
    async fn subscribe_tip_instructions(&self) -> Result<mpsc::Receiver<TipInstruction>>;

    /// Subscribe to priority fees stream
    async fn subscribe_priority_fees(&self) -> Result<mpsc::Receiver<PriorityFee>>;
}

/// Fallback chain for protocol selection
pub struct FallbackChain {
    timeouts: ProtocolTimeouts,
}

impl FallbackChain {
    /// Create a new fallback chain with custom timeouts
    pub fn new(timeouts: ProtocolTimeouts) -> Self {
        Self { timeouts }
    }

    /// Get the timeout for a protocol
    pub fn timeout_for(&self, protocol: Protocol) -> Duration {
        match protocol {
            Protocol::Quic => self.timeouts.quic,
            Protocol::Grpc => self.timeouts.grpc,
            Protocol::WebSocket => self.timeouts.websocket,
            Protocol::Http => self.timeouts.http,
        }
    }

    /// Attempt to connect using the fallback chain
    pub async fn connect(&self, config: &Config) -> Result<Box<dyn Transport>> {
        // If a preferred protocol is set, only try that one
        if let Some(preferred) = config.preferred_protocol {
            return self.try_protocol(config, preferred).await;
        }

        // Try protocols in fallback order
        let mut last_error = None;
        for protocol in Protocol::fallback_order() {
            match self.try_protocol(config, *protocol).await {
                Ok(transport) => {
                    info!(protocol = ?protocol, "Connected successfully");
                    return Ok(transport);
                }
                Err(e) => {
                    warn!(protocol = ?protocol, error = %e, "Protocol failed, trying next");
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or(SdkError::AllProtocolsFailed))
    }

    /// Try to connect with a specific protocol
    async fn try_protocol(&self, config: &Config, protocol: Protocol) -> Result<Box<dyn Transport>> {
        let timeout = self.timeout_for(protocol);
        debug!(protocol = ?protocol, timeout_ms = timeout.as_millis(), "Attempting protocol");

        match protocol {
            Protocol::Http => {
                let mut transport = http::HttpTransport::new();
                tokio::time::timeout(timeout, transport.connect(config))
                    .await
                    .map_err(|_| SdkError::Timeout(timeout))??;
                Ok(Box::new(transport))
            }
            Protocol::WebSocket => {
                // WebSocket implementation will be added in Phase 2
                Err(SdkError::protocol("WebSocket not yet implemented"))
            }
            Protocol::Grpc => {
                // gRPC implementation will be added in Phase 3
                Err(SdkError::protocol("gRPC not yet implemented"))
            }
            Protocol::Quic => {
                // QUIC implementation will be added in Phase 4
                Err(SdkError::protocol("QUIC not yet implemented"))
            }
        }
    }
}

impl Default for FallbackChain {
    fn default() -> Self {
        Self::new(ProtocolTimeouts::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_chain_timeouts() {
        let chain = FallbackChain::default();
        assert_eq!(chain.timeout_for(Protocol::Quic), Duration::from_millis(2000));
        assert_eq!(chain.timeout_for(Protocol::Grpc), Duration::from_millis(3000));
        assert_eq!(chain.timeout_for(Protocol::WebSocket), Duration::from_millis(3000));
        assert_eq!(chain.timeout_for(Protocol::Http), Duration::from_millis(5000));
    }
}
