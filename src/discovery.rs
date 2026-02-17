//! Service discovery for automatic worker endpoint resolution
//!
//! SDKs call the discovery endpoint to find available workers
//! and their IP addresses, then connect directly to worker IPs.

use crate::error::{Result, SdkError};
use crate::types::WorkerEndpoint;
use serde::Deserialize;
use tracing::{debug, info, warn};

/// Default discovery URL
pub const DEFAULT_DISCOVERY_URL: &str = "https://discovery.allenhark.network";

/// Discovery API response
#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveryResponse {
    pub regions: Vec<DiscoveryRegion>,
    pub workers: Vec<DiscoveryWorker>,
    pub recommended_region: Option<String>,
}

/// Region information from discovery
#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveryRegion {
    pub id: String,
    pub name: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

/// Worker information from discovery
#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveryWorker {
    pub id: String,
    pub region: String,
    pub ip: String,
    pub ports: WorkerPorts,
    pub healthy: bool,
    pub version: Option<String>,
}

/// Worker port configuration
#[derive(Debug, Clone, Deserialize)]
pub struct WorkerPorts {
    pub quic: u16,
    pub ws: u16,
    pub http: u16,
}

/// Client for the discovery service
pub struct DiscoveryClient {
    http: reqwest::Client,
    url: String,
}

impl DiscoveryClient {
    /// Create a new discovery client
    pub fn new(discovery_url: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            http,
            url: discovery_url.to_string(),
        }
    }

    /// Fetch available workers and regions from the discovery endpoint
    pub async fn discover(&self) -> Result<DiscoveryResponse> {
        let url = format!("{}/v1/discovery", self.url);
        debug!(url = %url, "Fetching worker discovery");

        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| SdkError::connection(format!("Discovery request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(SdkError::connection(format!(
                "Discovery failed (HTTP {}): {}",
                status, body
            )));
        }

        let discovery: DiscoveryResponse = response
            .json()
            .await
            .map_err(|e| SdkError::connection(format!("Invalid discovery response: {}", e)))?;

        info!(
            regions = discovery.regions.len(),
            workers = discovery.workers.len(),
            recommended = ?discovery.recommended_region,
            "Discovery complete"
        );

        Ok(discovery)
    }

    /// Get healthy workers for a specific region
    pub fn workers_for_region<'a>(
        &self,
        response: &'a DiscoveryResponse,
        region: &str,
    ) -> Vec<&'a DiscoveryWorker> {
        response
            .workers
            .iter()
            .filter(|w| w.region == region && w.healthy)
            .collect()
    }

    /// Get the best region — either use `preferred` if provided,
    /// or fall back to `recommended_region` from the server.
    pub fn best_region(
        &self,
        response: &DiscoveryResponse,
        preferred: Option<&str>,
    ) -> Option<String> {
        if let Some(pref) = preferred {
            // Verify the preferred region has healthy workers
            if response
                .workers
                .iter()
                .any(|w| w.region == pref && w.healthy)
            {
                return Some(pref.to_string());
            }
            warn!(
                preferred = pref,
                "Preferred region has no healthy workers, falling back"
            );
        }

        response.recommended_region.clone()
    }

    /// Convert discovery workers to SDK WorkerEndpoints
    pub fn to_worker_endpoints(workers: &[DiscoveryWorker]) -> Vec<WorkerEndpoint> {
        workers
            .iter()
            .filter(|w| w.healthy)
            .map(|w| {
                WorkerEndpoint::with_ports(
                    &w.id,
                    &w.region,
                    &w.ip,
                    w.ports.quic,
                    10000, // gRPC port (standard)
                    w.ports.ws,
                    w.ports.http,
                )
            })
            .collect()
    }

    /// Convert a single discovery worker to a WorkerEndpoint
    pub fn worker_to_endpoint(worker: &DiscoveryWorker) -> WorkerEndpoint {
        WorkerEndpoint::with_ports(
            &worker.id,
            &worker.region,
            &worker.ip,
            worker.ports.quic,
            10000,
            worker.ports.ws,
            worker.ports.http,
        )
    }
}
