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
use crate::discovery::DiscoveryClient;
use crate::error::{Result, SdkError};
use crate::types::{
    Balance, BundleResult, ConnectionInfo, ConnectionState, ConnectionStatus, FallbackStrategy,
    LandingRateOptions, LandingRateStats, LatestBlockhash, LatestSlot, LeaderHint,
    PerformanceMetrics, PingResult, PriorityFee, RegisterWebhookRequest, RpcResponse,
    RoutingRecommendation, SimulationResult, SubmitOptions, TipInstruction, TopUpInfo,
    TransactionResult, UsageEntry, UsageHistoryOptions, WebhookConfig,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

/// Tracks ping results for latency and clock synchronization
struct TimeSyncState {
    samples: std::sync::RwLock<Vec<PingResult>>,
    max_samples: usize,
}

impl TimeSyncState {
    fn new(max_samples: usize) -> Self {
        Self {
            samples: std::sync::RwLock::new(Vec::with_capacity(max_samples)),
            max_samples,
        }
    }

    fn record(&self, result: PingResult) {
        let mut samples = self.samples.write().unwrap();
        if samples.len() >= self.max_samples {
            samples.remove(0);
        }
        samples.push(result);
    }

    fn median_rtt_ms(&self) -> Option<u64> {
        let samples = self.samples.read().unwrap();
        if samples.is_empty() {
            return None;
        }
        let mut rtts: Vec<u64> = samples.iter().map(|s| s.rtt_ms).collect();
        rtts.sort();
        Some(rtts[rtts.len() / 2])
    }

    fn median_clock_offset_ms(&self) -> Option<i64> {
        let samples = self.samples.read().unwrap();
        if samples.is_empty() {
            return None;
        }
        let mut offsets: Vec<i64> = samples.iter().map(|s| s.clock_offset_ms).collect();
        offsets.sort();
        Some(offsets[offsets.len() / 2])
    }
}

/// Client-side deduplication cache
///
/// Tracks recently submitted transaction signatures to prevent double-submission.
/// Entries expire after 60 seconds.
struct DedupCache {
    entries: std::sync::RwLock<HashMap<Vec<u8>, Instant>>,
    ttl: std::time::Duration,
}

impl DedupCache {
    fn new() -> Self {
        Self {
            entries: std::sync::RwLock::new(HashMap::new()),
            ttl: std::time::Duration::from_secs(60),
        }
    }

    /// Check if a transaction was recently submitted. Returns true if duplicate.
    fn is_duplicate(&self, tx_bytes: &[u8]) -> bool {
        let entries = self.entries.read().unwrap();
        if let Some(submitted_at) = entries.get(tx_bytes) {
            submitted_at.elapsed() < self.ttl
        } else {
            false
        }
    }

    /// Record a submitted transaction
    fn record(&self, tx_bytes: &[u8]) {
        let mut entries = self.entries.write().unwrap();
        entries.insert(tx_bytes.to_vec(), Instant::now());

        // Cleanup expired entries if cache is getting large
        if entries.len() > 1000 {
            let ttl = self.ttl;
            entries.retain(|_, submitted_at| submitted_at.elapsed() < ttl);
        }
    }
}

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
    /// Time synchronization state from keep-alive pings
    time_sync: Arc<TimeSyncState>,
    /// Keep-alive background task handle (Mutex for interior mutability)
    keepalive_handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Client-side deduplication cache
    dedup_cache: Arc<DedupCache>,
    /// Subscription tracker for auto-resubscribe on reconnect
    subscription_tracker: Arc<crate::connection::health::SubscriptionTracker>,
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
    /// If no explicit endpoint is set, the SDK automatically discovers
    /// available workers via the discovery service, selects the best
    /// worker based on region preference and health, and connects directly
    /// to the worker's IP address.
    ///
    /// Protocol fallback chain: QUIC (2s) -> gRPC (3s) -> WebSocket (3s) -> HTTP (5s)
    pub async fn connect(mut config: Config) -> Result<Self> {
        config.validate()?;

        // If no explicit endpoint or worker is set, use discovery
        if config.endpoint.is_none() && config.selected_worker.is_none() {
            info!(
                discovery_url = %config.discovery_url,
                region = ?config.region,
                "Discovering workers"
            );

            let discovery = DiscoveryClient::new(&config.discovery_url);
            let response = discovery.discover().await?;

            // Pick the best region
            let region = discovery
                .best_region(&response, config.region.as_deref())
                .ok_or_else(|| SdkError::connection("No healthy workers found via discovery"))?;

            // Get healthy workers in that region
            let region_workers = discovery.workers_for_region(&response, &region);
            if region_workers.is_empty() {
                return Err(SdkError::connection(format!(
                    "No healthy workers in region '{}'",
                    region
                )));
            }

            // Select the first healthy worker (latency-based selection happens at transport level)
            let worker = &region_workers[0];
            let endpoint = DiscoveryClient::worker_to_endpoint(worker);

            info!(
                worker_id = %endpoint.id,
                region = %region,
                ip = %worker.ip,
                "Selected worker via discovery"
            );

            config.selected_worker = Some(endpoint);
            config.region = Some(region);
        }

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

        // Start health monitor to handle auto-reconnection with subscription tracking
        let subscription_tracker = Arc::new(crate::connection::health::SubscriptionTracker::new());
        let monitor = crate::connection::health::HealthMonitor::new(
            config.clone(),
            transport.clone(),
            Arc::clone(&subscription_tracker),
        );
        monitor.start();

        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| SdkError::connection(format!("Failed to create HTTP client: {}", e)))?;

        let time_sync = Arc::new(TimeSyncState::new(10));
        let keepalive_handle = if config.keepalive {
            let transport_clone = Arc::clone(&transport);
            let sync_clone = Arc::clone(&time_sync);
            let interval = config.keepalive_interval;
            Some(tokio::spawn(async move {
                let mut ticker = tokio::time::interval(interval);
                loop {
                    ticker.tick().await;
                    let transport = transport_clone.read().await;
                    if !transport.is_connected() {
                        break;
                    }
                    match transport.ping().await {
                        Ok(result) => {
                            debug!(rtt_ms = result.rtt_ms, offset_ms = result.clock_offset_ms, "Keepalive ping");
                            sync_clone.record(result);
                        }
                        Err(_) => {
                            // Transport may not support ping -- that's OK
                            break;
                        }
                    }
                }
            }))
        } else {
            None
        };

        let client = Self {
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
            time_sync,
            keepalive_handle: tokio::sync::Mutex::new(keepalive_handle),
            dedup_cache: Arc::new(DedupCache::new()),
            subscription_tracker,
        };

        // Auto-register webhook if configured
        if client.config.webhook_url.is_some() {
            let url = client.config.webhook_url.clone().unwrap();
            let events = client.config.webhook_events.clone();
            let level = client.config.webhook_notification_level.clone();
            match client
                .register_webhook(&url, Some(events), Some(level))
                .await
            {
                Ok(_) => info!("Webhook auto-registered at {}", url),
                Err(e) => debug!("Failed to auto-register webhook: {}", e),
            }
        }

        Ok(client)
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
        if let Some(handle) = self.keepalive_handle.lock().await.take() {
            handle.abort();
        }
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
        // Client-side deduplication check
        if self.dedup_cache.is_duplicate(transaction) {
            warn!(tx_size = transaction.len(), "Duplicate transaction detected by client-side cache");
            return Err(SdkError::Transaction(
                "Duplicate transaction: already submitted within the last 60 seconds".to_string(),
            ));
        }

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
            self.dedup_cache.record(transaction);
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
        self.subscription_tracker
            .track(crate::connection::health::StreamType::LeaderHints);
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
        self.subscription_tracker
            .track(crate::connection::health::StreamType::TipInstructions);
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
        self.subscription_tracker
            .track(crate::connection::health::StreamType::PriorityFees);
        let transport = self.transport.read().await;
        transport.subscribe_priority_fees().await
    }

    /// Subscribe to latest blockhash stream
    ///
    /// Latest blockhash provides the most recent blockhash for transaction building,
    /// updated as new blocks are produced.
    ///
    /// # Returns
    ///
    /// A receiver channel that yields latest blockhash updates as they arrive
    pub async fn subscribe_latest_blockhash(&self) -> Result<mpsc::Receiver<LatestBlockhash>> {
        debug!("Subscribing to latest blockhash");
        self.subscription_tracker
            .track(crate::connection::health::StreamType::LatestBlockhash);
        let transport = self.transport.read().await;
        transport.subscribe_latest_blockhash().await
    }

    /// Subscribe to latest slot stream
    ///
    /// Latest slot provides the current slot number, updated in real-time
    /// as slots advance.
    ///
    /// # Returns
    ///
    /// A receiver channel that yields latest slot updates as they arrive
    pub async fn subscribe_latest_slot(&self) -> Result<mpsc::Receiver<LatestSlot>> {
        debug!("Subscribing to latest slot");
        self.subscription_tracker
            .track(crate::connection::health::StreamType::LatestSlot);
        let transport = self.transport.read().await;
        transport.subscribe_latest_slot().await
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
            leader_pubkey: String::new(),
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

    /// Get deposit history for the authenticated API key
    ///
    /// Returns a list of SOL deposits with credited/pending status.
    pub async fn get_deposit_history(
        &self,
        options: crate::types::DepositHistoryOptions,
    ) -> Result<Vec<crate::types::DepositEntry>> {
        let base_url = self.config.get_endpoint(crate::types::Protocol::Http);
        let mut url = format!("{}/v1/deposit-history", base_url);

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

        debug!(url = %url, "Fetching deposit history");

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
            return Err(SdkError::Internal(format!(
                "Failed to fetch deposit history: {}",
                error_text
            )));
        }

        let body: serde_json::Value = response.json().await?;
        let entries: Vec<crate::types::DepositEntry> = serde_json::from_value(
            body["deposits"].clone(),
        )
        .unwrap_or_default();

        Ok(entries)
    }

    /// Get pending (uncredited) deposit information
    ///
    /// Returns the total uncredited deposit amount and the $10 USD minimum.
    pub async fn get_pending_deposit(&self) -> Result<crate::types::PendingDeposit> {
        let base_url = self.config.get_endpoint(crate::types::Protocol::Http);
        let url = format!("{}/v1/deposit-pending", base_url);
        debug!(url = %url, "Fetching pending deposit info");

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
            return Err(SdkError::Internal(format!(
                "Failed to fetch pending deposits: {}",
                error_text
            )));
        }

        let pending: crate::types::PendingDeposit = response.json().await?;
        Ok(pending)
    }

    /// Get free tier daily usage statistics
    ///
    /// Returns the number of transactions used today, remaining quota,
    /// and when the counter resets (UTC midnight).
    /// Only meaningful for keys on the 'free' tier.
    pub async fn get_free_tier_usage(&self) -> Result<crate::types::FreeTierUsage> {
        let base_url = self.config.get_endpoint(crate::types::Protocol::Http);
        let url = format!("{}/v1/free-tier-usage", base_url);
        debug!(url = %url, "Fetching free tier usage");

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
            return Err(SdkError::Internal(format!(
                "Failed to fetch free tier usage: {}",
                error_text
            )));
        }

        let usage: crate::types::FreeTierUsage = response.json().await?;
        Ok(usage)
    }

    /// Get the minimum deposit amount in USD
    ///
    /// Returns the minimum USD equivalent of SOL that must be deposited
    /// before tokens are credited. Currently $10 USD.
    pub fn get_minimum_deposit_usd(&self) -> f64 {
        10.0
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

    // ========================================================================
    // Bundle Submission
    // ========================================================================

    /// Submit a bundle of transactions for atomic execution
    ///
    /// Bundles contain 2-5 transactions that are executed atomically — either
    /// all succeed or none. The sender must support bundle submission.
    ///
    /// # Arguments
    ///
    /// * `transactions` - 2 to 5 signed transactions (raw bytes)
    ///
    /// # Returns
    ///
    /// Bundle result with bundle ID, acceptance status, and individual signatures
    ///
    /// # Billing
    ///
    /// Bundle submission costs 5 tokens (0.00025 SOL) per bundle, regardless of
    /// the number of transactions in the bundle.
    pub async fn submit_bundle(&self, transactions: &[Vec<u8>]) -> Result<BundleResult> {
        self.submit_bundle_with_tip(transactions, None).await
    }

    /// Submit a bundle with an optional tip amount
    ///
    /// # Arguments
    ///
    /// * `transactions` - 2 to 5 signed transactions (raw bytes)
    /// * `tip_lamports` - Optional tip amount in lamports
    pub async fn submit_bundle_with_tip(
        &self,
        transactions: &[Vec<u8>],
        tip_lamports: Option<u64>,
    ) -> Result<BundleResult> {
        if transactions.len() < 2 || transactions.len() > 5 {
            return Err(SdkError::config(
                "Bundle must contain 2-5 transactions",
            ));
        }

        debug!(
            tx_count = transactions.len(),
            tip = ?tip_lamports,
            "Submitting bundle"
        );

        let base_url = self.config.get_endpoint(crate::types::Protocol::Http);
        let url = format!("{}/v1/bundles/submit", base_url);

        let txs_base64: Vec<String> = transactions
            .iter()
            .map(|tx| base64::Engine::encode(&base64::engine::general_purpose::STANDARD, tx))
            .collect();

        let body = serde_json::json!({
            "transactions": txs_base64,
            "tip_lamports": tip_lamports,
        });

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&body)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SdkError::auth("Invalid API key"));
        }

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(SdkError::Internal(format!(
                "Failed to submit bundle: {}",
                error_text
            )));
        }

        let result: BundleResult = response.json().await?;
        Ok(result)
    }

    // ========================================================================
    // Keep-Alive & Time Sync Methods
    // ========================================================================

    /// Get estimated one-way latency in milliseconds (RTT/2)
    pub fn latency_ms(&self) -> Option<u64> {
        self.time_sync.median_rtt_ms().map(|rtt| rtt / 2)
    }

    /// Get estimated clock offset between client and server (milliseconds)
    pub fn clock_offset_ms(&self) -> Option<i64> {
        self.time_sync.median_clock_offset_ms()
    }

    /// Get estimated current server time (unix millis), corrected by clock offset
    pub fn server_time(&self) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let offset = self.time_sync.median_clock_offset_ms().unwrap_or(0);
        (now as i64 + offset) as u64
    }

    /// Send a single ping and return the result
    pub async fn ping(&self) -> Result<PingResult> {
        let transport = self.transport.read().await;
        let result = transport.ping().await?;
        self.time_sync.record(result.clone());
        Ok(result)
    }

    // ========================================================================
    // Webhook Methods
    // ========================================================================

    /// Register or update a webhook for this API key
    ///
    /// If a webhook already exists for this key, it will be updated.
    /// Returns the webhook configuration including the secret (only visible
    /// on register/update).
    pub async fn register_webhook(
        &self,
        url: &str,
        events: Option<Vec<String>>,
        notification_level: Option<String>,
    ) -> Result<WebhookConfig> {
        let base_url = self.config.get_endpoint(crate::types::Protocol::Http);
        let api_url = format!("{}/v1/webhooks", base_url);

        let request = RegisterWebhookRequest {
            url: url.to_string(),
            events,
            notification_level,
        };

        let response = self
            .http_client
            .post(&api_url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&request)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SdkError::auth("Invalid API key"));
        }

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(SdkError::Internal(format!(
                "Failed to register webhook: {}",
                error_text
            )));
        }

        let config: WebhookConfig = response.json().await?;
        Ok(config)
    }

    /// Get current webhook configuration for this API key
    ///
    /// Returns the webhook configuration with the secret masked.
    /// Returns None if no webhook is configured.
    pub async fn get_webhook(&self) -> Result<Option<WebhookConfig>> {
        let base_url = self.config.get_endpoint(crate::types::Protocol::Http);
        let url = format!("{}/v1/webhooks", base_url);

        let response = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SdkError::auth("Invalid API key"));
        }

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(SdkError::Internal(format!(
                "Failed to get webhook: {}",
                error_text
            )));
        }

        let config: WebhookConfig = response.json().await?;
        Ok(Some(config))
    }

    /// Delete (disable) the webhook for this API key
    pub async fn delete_webhook(&self) -> Result<()> {
        let base_url = self.config.get_endpoint(crate::types::Protocol::Http);
        let url = format!("{}/v1/webhooks", base_url);

        let response = self
            .http_client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SdkError::auth("Invalid API key"));
        }

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(SdkError::Internal(format!(
                "Failed to delete webhook: {}",
                error_text
            )));
        }

        Ok(())
    }

    // ========================================================================
    // Landing Rates
    // ========================================================================

    /// Get transaction landing rate statistics for this API key.
    ///
    /// Returns overall landing rate plus per-sender and per-region breakdowns.
    /// Defaults to the last 24 hours if no time range is specified.
    pub async fn get_landing_rates(
        &self,
        options: Option<LandingRateOptions>,
    ) -> Result<LandingRateStats> {
        let base_url = self.config.get_endpoint(crate::types::Protocol::Http);
        let mut url = format!("{}/v1/metrics/landing-rates", base_url);

        if let Some(opts) = &options {
            let mut params = Vec::new();
            if let Some(start) = &opts.start {
                params.push(format!("start={}", start));
            }
            if let Some(end) = &opts.end {
                params.push(format!("end={}", end));
            }
            if !params.is_empty() {
                url.push('?');
                url.push_str(&params.join("&"));
            }
        }

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
            return Err(SdkError::Internal(format!(
                "Failed to get landing rates: {}",
                error_text
            )));
        }

        let stats: LandingRateStats = response.json().await?;
        Ok(stats)
    }

    // === Solana RPC Proxy ===

    /// Execute a Solana JSON-RPC call via the Slipstream proxy.
    ///
    /// Costs 1 token (0.00005 SOL) per call. Only methods from the allowlist
    /// are supported (simulateTransaction, getTransaction, getBalance, etc.).
    ///
    /// Returns the raw JSON-RPC 2.0 response.
    pub async fn rpc(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<RpcResponse> {
        let base_url = self.config.get_endpoint(crate::types::Protocol::Http);
        let url = format!("{}/v1/rpc", base_url);

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&body)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SdkError::auth("Invalid API key"));
        }

        if response.status() == reqwest::StatusCode::PAYMENT_REQUIRED {
            return Err(SdkError::Internal(
                "Insufficient token balance for RPC query".to_string(),
            ));
        }

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(SdkError::Internal(format!("RPC proxy error: {}", error_text)));
        }

        let rpc_response: RpcResponse = response.json().await?;
        Ok(rpc_response)
    }

    /// Simulate a transaction without submitting it to the network.
    ///
    /// Costs 1 token. Returns simulation result with logs and compute units.
    pub async fn simulate_transaction(&self, transaction: &[u8]) -> Result<SimulationResult> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let tx_b64 = STANDARD.encode(transaction);

        let response = self
            .rpc(
                "simulateTransaction",
                serde_json::json!([tx_b64, {
                    "encoding": "base64",
                    "commitment": "confirmed",
                    "replaceRecentBlockhash": true,
                }]),
            )
            .await?;

        if let Some(error) = response.error {
            return Err(SdkError::Internal(format!(
                "RPC error {}: {}",
                error.code, error.message
            )));
        }

        let result_value = response
            .result
            .ok_or_else(|| SdkError::Internal("No result in RPC response".to_string()))?;

        // simulateTransaction returns { value: { err, logs, unitsConsumed, returnData } }
        let value = result_value
            .get("value")
            .cloned()
            .unwrap_or(result_value);

        let sim: SimulationResult = serde_json::from_value(value)
            .map_err(|e| SdkError::Internal(format!("Failed to parse simulation result: {}", e)))?;

        Ok(sim)
    }

    /// Simulate each transaction in a bundle sequentially.
    ///
    /// Costs 1 token per transaction simulated. Stops on first failure.
    /// Returns a Vec of SimulationResult (one per transaction attempted).
    pub async fn simulate_bundle(
        &self,
        transactions: &[Vec<u8>],
    ) -> Result<Vec<SimulationResult>> {
        let mut results = Vec::with_capacity(transactions.len());

        for tx in transactions {
            let sim = self.simulate_transaction(tx).await?;
            let failed = sim.err.is_some();
            results.push(sim);
            if failed {
                break;
            }
        }

        Ok(results)
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
