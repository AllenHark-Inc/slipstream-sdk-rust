//! HTTP transport implementation using reqwest
//!
//! This provides a polling-based fallback when streaming protocols are unavailable.

use super::Transport;
use crate::config::{Config, Protocol};
use crate::error::{Result, SdkError};
use crate::types::{
    ConnectionInfo, LatestBlockhash, LatestSlot, LeaderHint, PingResult, PriorityFee, RateLimitInfo,
    SubmitOptions, TipInstruction, TransactionResult, TransactionStatus,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// HTTP polling interval for streams
const POLL_INTERVAL_MS: u64 = 1000;

/// HTTP transport implementation
pub struct HttpTransport {
    client: Client,
    config: Option<Config>,
    base_url: String,
    connected: AtomicBool,
    session_id: Option<String>,
    request_counter: AtomicU64,
}

impl HttpTransport {
    /// Create a new HTTP transport
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
            config: None,
            base_url: String::new(),
            connected: AtomicBool::new(false),
            session_id: None,
            request_counter: AtomicU64::new(0),
        }
    }

    /// Generate a unique request ID
    fn next_request_id(&self) -> String {
        let id = self.request_counter.fetch_add(1, Ordering::Relaxed);
        format!("req-{}", id)
    }

    /// Build authorization header
    fn auth_header(&self) -> Option<String> {
        self.config.as_ref().map(|c| format!("Bearer {}", c.api_key))
    }
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Transport for HttpTransport {
    async fn connect(&mut self, config: &Config) -> Result<ConnectionInfo> {
        self.base_url = config.get_endpoint(Protocol::Http);
        self.config = Some(config.clone());

        // Verify connectivity by calling a health endpoint
        let health_url = format!("{}/health/", self.base_url);
        debug!(url = %health_url, "Checking HTTP connectivity");

        let response = self
            .client
            .get(&health_url)
            .send()
            .await
            .map_err(|e| SdkError::connection(format!("HTTP health check failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(SdkError::connection(format!(
                "HTTP health check returned status {}",
                response.status()
            )));
        }

        self.connected.store(true, Ordering::SeqCst);
        self.session_id = Some(uuid::Uuid::new_v4().to_string());

        debug!("HTTP transport connected successfully");

        Ok(ConnectionInfo {
            session_id: self.session_id.clone().unwrap_or_default(),
            protocol: "http".to_string(),
            region: config.region.clone(),
            server_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            features: vec!["polling".to_string()],
            rate_limit: RateLimitInfo { rps: 100, burst: 200 },
        })
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.session_id = None;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn protocol(&self) -> Protocol {
        Protocol::Http
    }

    async fn submit_transaction(
        &self,
        transaction: &[u8],
        options: &SubmitOptions,
    ) -> Result<TransactionResult> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let request_id = self.next_request_id();
        let submit_url = format!("{}/v1/transactions/submit", self.base_url);

        let request = HttpSubmitRequest {
            request_id: request_id.clone(),
            transaction: BASE64.encode(transaction),
            dedup_id: options.dedup_id.clone(),
            options: HttpSubmitOptions {
                broadcast_mode: options.broadcast_mode,
                preferred_sender: options.preferred_sender.clone(),
                max_retries: options.max_retries,
                timeout_ms: options.timeout_ms,
            },
        };

        debug!(url = %submit_url, request_id = %request_id, "Submitting transaction via HTTP");

        let mut req = self.client.post(&submit_url).json(&request);

        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }

        let response = req.send().await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SdkError::auth("Invalid API key"));
        }

        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SdkError::RateLimited("Too many requests".to_string()));
        }

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(SdkError::transaction(format!(
                "Transaction submission failed: {}",
                error_text
            )));
        }

        let result: HttpSubmitResponse = response.json().await?;

        Ok(TransactionResult {
            request_id: result.request_id,
            transaction_id: result.transaction_id,
            signature: result.signature,
            status: parse_status(&result.status),
            slot: result.slot,
            timestamp: result.timestamp,
            routing: result.routing.map(|r| crate::types::RoutingInfo {
                region: r.region,
                sender: r.sender,
                routing_latency_ms: r.routing_latency_ms,
                sender_latency_ms: r.sender_latency_ms,
                total_latency_ms: r.total_latency_ms,
            }),
            error: result.error.map(|e| crate::types::TransactionError {
                code: e.code,
                message: e.message,
                details: e.details,
            }),
        })
    }

    async fn subscribe_leader_hints(&self) -> Result<mpsc::Receiver<LeaderHint>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let (tx, rx) = mpsc::channel(32);
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let auth = self.auth_header();

        tokio::spawn(async move {
            let mut last_id: Option<String> = None;

            loop {
                let url = format!("{}/v1/stream/leader-hints", base_url);
                let mut req = client
                    .get(&url)
                    .query(&[("timeout", "30")]);

                if let Some(ref id) = last_id {
                    req = req.query(&[("lastId", id.as_str())]);
                }

                if let Some(ref auth) = auth {
                    req = req.header("Authorization", auth.clone());
                }

                match req.send().await {
                    Ok(response) if response.status().is_success() => {
                        if let Ok(poll_response) = response.json::<PollResponse<LeaderHint>>().await {
                            if let Some(data) = poll_response.data {
                                last_id = Some(poll_response.message_id);
                                if tx.send(data).await.is_err() {
                                    break; // Receiver dropped
                                }
                            }
                        }
                    }
                    Ok(response) => {
                        warn!(status = %response.status(), "Leader hints poll failed");
                    }
                    Err(e) => {
                        warn!(error = %e, "Leader hints poll error");
                    }
                }

                tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
            }
        });

        Ok(rx)
    }

    async fn subscribe_tip_instructions(&self) -> Result<mpsc::Receiver<TipInstruction>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let (tx, rx) = mpsc::channel(32);
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let auth = self.auth_header();

        tokio::spawn(async move {
            let mut last_id: Option<String> = None;

            loop {
                let url = format!("{}/v1/stream/tip-instructions", base_url);
                let mut req = client
                    .get(&url)
                    .query(&[("timeout", "30")]);

                if let Some(ref id) = last_id {
                    req = req.query(&[("lastId", id.as_str())]);
                }

                if let Some(ref auth) = auth {
                    req = req.header("Authorization", auth.clone());
                }

                match req.send().await {
                    Ok(response) if response.status().is_success() => {
                        if let Ok(poll_response) = response.json::<PollResponse<TipInstruction>>().await {
                            if let Some(data) = poll_response.data {
                                last_id = Some(poll_response.message_id);
                                if tx.send(data).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(response) => {
                        warn!(status = %response.status(), "Tip instructions poll failed");
                    }
                    Err(e) => {
                        warn!(error = %e, "Tip instructions poll error");
                    }
                }

                tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
            }
        });

        Ok(rx)
    }

    async fn subscribe_priority_fees(&self) -> Result<mpsc::Receiver<PriorityFee>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let (tx, rx) = mpsc::channel(32);
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let auth = self.auth_header();

        tokio::spawn(async move {
            let mut last_id: Option<String> = None;

            loop {
                let url = format!("{}/v1/stream/priority-fees", base_url);
                let mut req = client
                    .get(&url)
                    .query(&[("timeout", "30")]);

                if let Some(ref id) = last_id {
                    req = req.query(&[("lastId", id.as_str())]);
                }

                if let Some(ref auth) = auth {
                    req = req.header("Authorization", auth.clone());
                }

                match req.send().await {
                    Ok(response) if response.status().is_success() => {
                        if let Ok(poll_response) = response.json::<PollResponse<PriorityFee>>().await {
                            if let Some(data) = poll_response.data {
                                last_id = Some(poll_response.message_id);
                                if tx.send(data).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(response) => {
                        warn!(status = %response.status(), "Priority fees poll failed");
                    }
                    Err(e) => {
                        warn!(error = %e, "Priority fees poll error");
                    }
                }

                tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
            }
        });

        Ok(rx)
    }

    async fn subscribe_latest_blockhash(&self) -> Result<mpsc::Receiver<LatestBlockhash>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let (tx, rx) = mpsc::channel(32);
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let auth = self.auth_header();

        tokio::spawn(async move {
            let mut last_id: Option<String> = None;

            loop {
                let url = format!("{}/v1/stream/latest-blockhash", base_url);
                let mut req = client
                    .get(&url)
                    .query(&[("timeout", "30")]);

                if let Some(ref id) = last_id {
                    req = req.query(&[("lastId", id.as_str())]);
                }

                if let Some(ref auth) = auth {
                    req = req.header("Authorization", auth.clone());
                }

                match req.send().await {
                    Ok(response) if response.status().is_success() => {
                        if let Ok(poll_response) = response.json::<PollResponse<LatestBlockhash>>().await {
                            if let Some(data) = poll_response.data {
                                last_id = Some(poll_response.message_id);
                                if tx.send(data).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(response) => {
                        warn!(status = %response.status(), "Latest blockhash poll failed");
                    }
                    Err(e) => {
                        warn!(error = %e, "Latest blockhash poll error");
                    }
                }

                tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
            }
        });

        Ok(rx)
    }

    async fn subscribe_latest_slot(&self) -> Result<mpsc::Receiver<LatestSlot>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let (tx, rx) = mpsc::channel(32);
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let auth = self.auth_header();

        tokio::spawn(async move {
            let mut last_id: Option<String> = None;

            loop {
                let url = format!("{}/v1/stream/latest-slot", base_url);
                let mut req = client
                    .get(&url)
                    .query(&[("timeout", "30")]);

                if let Some(ref id) = last_id {
                    req = req.query(&[("lastId", id.as_str())]);
                }

                if let Some(ref auth) = auth {
                    req = req.header("Authorization", auth.clone());
                }

                match req.send().await {
                    Ok(response) if response.status().is_success() => {
                        if let Ok(poll_response) = response.json::<PollResponse<LatestSlot>>().await {
                            if let Some(data) = poll_response.data {
                                last_id = Some(poll_response.message_id);
                                if tx.send(data).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(response) => {
                        warn!(status = %response.status(), "Latest slot poll failed");
                    }
                    Err(e) => {
                        warn!(error = %e, "Latest slot poll error");
                    }
                }

                tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
            }
        });

        Ok(rx)
    }

    async fn ping(&self) -> Result<PingResult> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let client_send_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let url = format!("{}/v1/ping", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SdkError::connection(format!("Ping failed: {}", e)))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| SdkError::connection(format!("Invalid ping response: {}", e)))?;

        let server_time = body["server_time"].as_u64().unwrap_or(now);
        let rtt_ms = now.saturating_sub(client_send_time);
        let clock_offset_ms = server_time as i64 - (client_send_time as i64 + rtt_ms as i64 / 2);

        Ok(PingResult {
            seq: 0,
            rtt_ms,
            clock_offset_ms,
            server_time,
        })
    }
}

// Helper types for HTTP requests/responses

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HttpSubmitRequest {
    request_id: String,
    transaction: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    dedup_id: Option<String>,
    options: HttpSubmitOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HttpSubmitOptions {
    broadcast_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    preferred_sender: Option<String>,
    max_retries: u32,
    timeout_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HttpSubmitResponse {
    request_id: String,
    transaction_id: String,
    signature: Option<String>,
    status: String,
    slot: Option<u64>,
    timestamp: u64,
    routing: Option<HttpRoutingInfo>,
    error: Option<HttpErrorInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HttpRoutingInfo {
    region: String,
    sender: String,
    routing_latency_ms: u32,
    sender_latency_ms: u32,
    total_latency_ms: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HttpErrorInfo {
    code: String,
    message: String,
    details: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PollResponse<T> {
    data: Option<T>,
    message_id: String,
    timestamp: u64,
    has_more: bool,
}

fn parse_status(status: &str) -> TransactionStatus {
    match status.to_lowercase().as_str() {
        "pending" => TransactionStatus::Pending,
        "processing" => TransactionStatus::Processing,
        "sent" => TransactionStatus::Sent,
        "confirmed" => TransactionStatus::Confirmed,
        "failed" => TransactionStatus::Failed,
        "duplicate" => TransactionStatus::Duplicate,
        "rate_limited" | "ratelimited" => TransactionStatus::RateLimited,
        "insufficient_tokens" | "insufficienttokens" => TransactionStatus::InsufficientTokens,
        _ => TransactionStatus::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_transport_new() {
        let transport = HttpTransport::new();
        assert!(!transport.is_connected());
        assert_eq!(transport.protocol(), Protocol::Http);
    }

    #[test]
    fn test_request_id_generation() {
        let transport = HttpTransport::new();
        let id1 = transport.next_request_id();
        let id2 = transport.next_request_id();
        assert_ne!(id1, id2);
        assert!(id1.starts_with("req-"));
    }

    #[test]
    fn test_parse_status() {
        assert_eq!(parse_status("pending"), TransactionStatus::Pending);
        assert_eq!(parse_status("CONFIRMED"), TransactionStatus::Confirmed);
        assert_eq!(parse_status("rate_limited"), TransactionStatus::RateLimited);
        assert_eq!(parse_status("unknown"), TransactionStatus::Pending);
    }
}
