//! Worker selection with ping-based latency measurement
//!
//! Selects the best worker endpoint based on measured latency.

use crate::error::{Result, SdkError};
use crate::types::{Protocol, WorkerEndpoint};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Default cache TTL for latency measurements
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(30);

/// Default ping timeout
const DEFAULT_PING_TIMEOUT: Duration = Duration::from_secs(2);



/// Latency measurement for a worker
#[derive(Debug, Clone)]
pub struct LatencyMeasurement {
    /// Round-trip time in milliseconds
    pub rtt_ms: u64,
    /// When this measurement was taken
    pub measured_at: Instant,
    /// Whether the worker responded successfully
    pub reachable: bool,
}

impl LatencyMeasurement {
    /// Check if this measurement is still valid
    pub fn is_fresh(&self, ttl: Duration) -> bool {
        self.measured_at.elapsed() < ttl
    }
}

/// Worker selector with ping-based latency measurement
pub struct WorkerSelector {
    /// Available worker endpoints
    workers: Vec<WorkerEndpoint>,
    /// Cached latency measurements keyed by worker ID
    latencies: Arc<RwLock<HashMap<String, LatencyMeasurement>>>,
    /// Cache TTL
    cache_ttl: Duration,
    /// Ping timeout
    ping_timeout: Duration,
}

impl WorkerSelector {
    /// Create a new worker selector
    pub fn new(workers: Vec<WorkerEndpoint>) -> Self {
        Self {
            workers,
            latencies: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl: DEFAULT_CACHE_TTL,
            ping_timeout: DEFAULT_PING_TIMEOUT,
        }
    }

    /// Create with custom cache TTL
    pub fn with_cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache_ttl = ttl;
        self
    }

    /// Create with custom ping timeout
    pub fn with_ping_timeout(mut self, timeout: Duration) -> Self {
        self.ping_timeout = timeout;
        self
    }

    /// Get the number of workers
    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    /// Get all workers
    pub fn workers(&self) -> &[WorkerEndpoint] {
        &self.workers
    }

    /// Select the best worker based on latency
    pub async fn select_best(&self) -> Result<&WorkerEndpoint> {
        if self.workers.is_empty() {
            return Err(SdkError::config("No workers configured"));
        }

        // Check if we have fresh cached measurements
        let have_fresh_cache = {
            let latencies = self.latencies.read().await;
            self.workers.iter().any(|w| {
                latencies
                    .get(&w.id)
                    .map(|m| m.is_fresh(self.cache_ttl) && m.reachable)
                    .unwrap_or(false)
            })
        };

        // If no fresh cache, measure all workers
        if !have_fresh_cache {
            self.measure_all().await;
        }

        // Select worker with lowest latency
        let latencies = self.latencies.read().await;
        let mut best: Option<(&WorkerEndpoint, u64)> = None;

        for worker in &self.workers {
            if let Some(measurement) = latencies.get(&worker.id) {
                if measurement.reachable && measurement.is_fresh(self.cache_ttl) {
                    match best {
                        None => best = Some((worker, measurement.rtt_ms)),
                        Some((_, best_rtt)) if measurement.rtt_ms < best_rtt => {
                            best = Some((worker, measurement.rtt_ms))
                        }
                        _ => {}
                    }
                }
            }
        }

        match best {
            Some((worker, rtt)) => {
                debug!(
                    worker_id = %worker.id,
                    region = %worker.region,
                    rtt_ms = rtt,
                    "Selected best worker"
                );
                Ok(worker)
            }
            None => {
                // Fall back to first worker if all unreachable
                warn!("All workers unreachable, falling back to first worker");
                Ok(&self.workers[0])
            }
        }
    }

    /// Select the best worker in a specific region
    pub async fn select_best_in_region(&self, region: &str) -> Result<&WorkerEndpoint> {
        let regional_workers: Vec<_> = self
            .workers
            .iter()
            .filter(|w| w.region == region)
            .collect();

        if regional_workers.is_empty() {
            return Err(SdkError::config(format!("No workers in region: {}", region)));
        }

        // Check fresh cache for regional workers
        let have_fresh_cache = {
            let latencies = self.latencies.read().await;
            regional_workers.iter().any(|w| {
                latencies
                    .get(&w.id)
                    .map(|m| m.is_fresh(self.cache_ttl) && m.reachable)
                    .unwrap_or(false)
            })
        };

        if !have_fresh_cache {
            self.measure_workers(&regional_workers).await;
        }

        // Select best in region
        let latencies = self.latencies.read().await;
        let mut best: Option<(&WorkerEndpoint, u64)> = None;

        for worker in regional_workers {
            if let Some(measurement) = latencies.get(&worker.id) {
                if measurement.reachable && measurement.is_fresh(self.cache_ttl) {
                    match best {
                        None => best = Some((worker, measurement.rtt_ms)),
                        Some((_, best_rtt)) if measurement.rtt_ms < best_rtt => {
                            best = Some((worker, measurement.rtt_ms))
                        }
                        _ => {}
                    }
                }
            }
        }

        match best {
            Some((worker, _)) => Ok(worker),
            None => {
                // Fall back to first regional worker
                Ok(self.workers.iter().find(|w| w.region == region).unwrap())
            }
        }
    }

    /// Get cached latency for a worker
    pub async fn get_latency(&self, worker_id: &str) -> Option<LatencyMeasurement> {
        let latencies = self.latencies.read().await;
        latencies.get(worker_id).cloned()
    }

    /// Measure latency to all workers in parallel
    pub async fn measure_all(&self) {
        let workers: Vec<_> = self.workers.iter().collect();
        self.measure_workers(&workers).await;
    }

    /// Measure latency to specific workers in parallel
    async fn measure_workers(&self, workers: &[&WorkerEndpoint]) {
        let mut handles = Vec::with_capacity(workers.len());

        for worker in workers {
            let worker_id = worker.id.clone();
            let endpoint = worker.http.clone();
            let timeout = self.ping_timeout;
            let latencies = Arc::clone(&self.latencies);

            handles.push(tokio::spawn(async move {
                let measurement = Self::ping_worker(endpoint.as_deref(), timeout).await;
                let mut lat = latencies.write().await;
                lat.insert(worker_id, measurement);
            }));
        }

        // Wait for all pings to complete
        for handle in handles {
            let _ = handle.await;
        }
    }

    /// Ping a single worker and measure latency
    async fn ping_worker(endpoint: Option<&str>, timeout: Duration) -> LatencyMeasurement {
        let endpoint = match endpoint {
            Some(e) => e,
            None => {
                return LatencyMeasurement {
                    rtt_ms: u64::MAX,
                    measured_at: Instant::now(),
                    reachable: false,
                }
            }
        };

        let start = Instant::now();
        let health_url = format!("{}/health/", endpoint.trim_end_matches('/'));

        let result = tokio::time::timeout(timeout, async {
            reqwest::Client::new()
                .head(&health_url)
                .send()
                .await
        })
        .await;

        let measured_at = Instant::now();
        let rtt_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(response)) if response.status().is_success() => {
                debug!(endpoint = %endpoint, rtt_ms = rtt_ms, "Worker ping successful");
                LatencyMeasurement {
                    rtt_ms,
                    measured_at,
                    reachable: true,
                }
            }
            Ok(Ok(response)) => {
                debug!(
                    endpoint = %endpoint,
                    status = %response.status(),
                    "Worker ping returned non-success status"
                );
                LatencyMeasurement {
                    rtt_ms,
                    measured_at,
                    reachable: false,
                }
            }
            Ok(Err(e)) => {
                debug!(endpoint = %endpoint, error = %e, "Worker ping failed");
                LatencyMeasurement {
                    rtt_ms: u64::MAX,
                    measured_at,
                    reachable: false,
                }
            }
            Err(_) => {
                debug!(endpoint = %endpoint, "Worker ping timed out");
                LatencyMeasurement {
                    rtt_ms: u64::MAX,
                    measured_at,
                    reachable: false,
                }
            }
        }
    }

    /// Invalidate cache for a specific worker
    pub async fn invalidate(&self, worker_id: &str) {
        let mut latencies = self.latencies.write().await;
        latencies.remove(worker_id);
    }

    /// Invalidate all cached measurements
    pub async fn invalidate_all(&self) {
        let mut latencies = self.latencies.write().await;
        latencies.clear();
    }

    /// Get all cached measurements
    pub async fn get_all_latencies(&self) -> HashMap<String, LatencyMeasurement> {
        self.latencies.read().await.clone()
    }
}

impl Default for WorkerSelector {
    fn default() -> Self {
        Self::new(vec![])
    }
}

/// Builder for creating a WorkerSelector with workers from config
pub struct WorkerSelectorBuilder {
    workers: Vec<WorkerEndpoint>,
    cache_ttl: Duration,
    ping_timeout: Duration,
}

impl WorkerSelectorBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            workers: Vec::new(),
            cache_ttl: DEFAULT_CACHE_TTL,
            ping_timeout: DEFAULT_PING_TIMEOUT,
        }
    }

    /// Add a worker endpoint
    pub fn add_worker(mut self, worker: WorkerEndpoint) -> Self {
        self.workers.push(worker);
        self
    }

    /// Add a worker with default ports
    pub fn add_worker_host(mut self, id: &str, region: &str, host: &str) -> Self {
        self.workers.push(WorkerEndpoint::new(id, region, host));
        self
    }

    /// Set cache TTL
    pub fn cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache_ttl = ttl;
        self
    }

    /// Set ping timeout
    pub fn ping_timeout(mut self, timeout: Duration) -> Self {
        self.ping_timeout = timeout;
        self
    }

    /// Build the WorkerSelector
    pub fn build(self) -> WorkerSelector {
        WorkerSelector::new(self.workers)
            .with_cache_ttl(self.cache_ttl)
            .with_ping_timeout(self.ping_timeout)
    }
}

impl Default for WorkerSelectorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_workers() -> Vec<WorkerEndpoint> {
        vec![
            WorkerEndpoint {
                id: "worker-1".to_string(),
                region: "us-east".to_string(),
                quic: Some("127.0.0.1:4433".to_string()),
                grpc: Some("http://127.0.0.1:10000".to_string()),
                websocket: Some("ws://127.0.0.1:9000/ws".to_string()),
                http: Some("http://127.0.0.1:9000".to_string()),
            },
            WorkerEndpoint {
                id: "worker-2".to_string(),
                region: "us-west".to_string(),
                quic: Some("127.0.0.2:4433".to_string()),
                grpc: Some("http://127.0.0.2:10000".to_string()),
                websocket: Some("ws://127.0.0.2:9000/ws".to_string()),
                http: Some("http://127.0.0.2:9000".to_string()),
            },
            WorkerEndpoint {
                id: "worker-3".to_string(),
                region: "us-east".to_string(),
                quic: Some("127.0.0.3:4433".to_string()),
                grpc: Some("http://127.0.0.3:10000".to_string()),
                websocket: Some("ws://127.0.0.3:9000/ws".to_string()),
                http: Some("http://127.0.0.3:9000".to_string()),
            },
        ]
    }

    #[test]
    fn test_worker_endpoint_new() {
        let worker = WorkerEndpoint::new("w1", "us-east", "worker1.slipstream.allenhark.com");
        assert_eq!(worker.id, "w1");
        assert_eq!(worker.region, "us-east");
        assert_eq!(worker.quic, Some("worker1.slipstream.allenhark.com:4433".to_string()));
        assert_eq!(worker.grpc, Some("http://worker1.slipstream.allenhark.com:10000".to_string()));
        assert_eq!(worker.websocket, Some("wss://worker1.slipstream.allenhark.com/ws".to_string()));
        assert_eq!(worker.http, Some("https://worker1.slipstream.allenhark.com".to_string()));
    }

    #[test]
    fn test_worker_endpoint_get_endpoint() {
        let worker = WorkerEndpoint::new("w1", "us-east", "worker1.slipstream.allenhark.com");
        assert_eq!(worker.get_endpoint(Protocol::Quic), Some("worker1.slipstream.allenhark.com:4433"));
        assert_eq!(worker.get_endpoint(Protocol::Grpc), Some("http://worker1.slipstream.allenhark.com:10000"));
        assert_eq!(worker.get_endpoint(Protocol::WebSocket), Some("wss://worker1.slipstream.allenhark.com/ws"));
        assert_eq!(worker.get_endpoint(Protocol::Http), Some("https://worker1.slipstream.allenhark.com"));
    }

    #[test]
    fn test_worker_selector_new() {
        let workers = create_test_workers();
        let selector = WorkerSelector::new(workers.clone());
        assert_eq!(selector.worker_count(), 3);
    }

    #[test]
    fn test_latency_measurement_is_fresh() {
        let measurement = LatencyMeasurement {
            rtt_ms: 50,
            measured_at: Instant::now(),
            reachable: true,
        };
        assert!(measurement.is_fresh(Duration::from_secs(30)));

        // Old measurement
        let old_measurement = LatencyMeasurement {
            rtt_ms: 50,
            measured_at: Instant::now() - Duration::from_secs(60),
            reachable: true,
        };
        assert!(!old_measurement.is_fresh(Duration::from_secs(30)));
    }

    #[test]
    fn test_worker_selector_builder() {
        let selector = WorkerSelectorBuilder::new()
            .add_worker_host("w1", "us-east", "worker1.example.com")
            .add_worker_host("w2", "us-west", "worker2.example.com")
            .cache_ttl(Duration::from_secs(60))
            .ping_timeout(Duration::from_secs(5))
            .build();

        assert_eq!(selector.worker_count(), 2);
        assert_eq!(selector.cache_ttl, Duration::from_secs(60));
        assert_eq!(selector.ping_timeout, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_select_best_empty() {
        let selector = WorkerSelector::new(vec![]);
        let result = selector.select_best().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_invalidate_cache() {
        let workers = create_test_workers();
        let selector = WorkerSelector::new(workers);

        // Add a measurement
        {
            let mut latencies = selector.latencies.write().await;
            latencies.insert(
                "worker-1".to_string(),
                LatencyMeasurement {
                    rtt_ms: 50,
                    measured_at: Instant::now(),
                    reachable: true,
                },
            );
        }

        // Verify it exists
        assert!(selector.get_latency("worker-1").await.is_some());

        // Invalidate
        selector.invalidate("worker-1").await;

        // Verify it's gone
        assert!(selector.get_latency("worker-1").await.is_none());
    }

    #[tokio::test]
    async fn test_invalidate_all() {
        let workers = create_test_workers();
        let selector = WorkerSelector::new(workers);

        // Add measurements
        {
            let mut latencies = selector.latencies.write().await;
            latencies.insert(
                "worker-1".to_string(),
                LatencyMeasurement {
                    rtt_ms: 50,
                    measured_at: Instant::now(),
                    reachable: true,
                },
            );
            latencies.insert(
                "worker-2".to_string(),
                LatencyMeasurement {
                    rtt_ms: 60,
                    measured_at: Instant::now(),
                    reachable: true,
                },
            );
        }

        // Invalidate all
        selector.invalidate_all().await;

        // Verify all are gone
        let latencies = selector.get_all_latencies().await;
        assert!(latencies.is_empty());
    }

    #[tokio::test]
    async fn test_select_best_with_cached_measurements() {
        let workers = create_test_workers();
        let selector = WorkerSelector::new(workers);

        // Add measurements with different latencies
        {
            let mut latencies = selector.latencies.write().await;
            latencies.insert(
                "worker-1".to_string(),
                LatencyMeasurement {
                    rtt_ms: 100,
                    measured_at: Instant::now(),
                    reachable: true,
                },
            );
            latencies.insert(
                "worker-2".to_string(),
                LatencyMeasurement {
                    rtt_ms: 50, // Best latency
                    measured_at: Instant::now(),
                    reachable: true,
                },
            );
            latencies.insert(
                "worker-3".to_string(),
                LatencyMeasurement {
                    rtt_ms: 75,
                    measured_at: Instant::now(),
                    reachable: true,
                },
            );
        }

        let best = selector.select_best().await.unwrap();
        assert_eq!(best.id, "worker-2");
    }

    #[tokio::test]
    async fn test_select_best_in_region() {
        let workers = create_test_workers();
        let selector = WorkerSelector::new(workers);

        // Add measurements
        {
            let mut latencies = selector.latencies.write().await;
            latencies.insert(
                "worker-1".to_string(),
                LatencyMeasurement {
                    rtt_ms: 100,
                    measured_at: Instant::now(),
                    reachable: true,
                },
            );
            latencies.insert(
                "worker-2".to_string(),
                LatencyMeasurement {
                    rtt_ms: 50, // Best overall but in us-west
                    measured_at: Instant::now(),
                    reachable: true,
                },
            );
            latencies.insert(
                "worker-3".to_string(),
                LatencyMeasurement {
                    rtt_ms: 75, // Best in us-east
                    measured_at: Instant::now(),
                    reachable: true,
                },
            );
        }

        let best_us_east = selector.select_best_in_region("us-east").await.unwrap();
        assert_eq!(best_us_east.id, "worker-3");

        let best_us_west = selector.select_best_in_region("us-west").await.unwrap();
        assert_eq!(best_us_west.id, "worker-2");
    }
}
