//! SlipstreamClient - Main SDK entry point
//!
//! # Example
//!
//! ```rust,no_run
//! use allenhark_slipstream::{Config, SlipstreamClient};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = Config::builder()
//!         .api_key("sk_test_12345678")
//!         .region("us-west")
//!         .build()?;
//!
//!     let client = SlipstreamClient::connect(config).await?;
//!
//!     // Subscribe to leader hints
//!     let mut hints = client.subscribe_leader_hints().await?;
//!     while let Some(hint) = hints.recv().await {
//!         println!("Leader hint: {}", hint.preferred_region);
//!     }
//!
//!     Ok(())
//! }
//! ```

use crate::config::Config;
use crate::connection::{FallbackChain, Transport};
use crate::error::{Result, SdkError};
use crate::types::{
    ConnectionInfo, ConnectionState, ConnectionStatus, LeaderHint, PerformanceMetrics,
    PriorityFee, SubmitOptions, TipInstruction, TransactionResult,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info};

/// Slipstream client for transaction submission and streaming
pub struct SlipstreamClient {
    config: Config,
    transport: Arc<RwLock<Box<dyn Transport>>>,
    connection_info: ConnectionInfo,
    /// Cached latest tip instruction
    latest_tip: Arc<RwLock<Option<TipInstruction>>>,
    /// Performance metrics
    metrics: Arc<ClientMetrics>,
}

/// Internal metrics tracking
struct ClientMetrics {
    transactions_submitted: AtomicU64,
    transactions_confirmed: AtomicU64,
    total_latency_ms: AtomicU64,
}

impl SlipstreamClient {
    /// Connect to Slipstream using the provided configuration
    ///
    /// This will attempt to connect using the protocol fallback chain:
    /// QUIC (2s) -> gRPC (3s) -> WebSocket (3s) -> HTTP (5s)
    pub async fn connect(config: Config) -> Result<Self> {
        config.validate()?;

        info!(
            region = ?config.region,
            endpoint = ?config.endpoint,
            "Connecting to Slipstream"
        );

        let fallback_chain = FallbackChain::new(config.protocol_timeouts.clone());
        let mut transport = fallback_chain.connect(&config).await?;

        let connection_info = transport.connect(&config).await?;

        info!(
            session_id = %connection_info.session_id,
            protocol = %connection_info.protocol,
            "Connected to Slipstream"
        );

        let transport = Arc::new(RwLock::new(transport));

        // Start health monitor to handle auto-reconnection
        let monitor = crate::connection::health::HealthMonitor::new(config.clone(), transport.clone());
        monitor.start();

        Ok(Self {
            config,
            transport,
            connection_info,
            latest_tip: Arc::new(RwLock::new(None)),
            metrics: Arc::new(ClientMetrics {
                transactions_submitted: AtomicU64::new(0),
                transactions_confirmed: AtomicU64::new(0),
                total_latency_ms: AtomicU64::new(0),
            }),
        })
    }

    /// Get the current connection information
    pub fn connection_info(&self) -> &ConnectionInfo {
        &self.connection_info
    }

    /// Get the configuration
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Check if the client is connected
    pub async fn is_connected(&self) -> bool {
        let transport = self.transport.read().await;
        transport.is_connected()
    }

    /// Disconnect from the server
    pub async fn disconnect(&self) -> Result<()> {
        let mut transport = self.transport.write().await;
        transport.disconnect().await
    }

    /// Submit a transaction
    ///
    /// # Arguments
    ///
    /// * `transaction` - The signed transaction bytes
    ///
    /// # Returns
    ///
    /// Transaction result with signature (if successful) and status
    pub async fn submit_transaction(&self, transaction: &[u8]) -> Result<TransactionResult> {
        self.submit_transaction_with_options(transaction, &SubmitOptions::default())
            .await
    }

    /// Submit a transaction with custom options
    ///
    /// # Arguments
    ///
    /// * `transaction` - The signed transaction bytes
    /// * `options` - Submission options (broadcast mode, preferred sender, etc.)
    ///
    /// # Returns
    ///
    /// Transaction result with signature (if successful) and status
    pub async fn submit_transaction_with_options(
        &self,
        transaction: &[u8],
        options: &SubmitOptions,
    ) -> Result<TransactionResult> {
        debug!(
            tx_size = transaction.len(),
            broadcast_mode = options.broadcast_mode,
            preferred_sender = ?options.preferred_sender,
            "Submitting transaction"
        );

        let transport = self.transport.read().await;
        transport.submit_transaction(transaction, options).await
    }

    /// Subscribe to leader hints stream
    ///
    /// Leader hints provide recommendations for which region to use based on
    /// the current leader validator's location.
    ///
    /// # Returns
    ///
    /// A receiver channel that yields leader hints as they arrive
    pub async fn subscribe_leader_hints(&self) -> Result<mpsc::Receiver<LeaderHint>> {
        debug!("Subscribing to leader hints");
        let transport = self.transport.read().await;
        transport.subscribe_leader_hints().await
    }

    /// Subscribe to tip instructions stream
    ///
    /// Tip instructions provide the current recommended tip wallet and amount
    /// for each sender.
    ///
    /// # Returns
    ///
    /// A receiver channel that yields tip instructions as they arrive
    pub async fn subscribe_tip_instructions(&self) -> Result<mpsc::Receiver<TipInstruction>> {
        debug!("Subscribing to tip instructions");
        let transport = self.transport.read().await;
        transport.subscribe_tip_instructions().await
    }

    /// Subscribe to priority fees stream
    ///
    /// Priority fees provide recommendations for compute unit pricing based on
    /// current network congestion.
    ///
    /// # Returns
    ///
    /// A receiver channel that yields priority fees as they arrive
    pub async fn subscribe_priority_fees(&self) -> Result<mpsc::Receiver<PriorityFee>> {
        debug!("Subscribing to priority fees");
        let transport = self.transport.read().await;
        transport.subscribe_priority_fees().await
    }

    /// Get the latest cached tip instruction
    ///
    /// Returns the most recent tip instruction received from the server.
    /// This is useful for building transactions with the recommended tip.
    pub async fn get_latest_tip(&self) -> Option<TipInstruction> {
        self.latest_tip.read().await.clone()
    }

    /// Update the cached latest tip (called internally by subscription)
    pub(crate) async fn set_latest_tip(&self, tip: TipInstruction) {
        let mut latest = self.latest_tip.write().await;
        *latest = Some(tip);
    }

    /// Get current connection status
    pub async fn connection_status(&self) -> ConnectionStatus {
        let transport = self.transport.read().await;
        let is_connected = transport.is_connected();
        let protocol = transport.protocol();
        
        ConnectionStatus {
            state: if is_connected { ConnectionState::Connected } else { ConnectionState::Disconnected },
            protocol,
            latency_ms: 0, // TODO: Implement latency tracking
            region: self.connection_info.region.clone(),
        }
    }

    /// Get performance metrics
    pub fn metrics(&self) -> PerformanceMetrics {
        let submitted = self.metrics.transactions_submitted.load(Ordering::Relaxed);
        let confirmed = self.metrics.transactions_confirmed.load(Ordering::Relaxed);
        let total_latency = self.metrics.total_latency_ms.load(Ordering::Relaxed);
        
        PerformanceMetrics {
            transactions_submitted: submitted,
            transactions_confirmed: confirmed,
            average_latency_ms: if submitted > 0 { total_latency as f64 / submitted as f64 } else { 0.0 },
            success_rate: if submitted > 0 { confirmed as f64 / submitted as f64 } else { 0.0 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submit_options_default() {
        let options = SubmitOptions::default();
        assert!(!options.broadcast_mode);
        assert_eq!(options.max_retries, 2);
    }
}
