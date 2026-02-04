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
    Balance, ConnectionInfo, ConnectionState, ConnectionStatus, FallbackStrategy, LeaderHint,
    PerformanceMetrics, PriorityFee, RoutingRecommendation, SubmitOptions, TipInstruction,
    TopUpInfo, TransactionResult, UsageEntry, UsageHistoryOptions,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info};

/// Slipstream client for transaction submission and streaming
pub struct SlipstreamClient {
    config: Config,
    transport: Arc<RwLock<Box<dyn Transport>>>,
    connection_info: ConnectionInfo,
    /// HTTP client for REST API calls (billing, etc.)
    http_client: reqwest::Client,
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
    last_latency_ms: AtomicU64,
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

        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| SdkError::connection(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            config,
            transport,
            connection_info,
            http_client,
            latest_tip: Arc::new(RwLock::new(None)),
            metrics: Arc::new(ClientMetrics {
                transactions_submitted: AtomicU64::new(0),
                transactions_confirmed: AtomicU64::new(0),
                total_latency_ms: AtomicU64::new(0),
                last_latency_ms: AtomicU64::new(0),
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

        let start = Instant::now();
        let transport = self.transport.read().await;
        let result = transport.submit_transaction(transaction, options).await;
        let elapsed_ms = start.elapsed().as_millis() as u64;

        self.metrics.last_latency_ms.store(elapsed_ms, Ordering::Relaxed);
        self.metrics.transactions_submitted.fetch_add(1, Ordering::Relaxed);
        self.metrics.total_latency_ms.fetch_add(elapsed_ms, Ordering::Relaxed);

        if result.is_ok() {
            self.metrics.transactions_confirmed.fetch_add(1, Ordering::Relaxed);
        }

        result
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
            latency_ms: self.metrics.last_latency_ms.load(Ordering::Relaxed),
            region: self.connection_info.region.clone(),
        }
    }

    // ========================================================================
    // Multi-Region Routing Methods
    // ========================================================================

    /// Get routing recommendation for leader-aware transaction submission
    ///
    /// Returns the best region for the current leader validator, along with
    /// fallback options and confidence scores. This is useful for bots that
    /// want to dynamically route transactions based on validator proximity.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use allenhark_slipstream::{Config, SlipstreamClient};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = Config::builder().api_key("test").build()?;
    /// # let client = SlipstreamClient::connect(config).await?;
    /// let recommendation = client.get_routing_recommendation().await?;
    /// println!("Best region: {} (confidence: {}%)",
    ///     recommendation.best_region, recommendation.confidence);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_routing_recommendation(&self) -> Result<RoutingRecommendation> {
        let base_url = self.config.get_endpoint(crate::types::Protocol::Http);
        let url = format!("{}/v1/routing/recommendation", base_url);
        debug!(url = %url, "Fetching routing recommendation");

        let response = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SdkError::auth("Invalid API key"));
        }

        if !response.status().is_success() {
            // If the endpoint doesn't exist, return a default recommendation
            // based on current connection info
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                debug!("Routing endpoint not available, using local fallback");
                return Ok(self.create_local_routing_recommendation().await);
            }
            let error_text = response.text().await.unwrap_or_default();
            return Err(SdkError::Internal(format!(
                "Failed to fetch routing recommendation: {}",
                error_text
            )));
        }

        let recommendation: RoutingRecommendation = response.json().await?;
        Ok(recommendation)
    }

    /// Create a local routing recommendation when server endpoint is unavailable
    async fn create_local_routing_recommendation(&self) -> RoutingRecommendation {
        RoutingRecommendation {
            best_region: self
                .connection_info
                .region
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            leader_pubkey: None,
            slot: 0,
            confidence: 50, // Low confidence for local fallback
            expected_rtt_ms: None,
            fallback_regions: vec![],
            fallback_strategy: FallbackStrategy::Retry,
            valid_for_ms: 1000,
        }
    }

    // ========================================================================
    // Token Billing Methods
    // ========================================================================

    /// Get token balance for the authenticated API key
    ///
    /// Returns the current balance in SOL, tokens, and lamports.
    pub async fn get_balance(&self) -> Result<Balance> {
        let base_url = self.config.get_endpoint(crate::types::Protocol::Http);
        let url = format!("{}/v1/balance", base_url);
        debug!(url = %url, "Fetching token balance");

        let response = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SdkError::auth("Invalid API key"));
        }

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(SdkError::Internal(format!("Failed to fetch balance: {}", error_text)));
        }

        let body: serde_json::Value = response.json().await?;

        let balance_lamports = body["balance_lamports"].as_i64().unwrap_or(0);
        let cost_per_query = 50_000i64; // 50,000 lamports per query
        let grace_limit = 1_000_000i64; // 1M lamports grace

        Ok(Balance {
            balance_sol: balance_lamports as f64 / 1_000_000_000.0,
            balance_tokens: balance_lamports / cost_per_query,
            balance_lamports,
            grace_remaining_tokens: (balance_lamports + grace_limit) / cost_per_query,
        })
    }

    /// Get deposit address for topping up the token balance
    ///
    /// Returns the Solana wallet address where SOL can be sent
    /// and the minimum top-up amount.
    pub async fn get_deposit_address(&self) -> Result<TopUpInfo> {
        let base_url = self.config.get_endpoint(crate::types::Protocol::Http);
        let url = format!("{}/v1/deposit-address", base_url);
        debug!(url = %url, "Fetching deposit address");

        let response = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SdkError::auth("Invalid API key"));
        }

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(SdkError::Internal(format!("Failed to fetch deposit address: {}", error_text)));
        }

        let info: TopUpInfo = response.json().await?;
        Ok(info)
    }

    /// Get token usage history for the authenticated API key
    ///
    /// Returns a paginated list of token transactions (debits, credits, deposits).
    pub async fn get_usage_history(&self, options: UsageHistoryOptions) -> Result<Vec<UsageEntry>> {
        let base_url = self.config.get_endpoint(crate::types::Protocol::Http);
        let mut url = format!("{}/v1/usage-history", base_url);

        let mut params = Vec::new();
        if let Some(limit) = options.limit {
            params.push(format!("limit={}", limit));
        }
        if let Some(offset) = options.offset {
            params.push(format!("offset={}", offset));
        }
        if !params.is_empty() {
            url = format!("{}?{}", url, params.join("&"));
        }

        debug!(url = %url, "Fetching usage history");

        let response = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SdkError::auth("Invalid API key"));
        }

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(SdkError::Internal(format!("Failed to fetch usage history: {}", error_text)));
        }

        let body: serde_json::Value = response.json().await?;
        let entries: Vec<UsageEntry> = serde_json::from_value(
            body["entries"].clone(),
        )
        .unwrap_or_default();

        Ok(entries)
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
