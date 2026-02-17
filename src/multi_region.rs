//! Multi-Region Client for leader-aware transaction routing
//!
//! This module provides a `MultiRegionClient` that automatically routes transactions
//! to the optimal region based on the current Solana leader validator's location.
//!
//! # Example
//!
//! ```rust,ignore
//! use allenhark_slipstream::{Config, MultiRegionClient, MultiRegionConfig, WorkerEndpoint};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = Config::builder()
//!         .api_key("sk_test_12345678")
//!         .build()?;
//!
//!     let workers = vec![
//!         WorkerEndpoint::new("w1", "us-west", "203.0.113.10"),
//!         WorkerEndpoint::new("w2", "eu-central", "203.0.113.20"),
//!         WorkerEndpoint::new("w3", "asia-east", "203.0.113.30"),
//!     ];
//!
//!     let multi_config = MultiRegionConfig::default();
//!     let client = MultiRegionClient::new(config, workers, multi_config).await?;
//!
//!     // Subscribe to routing updates
//!     let mut routing_rx = client.subscribe_routing_updates();
//!
//!     // Submit transaction - automatically routed to best region
//!     let tx_bytes = vec![/* signed transaction bytes */];
//!     let result = client.submit_transaction(&tx_bytes).await?;
//!
//!     Ok(())
//! }
//! ```

use crate::config::Config;
use crate::connection::selector::WorkerSelector;
use crate::discovery::DiscoveryClient;
use crate::error::{Result, SdkError};
use crate::types::{
    FallbackStrategy, LeaderHint, MultiRegionConfig, RoutingRecommendation, SubmitOptions,
    TransactionResult, WorkerEndpoint,
};
use crate::SlipstreamClient;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, info, warn};

/// Multi-region client for leader-aware transaction routing
///
/// This client maintains connections to workers in multiple regions and
/// automatically routes transactions to the region with the best proximity
/// to the current Solana leader validator.
pub struct MultiRegionClient {
    /// Configuration
    config: Config,
    /// Multi-region specific configuration
    multi_config: MultiRegionConfig,
    /// Worker selector for latency-based selection
    worker_selector: Arc<WorkerSelector>,
    /// Active clients per region (lazily connected)
    region_clients: Arc<RwLock<HashMap<String, Arc<SlipstreamClient>>>>,
    /// Current routing recommendation
    current_routing: Arc<RwLock<Option<RoutingRecommendation>>>,
    /// Last region switch time
    last_switch: Arc<RwLock<Option<Instant>>>,
    /// Routing update broadcaster
    routing_tx: broadcast::Sender<RoutingRecommendation>,
    /// Leader hint receiver (from primary connection)
    leader_hint_tx: Arc<RwLock<Option<mpsc::Sender<LeaderHint>>>>,
}

impl MultiRegionClient {
    /// Connect using automatic worker discovery
    ///
    /// Discovers all available workers across regions via the discovery
    /// service and creates a multi-region client. No manual worker
    /// configuration is needed.
    pub async fn connect(config: Config) -> Result<Self> {
        Self::connect_with_config(config, MultiRegionConfig::default()).await
    }

    /// Connect with custom multi-region settings
    pub async fn connect_with_config(
        config: Config,
        multi_config: MultiRegionConfig,
    ) -> Result<Self> {
        info!(
            discovery_url = %config.discovery_url,
            "Discovering workers for multi-region client"
        );

        let discovery = DiscoveryClient::new(&config.discovery_url);
        let response = discovery.discover().await?;

        let workers = DiscoveryClient::to_worker_endpoints(&response.workers);
        if workers.is_empty() {
            return Err(SdkError::connection("No healthy workers found via discovery"));
        }

        info!(
            worker_count = workers.len(),
            region_count = response.regions.len(),
            "Discovered workers for multi-region"
        );

        Self::new(config, workers, multi_config).await
    }

    /// Create a new multi-region client with explicit worker list
    ///
    /// # Arguments
    ///
    /// * `config` - Base SDK configuration (API key, etc.)
    /// * `workers` - Available worker endpoints across regions
    /// * `multi_config` - Multi-region specific settings
    pub async fn new(
        config: Config,
        workers: Vec<WorkerEndpoint>,
        multi_config: MultiRegionConfig,
    ) -> Result<Self> {
        if workers.is_empty() {
            return Err(SdkError::config("No workers provided for multi-region client"));
        }

        let worker_selector = Arc::new(WorkerSelector::new(workers));
        let (routing_tx, _) = broadcast::channel(16);

        let client = Self {
            config,
            multi_config,
            worker_selector,
            region_clients: Arc::new(RwLock::new(HashMap::new())),
            current_routing: Arc::new(RwLock::new(None)),
            last_switch: Arc::new(RwLock::new(None)),
            routing_tx,
            leader_hint_tx: Arc::new(RwLock::new(None)),
        };

        // Measure latencies to all workers
        client.worker_selector.measure_all().await;

        // Connect to the best region initially
        let best_worker = client.worker_selector.select_best().await?;
        client.ensure_region_connected(&best_worker.region).await?;

        // Start leader hint subscription if auto-follow is enabled
        if client.multi_config.auto_follow_leader {
            client.start_leader_hint_listener().await?;
        }

        info!(
            initial_region = %best_worker.region,
            worker_count = client.worker_selector.worker_count(),
            "Multi-region client initialized"
        );

        Ok(client)
    }

    /// Submit a transaction using leader-aware routing
    ///
    /// The transaction is routed to the region with best proximity to the
    /// current leader validator. If that region fails, fallback regions
    /// are tried based on the configured fallback strategy.
    pub async fn submit_transaction(&self, transaction: &[u8]) -> Result<TransactionResult> {
        self.submit_transaction_with_options(transaction, &SubmitOptions::default())
            .await
    }

    /// Submit a transaction with custom options
    pub async fn submit_transaction_with_options(
        &self,
        transaction: &[u8],
        options: &SubmitOptions,
    ) -> Result<TransactionResult> {
        // If broadcast mode is enabled (either in options or for high priority), fan out
        if options.broadcast_mode
            || (self.multi_config.broadcast_high_priority && self.is_high_priority(options))
        {
            return self.broadcast_transaction(transaction, options).await;
        }

        // Get current routing recommendation
        let routing = self.current_routing.read().await;
        let best_region = routing
            .as_ref()
            .map(|r| r.best_region.clone())
            .unwrap_or_else(|| self.get_default_region());

        let fallback_strategy = routing
            .as_ref()
            .map(|r| r.fallback_strategy)
            .unwrap_or(FallbackStrategy::Sequential);

        let fallback_regions: Vec<String> = routing
            .as_ref()
            .map(|r| r.fallback_regions.clone())
            .unwrap_or_default();
        drop(routing);

        // Try primary region
        match self.submit_to_region(&best_region, transaction, options).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                warn!(region = %best_region, error = %e, "Primary region failed");
            }
        }

        // Handle fallback based on strategy
        match fallback_strategy {
            FallbackStrategy::Sequential => {
                for region in fallback_regions {
                    match self.submit_to_region(&region, transaction, options).await {
                        Ok(result) => return Ok(result),
                        Err(e) => {
                            warn!(region = %region, error = %e, "Fallback region failed");
                        }
                    }
                }
            }
            FallbackStrategy::Broadcast => {
                return self.broadcast_transaction(transaction, options).await;
            }
            FallbackStrategy::Retry => {
                // Retry same region with backoff
                for attempt in 1..=options.max_retries {
                    let delay = Duration::from_millis(100 * (1 << attempt));
                    tokio::time::sleep(delay).await;

                    match self.submit_to_region(&best_region, transaction, options).await {
                        Ok(result) => return Ok(result),
                        Err(e) => {
                            warn!(
                                region = %best_region,
                                attempt = attempt,
                                error = %e,
                                "Retry failed"
                            );
                        }
                    }
                }
            }
            FallbackStrategy::None => {
                // No fallback
            }
        }

        Err(SdkError::transaction("All regions failed"))
    }

    /// Broadcast transaction to multiple regions simultaneously
    async fn broadcast_transaction(
        &self,
        transaction: &[u8],
        options: &SubmitOptions,
    ) -> Result<TransactionResult> {
        let regions = self.get_broadcast_regions().await;
        if regions.is_empty() {
            return Err(SdkError::config("No regions available for broadcast"));
        }

        let mut handles = Vec::with_capacity(regions.len());
        for region in regions {
            let tx = transaction.to_vec();
            let opts = options.clone();
            let client = self.clone_for_region(&region).await;

            handles.push(tokio::spawn(async move {
                match client {
                    Some(c) => c.submit_transaction_with_options(&tx, &opts).await,
                    None => Err(SdkError::connection(format!(
                        "Not connected to region: {}",
                        region
                    ))),
                }
            }));
        }

        // Return first successful result
        for handle in handles {
            if let Ok(Ok(result)) = handle.await {
                return Ok(result);
            }
        }

        Err(SdkError::transaction("All broadcast regions failed"))
    }

    /// Submit transaction to a specific region
    async fn submit_to_region(
        &self,
        region: &str,
        transaction: &[u8],
        options: &SubmitOptions,
    ) -> Result<TransactionResult> {
        self.ensure_region_connected(region).await?;

        let clients = self.region_clients.read().await;
        let client = clients
            .get(region)
            .ok_or_else(|| SdkError::connection(format!("Not connected to region: {}", region)))?;

        client.submit_transaction_with_options(transaction, options).await
    }

    /// Ensure a region has an active connection
    async fn ensure_region_connected(&self, region: &str) -> Result<()> {
        {
            let clients = self.region_clients.read().await;
            if clients.contains_key(region) {
                return Ok(());
            }
        }

        // Find best worker in region
        let worker = self
            .worker_selector
            .select_best_in_region(region)
            .await?;

        // Create config for this worker
        let mut region_config = self.config.clone();
        region_config.endpoint = worker.http.clone();
        region_config.region = Some(region.to_string());

        // Connect
        debug!(region = %region, worker = %worker.id, "Connecting to region");
        let client = SlipstreamClient::connect(region_config).await?;

        let mut clients = self.region_clients.write().await;
        clients.insert(region.to_string(), Arc::new(client));

        info!(region = %region, "Connected to region");
        Ok(())
    }

    /// Get a clone of the client for a region (for async operations)
    async fn clone_for_region(&self, region: &str) -> Option<Arc<SlipstreamClient>> {
        let clients = self.region_clients.read().await;
        clients.get(region).cloned()
    }

    /// Subscribe to routing recommendation updates
    pub fn subscribe_routing_updates(&self) -> broadcast::Receiver<RoutingRecommendation> {
        self.routing_tx.subscribe()
    }

    /// Get current routing recommendation
    pub async fn get_current_routing(&self) -> Option<RoutingRecommendation> {
        self.current_routing.read().await.clone()
    }

    /// Update routing based on a leader hint
    pub async fn update_routing_from_hint(&self, hint: &LeaderHint) {
        // Check confidence threshold
        if hint.confidence < self.multi_config.min_switch_confidence {
            debug!(
                confidence = hint.confidence,
                threshold = self.multi_config.min_switch_confidence,
                "Leader hint below confidence threshold"
            );
            return;
        }

        // Check cooldown
        if let Some(last) = *self.last_switch.read().await {
            let cooldown = Duration::from_millis(self.multi_config.switch_cooldown_ms);
            if last.elapsed() < cooldown {
                debug!("Region switch on cooldown");
                return;
            }
        }

        let recommendation = RoutingRecommendation {
            best_region: hint.preferred_region.clone(),
            leader_pubkey: hint.leader_pubkey.clone(),
            slot: hint.slot,
            confidence: hint.confidence,
            expected_rtt_ms: Some(hint.metadata.tpu_rtt_ms),
            fallback_regions: hint.backup_regions.clone(),
            fallback_strategy: FallbackStrategy::Sequential,
            valid_for_ms: ((hint.expires_at_slot - hint.slot) * 400) as u64, // ~400ms per slot
        };

        // Update current routing
        {
            let mut current = self.current_routing.write().await;
            let should_switch = current
                .as_ref()
                .map(|r| r.best_region != recommendation.best_region)
                .unwrap_or(true);

            if should_switch {
                info!(
                    old_region = ?current.as_ref().map(|r| &r.best_region),
                    new_region = %recommendation.best_region,
                    confidence = %recommendation.confidence,
                    "Switching routing to new region"
                );
                *self.last_switch.write().await = Some(Instant::now());
            }

            *current = Some(recommendation.clone());
        }

        // Broadcast update
        let _ = self.routing_tx.send(recommendation);
    }

    /// Start listening to leader hints from primary connection
    async fn start_leader_hint_listener(&self) -> Result<()> {
        // Get primary client
        let clients = self.region_clients.read().await;
        let primary_client = clients.values().next().cloned();
        drop(clients);

        let Some(client) = primary_client else {
            return Ok(());
        };

        let mut hints_rx = client.subscribe_leader_hints().await?;
        let routing = Arc::clone(&self.current_routing);
        let last_switch = Arc::clone(&self.last_switch);
        let routing_tx = self.routing_tx.clone();
        let min_confidence = self.multi_config.min_switch_confidence;
        let cooldown_ms = self.multi_config.switch_cooldown_ms;

        tokio::spawn(async move {
            while let Some(hint) = hints_rx.recv().await {
                if hint.confidence < min_confidence {
                    continue;
                }

                // Check cooldown
                if let Some(last) = *last_switch.read().await {
                    if last.elapsed() < Duration::from_millis(cooldown_ms) {
                        continue;
                    }
                }

                let recommendation = RoutingRecommendation {
                    best_region: hint.preferred_region.clone(),
                    leader_pubkey: hint.leader_pubkey.clone(),
                    slot: hint.slot,
                    confidence: hint.confidence,
                    expected_rtt_ms: Some(hint.metadata.tpu_rtt_ms),
                    fallback_regions: hint.backup_regions.clone(),
                    fallback_strategy: FallbackStrategy::Sequential,
                    valid_for_ms: ((hint.expires_at_slot - hint.slot) * 400) as u64,
                };

                {
                    let mut current = routing.write().await;
                    let should_switch = current
                        .as_ref()
                        .map(|r| r.best_region != recommendation.best_region)
                        .unwrap_or(true);

                    if should_switch {
                        *last_switch.write().await = Some(Instant::now());
                    }
                    *current = Some(recommendation.clone());
                }

                let _ = routing_tx.send(recommendation);
            }
        });

        Ok(())
    }

    /// Get regions for broadcast mode
    async fn get_broadcast_regions(&self) -> Vec<String> {
        let mut regions: Vec<String> = self
            .worker_selector
            .workers()
            .iter()
            .map(|w| w.region.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        // Sort by latency if we have measurements
        let latencies = self.worker_selector.get_all_latencies().await;
        regions.sort_by(|a, b| {
            let a_lat = self
                .worker_selector
                .workers()
                .iter()
                .filter(|w| &w.region == a)
                .filter_map(|w| latencies.get(&w.id))
                .filter(|m| m.reachable)
                .map(|m| m.rtt_ms)
                .min()
                .unwrap_or(u64::MAX);

            let b_lat = self
                .worker_selector
                .workers()
                .iter()
                .filter(|w| &w.region == b)
                .filter_map(|w| latencies.get(&w.id))
                .filter(|m| m.reachable)
                .map(|m| m.rtt_ms)
                .min()
                .unwrap_or(u64::MAX);

            a_lat.cmp(&b_lat)
        });

        // Limit to max broadcast regions
        regions.truncate(self.multi_config.max_broadcast_regions);
        regions
    }

    /// Get default region (first available)
    fn get_default_region(&self) -> String {
        self.worker_selector
            .workers()
            .first()
            .map(|w| w.region.clone())
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// Check if options indicate high priority
    fn is_high_priority(&self, _options: &SubmitOptions) -> bool {
        // Could be extended to check tip amount, dedup_id patterns, etc.
        false
    }

    /// Get the worker selector for direct access to latency measurements
    pub fn worker_selector(&self) -> &WorkerSelector {
        &self.worker_selector
    }

    /// Get list of connected regions
    pub async fn connected_regions(&self) -> Vec<String> {
        self.region_clients.read().await.keys().cloned().collect()
    }

    /// Disconnect from all regions
    pub async fn disconnect_all(&self) -> Result<()> {
        let mut clients = self.region_clients.write().await;
        for (region, client) in clients.drain() {
            debug!(region = %region, "Disconnecting from region");
            if let Err(e) = client.disconnect().await {
                warn!(region = %region, error = %e, "Error disconnecting");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_workers() -> Vec<WorkerEndpoint> {
        vec![
            WorkerEndpoint::with_endpoints(
                "w1",
                "us-west",
                Some("127.0.0.1:4433".to_string()),
                None,
                None,
                Some("http://127.0.0.1:9000".to_string()),
            ),
            WorkerEndpoint::with_endpoints(
                "w2",
                "eu-central",
                Some("127.0.0.2:4433".to_string()),
                None,
                None,
                Some("http://127.0.0.2:9000".to_string()),
            ),
        ]
    }

    #[test]
    fn test_multi_region_config_defaults() {
        let config = MultiRegionConfig::default();
        assert!(config.auto_follow_leader);
        assert_eq!(config.min_switch_confidence, 60);
    }

    #[test]
    fn test_fallback_strategy() {
        assert_eq!(FallbackStrategy::default(), FallbackStrategy::Sequential);
    }
}
