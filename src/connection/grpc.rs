//! gRPC transport implementation using tonic
//!
//! This provides a reliable fallback when QUIC is unavailable.

use super::Transport;
use crate::config::{Config, Protocol};
use crate::error::{Result, SdkError};
use crate::types::{
    ConnectionInfo, LatestBlockhash, LatestSlot, LeaderHint, LeaderHintMetadata, PriorityFee,
    RateLimitInfo, SubmitOptions, TipInstruction, TransactionResult, TransactionStatus,
};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tonic::transport::Channel;
use tracing::{debug, info, warn};

// Include the generated protobuf code
pub mod proto {
    tonic::include_proto!("slipstream");
}

use proto::slipstream_service_client::SlipstreamServiceClient;

/// gRPC transport implementation
pub struct GrpcTransport {
    client: Option<SlipstreamServiceClient<Channel>>,
    config: Option<Config>,
    connected: AtomicBool,
    session_id: Option<String>,
}

impl GrpcTransport {
    /// Create a new gRPC transport
    pub fn new() -> Self {
        Self {
            client: None,
            config: None,
            connected: AtomicBool::new(false),
            session_id: None,
        }
    }

    /// Parse gRPC endpoint URL
    fn parse_endpoint(url: &str) -> String {
        // Expected format: http://host:port or grpc://host:port
        let url = url.trim_start_matches("grpc://");
        if url.starts_with("http://") || url.starts_with("https://") {
            url.to_string()
        } else {
            format!("http://{}", url)
        }
    }

    /// Convert proto LeaderHint to SDK type
    fn convert_leader_hint(proto: proto::LeaderHint) -> LeaderHint {
        LeaderHint {
            timestamp: proto.timestamp,
            slot: proto.slot,
            expires_at_slot: proto.expires_at_slot,
            preferred_region: proto.preferred_region,
            backup_regions: proto.backup_regions,
            confidence: proto.confidence,
            leader_pubkey: proto.leader_pubkey,
            metadata: proto.metadata.map(|m| LeaderHintMetadata {
                tpu_rtt_ms: m.tpu_rtt_ms,
                region_score: m.region_score,
                leader_tpu_address: None,
                region_rtt_ms: None,
            }).unwrap_or(LeaderHintMetadata {
                tpu_rtt_ms: 0,
                region_score: 0.0,
                leader_tpu_address: None,
                region_rtt_ms: None,
            }),
        }
    }

    /// Convert proto TipInstruction to SDK type
    fn convert_tip_instruction(proto: proto::TipInstruction) -> TipInstruction {
        TipInstruction {
            timestamp: proto.timestamp,
            sender: proto.sender,
            sender_name: proto.sender_name,
            tip_wallet_address: proto.tip_wallet_address,
            tip_amount_sol: proto.tip_amount_sol,
            tip_tier: proto.tip_tier,
            expected_latency_ms: proto.expected_latency_ms,
            confidence: proto.confidence,
            valid_until_slot: proto.valid_until_slot,
            alternative_senders: proto
                .alternative_senders
                .into_iter()
                .map(|a| crate::types::AlternativeSender {
                    sender: a.sender,
                    tip_amount_sol: a.tip_amount_sol,
                    confidence: a.confidence,
                })
                .collect(),
        }
    }

    /// Convert proto PriorityFee to SDK type
    fn convert_priority_fee(proto: proto::PriorityFee) -> PriorityFee {
        PriorityFee {
            timestamp: proto.timestamp,
            speed: proto.speed,
            compute_unit_price: proto.compute_unit_price,
            compute_unit_limit: proto.compute_unit_limit,
            estimated_cost_sol: proto.estimated_cost_sol,
            landing_probability: proto.landing_probability,
            network_congestion: proto.network_congestion,
            recent_success_rate: proto.recent_success_rate,
        }
    }

    /// Convert proto TransactionStatus to SDK type
    fn convert_status(status: i32) -> TransactionStatus {
        match proto::TransactionStatus::try_from(status) {
            Ok(proto::TransactionStatus::Pending) => TransactionStatus::Pending,
            Ok(proto::TransactionStatus::Processing) => TransactionStatus::Processing,
            Ok(proto::TransactionStatus::Sent) => TransactionStatus::Sent,
            Ok(proto::TransactionStatus::Confirmed) => TransactionStatus::Confirmed,
            Ok(proto::TransactionStatus::Failed) => TransactionStatus::Failed,
            _ => TransactionStatus::Pending,
        }
    }
}

impl Default for GrpcTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Transport for GrpcTransport {
    async fn connect(&mut self, config: &Config) -> Result<ConnectionInfo> {
        let endpoint_url = config.get_endpoint(Protocol::Grpc);
        self.config = Some(config.clone());

        let grpc_url = Self::parse_endpoint(&endpoint_url);
        debug!(endpoint = %grpc_url, "Connecting via gRPC");

        // Create channel with timeout
        let channel = Channel::from_shared(grpc_url.clone())
            .map_err(|e| SdkError::connection(format!("Invalid gRPC endpoint: {}", e)))?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .connect()
            .await
            .map_err(|e| SdkError::connection(format!("gRPC connection failed: {}", e)))?;

        let mut client = SlipstreamServiceClient::new(channel);

        // Verify connection with GetConnectionStatus
        let request = tonic::Request::new(proto::ConnectionStatusRequest {
            api_key: config.api_key.clone(),
        });

        let response = client
            .get_connection_status(request)
            .await
            .map_err(|e| SdkError::auth(format!("Connection status check failed: {}", e)))?;

        let status = response.into_inner();

        if !status.connected {
            return Err(SdkError::auth("Server rejected connection"));
        }

        self.client = Some(client);
        self.session_id = Some(status.session_id.clone());
        self.connected.store(true, Ordering::SeqCst);

        info!(
            session_id = %status.session_id,
            region = %status.region,
            "gRPC transport connected"
        );

        Ok(ConnectionInfo {
            session_id: status.session_id,
            protocol: "grpc".to_string(),
            region: Some(status.region),
            server_time: status.server_time,
            features: status.enabled_features,
            rate_limit: status.rate_limit.map(|r| RateLimitInfo {
                rps: r.requests_per_second,
                burst: r.burst_size,
            }).unwrap_or(RateLimitInfo { rps: 100, burst: 200 }),
        })
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.client = None;
        self.session_id = None;
        debug!("gRPC transport disconnected");
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn protocol(&self) -> Protocol {
        Protocol::Grpc
    }

    async fn submit_transaction(
        &self,
        transaction: &[u8],
        options: &SubmitOptions,
    ) -> Result<TransactionResult> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let client = self.client.as_ref().ok_or(SdkError::NotConnected)?;
        let mut client = client.clone();

        let request_id = uuid::Uuid::new_v4().to_string();

        debug!(
            request_id = %request_id,
            tx_size = transaction.len(),
            "Submitting transaction via gRPC"
        );

        // Create transaction request
        let tx_request = proto::TransactionRequest {
            request_id: request_id.clone(),
            transaction: transaction.to_vec(),
            dedup_id: options.dedup_id.clone().unwrap_or_default(),
            options: Some(proto::TransactionOptions {
                broadcast_mode: options.broadcast_mode,
                preferred_sender: options.preferred_sender.clone().unwrap_or_default(),
                max_retries: options.max_retries,
                timeout_ms: options.timeout_ms,
            }),
        };

        // Create a stream with single request
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.send(tx_request).await.map_err(|_| SdkError::Internal("Failed to send request".into()))?;
        drop(tx);

        let request_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let request = tonic::Request::new(request_stream);

        // Send and receive response stream
        let response = client
            .submit_transaction(request)
            .await
            .map_err(|e| SdkError::transaction(format!("Transaction submission failed: {}", e)))?;

        let mut stream = response.into_inner();

        // Read first response (final status)
        let response = stream
            .message()
            .await
            .map_err(|e| SdkError::transaction(format!("Failed to read response: {}", e)))?
            .ok_or_else(|| SdkError::transaction("Empty response"))?;

        // Convert to SDK type
        let status = Self::convert_status(response.status);

        let signature = response.confirmation.as_ref().map(|c| c.signature.clone());
        let slot = response.confirmation.as_ref().map(|c| c.slot);

        let routing = response.confirmation.map(|c| crate::types::RoutingInfo {
            region: response.routing.as_ref().map(|r| r.selected_region.clone()).unwrap_or_default(),
            sender: response.routing.as_ref().map(|r| r.selected_sender.clone()).unwrap_or_default(),
            routing_latency_ms: c.routing_latency_ms,
            sender_latency_ms: c.sender_latency_ms,
            total_latency_ms: c.total_latency_ms,
        });

        let error = response.error.map(|e| crate::types::TransactionError {
            code: e.code,
            message: e.message,
            details: if e.details.is_empty() {
                None
            } else {
                Some(serde_json::json!(e.details))
            },
        });

        Ok(TransactionResult {
            request_id: response.request_id,
            transaction_id: response.transaction_id,
            signature,
            status,
            slot,
            timestamp: response.timestamp,
            routing,
            error,
        })
    }

    async fn subscribe_leader_hints(&self) -> Result<mpsc::Receiver<LeaderHint>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let client = self.client.as_ref().ok_or(SdkError::NotConnected)?;
        let mut client = client.clone();
        let config = self.config.as_ref().ok_or(SdkError::NotConnected)?;

        let (tx, rx) = mpsc::channel(32);

        let request = tonic::Request::new(proto::SubscriptionRequest {
            api_key: config.api_key.clone(),
            region: config.region.clone().unwrap_or_default(),
        });

        let response = client
            .subscribe_leader_hints(request)
            .await
            .map_err(|e| SdkError::connection(format!("Failed to subscribe to leader hints: {}", e)))?;

        let mut stream = response.into_inner();

        tokio::spawn(async move {
            loop {
                match stream.message().await {
                    Ok(Some(hint)) => {
                        let sdk_hint = Self::convert_leader_hint(hint);
                        if tx.send(sdk_hint).await.is_err() {
                            debug!("Receiver dropped, stopping leader hints subscription");
                            break;
                        }
                    }
                    Ok(None) => {
                        debug!("Leader hints stream ended");
                        break;
                    }
                    Err(e) => {
                        warn!(error = %e, "Error reading leader hints");
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn subscribe_tip_instructions(&self) -> Result<mpsc::Receiver<TipInstruction>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let client = self.client.as_ref().ok_or(SdkError::NotConnected)?;
        let mut client = client.clone();
        let config = self.config.as_ref().ok_or(SdkError::NotConnected)?;

        let (tx, rx) = mpsc::channel(32);

        let request = tonic::Request::new(proto::SubscriptionRequest {
            api_key: config.api_key.clone(),
            region: config.region.clone().unwrap_or_default(),
        });

        let response = client
            .subscribe_tip_instructions(request)
            .await
            .map_err(|e| SdkError::connection(format!("Failed to subscribe to tip instructions: {}", e)))?;

        let mut stream = response.into_inner();

        tokio::spawn(async move {
            loop {
                match stream.message().await {
                    Ok(Some(tip)) => {
                        let sdk_tip = Self::convert_tip_instruction(tip);
                        if tx.send(sdk_tip).await.is_err() {
                            debug!("Receiver dropped, stopping tip instructions subscription");
                            break;
                        }
                    }
                    Ok(None) => {
                        debug!("Tip instructions stream ended");
                        break;
                    }
                    Err(e) => {
                        warn!(error = %e, "Error reading tip instructions");
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn subscribe_priority_fees(&self) -> Result<mpsc::Receiver<PriorityFee>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let client = self.client.as_ref().ok_or(SdkError::NotConnected)?;
        let mut client = client.clone();
        let config = self.config.as_ref().ok_or(SdkError::NotConnected)?;

        let (tx, rx) = mpsc::channel(32);

        let request = tonic::Request::new(proto::SubscriptionRequest {
            api_key: config.api_key.clone(),
            region: config.region.clone().unwrap_or_default(),
        });

        let response = client
            .subscribe_priority_fees(request)
            .await
            .map_err(|e| SdkError::connection(format!("Failed to subscribe to priority fees: {}", e)))?;

        let mut stream = response.into_inner();

        tokio::spawn(async move {
            loop {
                match stream.message().await {
                    Ok(Some(fee)) => {
                        let sdk_fee = Self::convert_priority_fee(fee);
                        if tx.send(sdk_fee).await.is_err() {
                            debug!("Receiver dropped, stopping priority fees subscription");
                            break;
                        }
                    }
                    Ok(None) => {
                        debug!("Priority fees stream ended");
                        break;
                    }
                    Err(e) => {
                        warn!(error = %e, "Error reading priority fees");
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn subscribe_latest_blockhash(&self) -> Result<mpsc::Receiver<LatestBlockhash>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let client = self.client.as_ref().ok_or(SdkError::NotConnected)?;
        let mut client = client.clone();
        let config = self.config.as_ref().ok_or(SdkError::NotConnected)?;

        let (tx, rx) = mpsc::channel(32);

        let request = tonic::Request::new(proto::SubscriptionRequest {
            api_key: config.api_key.clone(),
            region: config.region.clone().unwrap_or_default(),
        });

        let response = client
            .subscribe_latest_blockhash(request)
            .await
            .map_err(|e| SdkError::connection(format!("Failed to subscribe to latest blockhash: {}", e)))?;

        let mut stream = response.into_inner();

        tokio::spawn(async move {
            loop {
                match stream.message().await {
                    Ok(Some(bh)) => {
                        let sdk_bh = LatestBlockhash {
                            blockhash: bh.blockhash,
                            last_valid_block_height: bh.last_valid_block_height,
                            timestamp: bh.timestamp,
                        };
                        if tx.send(sdk_bh).await.is_err() {
                            debug!("Receiver dropped, stopping blockhash subscription");
                            break;
                        }
                    }
                    Ok(None) => {
                        debug!("Latest blockhash stream ended");
                        break;
                    }
                    Err(e) => {
                        warn!(error = %e, "Error reading latest blockhash");
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn subscribe_latest_slot(&self) -> Result<mpsc::Receiver<LatestSlot>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let client = self.client.as_ref().ok_or(SdkError::NotConnected)?;
        let mut client = client.clone();
        let config = self.config.as_ref().ok_or(SdkError::NotConnected)?;

        let (tx, rx) = mpsc::channel(32);

        let request = tonic::Request::new(proto::SubscriptionRequest {
            api_key: config.api_key.clone(),
            region: config.region.clone().unwrap_or_default(),
        });

        let response = client
            .subscribe_latest_slot(request)
            .await
            .map_err(|e| SdkError::connection(format!("Failed to subscribe to latest slot: {}", e)))?;

        let mut stream = response.into_inner();

        tokio::spawn(async move {
            loop {
                match stream.message().await {
                    Ok(Some(slot)) => {
                        let sdk_slot = LatestSlot {
                            slot: slot.slot,
                            timestamp: slot.timestamp,
                        };
                        if tx.send(sdk_slot).await.is_err() {
                            debug!("Receiver dropped, stopping slot subscription");
                            break;
                        }
                    }
                    Ok(None) => {
                        debug!("Latest slot stream ended");
                        break;
                    }
                    Err(e) => {
                        warn!(error = %e, "Error reading latest slot");
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grpc_transport_new() {
        let transport = GrpcTransport::new();
        assert!(!transport.is_connected());
        assert_eq!(transport.protocol(), Protocol::Grpc);
    }

    #[test]
    fn test_parse_endpoint() {
        assert_eq!(
            GrpcTransport::parse_endpoint("grpc://localhost:10000"),
            "http://localhost:10000"
        );
        assert_eq!(
            GrpcTransport::parse_endpoint("localhost:10000"),
            "http://localhost:10000"
        );
        assert_eq!(
            GrpcTransport::parse_endpoint("http://localhost:10000"),
            "http://localhost:10000"
        );
        assert_eq!(
            GrpcTransport::parse_endpoint("https://api.slipstream.allenhark.com:10000"),
            "https://api.slipstream.allenhark.com:10000"
        );
    }

    #[test]
    fn test_convert_status() {
        assert_eq!(
            GrpcTransport::convert_status(proto::TransactionStatus::Pending as i32),
            TransactionStatus::Pending
        );
        assert_eq!(
            GrpcTransport::convert_status(proto::TransactionStatus::Confirmed as i32),
            TransactionStatus::Confirmed
        );
        assert_eq!(
            GrpcTransport::convert_status(proto::TransactionStatus::Failed as i32),
            TransactionStatus::Failed
        );
    }
}
