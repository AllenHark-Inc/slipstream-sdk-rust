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
    /// Best worker-to-leader RTT in this region (ms), if known
    pub leader_rtt_ms: Option<f64>,
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
    #[serde(default = "default_grpc_port")]
    pub grpc: u16,
    /// WebSocket port for streaming subscriptions
    #[serde(default)]
    pub ws: Option<u16>,
    /// HTTP management port for billing proxy (e.g., 9091)
    #[serde(default)]
    pub http: Option<u16>,
    /// Legacy QUIC port advertised during a port migration; absent on old control planes.
    #[serde(default)]
    pub legacy_quic: Option<u16>,
    /// Legacy gRPC port advertised during a port migration; absent on old control planes.
    #[serde(default)]
    pub legacy_grpc: Option<u16>,
    /// Legacy WebSocket port advertised during a port migration; absent on old control planes.
    #[serde(default)]
    pub legacy_ws: Option<u16>,
}

fn default_grpc_port() -> u16 {
    10000
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
            .map(|w| Self::worker_to_endpoint(w))
            .collect()
    }

    /// Convert a single discovery worker to a WorkerEndpoint
    pub fn worker_to_endpoint(worker: &DiscoveryWorker) -> WorkerEndpoint {
        let http_endpoint = worker.ports.http
            .map(|port| format!("http://{}:{}", worker.ip, port));

        let ws_endpoint = worker.ports.ws
            .map(|port| format!("ws://{}:{}/ws", worker.ip, port));

        WorkerEndpoint::with_endpoints(
            &worker.id,
            &worker.region,
            Some(format!("{}:{}", worker.ip, worker.ports.quic)),
            Some(format!("http://{}:{}", worker.ip, worker.ports.grpc)),
            ws_endpoint,
            http_endpoint,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_ports_deserializes_with_legacy_fields() {
        let json = r#"{
            "quic": 4435, "grpc": 10001, "ws": 9002, "http": 9092,
            "legacy_quic": 4433, "legacy_grpc": 10000, "legacy_ws": 9000
        }"#;
        let p: WorkerPorts = serde_json::from_str(json).unwrap();
        assert_eq!(p.quic, 4435);
        assert_eq!(p.legacy_quic, Some(4433));
        assert_eq!(p.legacy_grpc, Some(10000));
        assert_eq!(p.legacy_ws, Some(9000));
    }

    #[test]
    fn worker_ports_deserializes_without_legacy_fields_old_cp() {
        // An OLD control plane omits legacy_* entirely — must still parse.
        let json = r#"{ "quic": 4433, "grpc": 10000, "ws": 9000, "http": 9091 }"#;
        let p: WorkerPorts = serde_json::from_str(json).unwrap();
        assert_eq!(p.quic, 4433);
        assert_eq!(p.legacy_quic, None);
        assert_eq!(p.legacy_grpc, None);
        assert_eq!(p.legacy_ws, None);
    }

    #[test]
    fn worker_ports_deserializes_bare_minimum() {
        // grpc/ws/http all defaulted; only quic present.
        let p: WorkerPorts = serde_json::from_str(r#"{ "quic": 4435 }"#).unwrap();
        assert_eq!(p.quic, 4435);
        assert_eq!(p.grpc, 10000); // default_grpc_port
        assert_eq!(p.ws, None);
        assert_eq!(p.legacy_quic, None);
    }
}
