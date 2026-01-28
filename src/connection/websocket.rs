//! WebSocket transport implementation using tokio-tungstenite
//!
//! This provides a browser-compatible fallback when QUIC and gRPC are unavailable.

use super::Transport;
use crate::config::{Config, Protocol};
use crate::error::{Result, SdkError};
use crate::types::{
    AlternativeSender, ConnectionInfo, LeaderHint, LeaderHintMetadata, PriorityFee, RateLimitInfo,
    SubmitOptions, TipInstruction, TransactionResult, TransactionStatus,
};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

/// WebSocket client message types
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Connect {
        version: String,
        api_key: String,
        region: Option<String>,
    },
    Subscribe {
        stream: String,
    },
    Unsubscribe {
        stream: String,
    },
    SubmitTransaction {
        request_id: String,
        transaction: String, // base64 encoded
        #[serde(skip_serializing_if = "Option::is_none")]
        dedup_id: Option<String>,
        options: WsSubmitOptions,
    },
    Pong,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WsSubmitOptions {
    broadcast_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    preferred_sender: Option<String>,
    max_retries: u32,
    timeout_ms: u64,
}

/// WebSocket server message types
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Connected {
        session_id: String,
        region: Option<String>,
        server_time: u64,
        features: Vec<String>,
        rate_limit: WsRateLimitInfo,
    },
    Error {
        code: String,
        message: String,
    },
    LeaderHint(WsLeaderHint),
    TipInstruction(WsTipInstruction),
    PriorityFee(WsPriorityFee),
    TransactionAccepted {
        request_id: String,
        transaction_id: String,
    },
    TransactionUpdate {
        request_id: String,
        transaction_id: String,
        status: String,
        timestamp: u64,
    },
    TransactionConfirmed {
        request_id: String,
        transaction_id: String,
        signature: String,
        slot: u64,
        routing: Option<WsRoutingInfo>,
    },
    TransactionFailed {
        request_id: String,
        transaction_id: String,
        error: WsErrorInfo,
    },
    Heartbeat,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsRateLimitInfo {
    rps: u32,
    burst: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsLeaderHint {
    timestamp: u64,
    slot: u64,
    expires_at_slot: u64,
    preferred_region: String,
    #[serde(default)]
    backup_regions: Vec<String>,
    confidence: u32,
    leader_pubkey: Option<String>,
    metadata: Option<WsLeaderHintMetadata>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsLeaderHintMetadata {
    tpu_rtt_ms: u32,
    region_score: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsTipInstruction {
    timestamp: u64,
    sender: String,
    sender_name: String,
    tip_wallet_address: String,
    tip_amount_sol: f64,
    tip_tier: String,
    expected_latency_ms: u32,
    confidence: u32,
    valid_until_slot: u64,
    #[serde(default)]
    alternative_senders: Vec<WsAlternativeSender>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsAlternativeSender {
    sender: String,
    tip_amount_sol: f64,
    confidence: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsPriorityFee {
    timestamp: u64,
    speed: String,
    compute_unit_price: u64,
    compute_unit_limit: u32,
    estimated_cost_sol: f64,
    landing_probability: u32,
    network_congestion: String,
    recent_success_rate: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsRoutingInfo {
    region: String,
    sender: String,
    routing_latency_ms: u32,
    sender_latency_ms: u32,
    total_latency_ms: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsErrorInfo {
    code: String,
    message: String,
    details: Option<serde_json::Value>,
}

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

type WsStream = futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

/// WebSocket transport implementation
pub struct WebSocketTransport {
    sink: Arc<RwLock<Option<WsSink>>>,
    config: Option<Config>,
    connected: AtomicBool,
    session_id: RwLock<Option<String>>,
    /// Channel for receiving parsed messages
    message_tx: Option<mpsc::Sender<ServerMessage>>,
    message_rx: Arc<RwLock<Option<mpsc::Receiver<ServerMessage>>>>,
}

impl WebSocketTransport {
    /// Create a new WebSocket transport
    pub fn new() -> Self {
        Self {
            sink: Arc::new(RwLock::new(None)),
            config: None,
            connected: AtomicBool::new(false),
            session_id: RwLock::new(None),
            message_tx: None,
            message_rx: Arc::new(RwLock::new(None)),
        }
    }

    /// Parse WebSocket endpoint URL
    fn parse_endpoint(url: &str) -> String {
        // Expected format: ws://host:port/ws or wss://host:port/ws
        let url = url.trim_start_matches("websocket://");
        if url.starts_with("ws://") || url.starts_with("wss://") {
            url.to_string()
        } else if url.starts_with("https://") {
            url.replace("https://", "wss://")
        } else if url.starts_with("http://") {
            url.replace("http://", "ws://")
        } else {
            format!("wss://{}", url)
        }
    }

    /// Send a message to the server
    async fn send_message(&self, msg: ClientMessage) -> Result<()> {
        let json = serde_json::to_string(&msg)
            .map_err(|e| SdkError::protocol(format!("Failed to serialize message: {}", e)))?;

        let mut sink_guard = self.sink.write().await;
        let sink = sink_guard.as_mut().ok_or(SdkError::NotConnected)?;

        sink.send(Message::Text(json))
            .await
            .map_err(|e| SdkError::connection(format!("Failed to send message: {}", e)))?;

        Ok(())
    }

    /// Start the message receiving loop
    fn spawn_receiver(
        mut stream: WsStream,
        tx: mpsc::Sender<ServerMessage>,
        sink: Arc<RwLock<Option<WsSink>>>,
    ) {
        tokio::spawn(async move {
            loop {
                match stream.next().await {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ServerMessage>(&text) {
                            Ok(msg) => {
                                // Handle heartbeat internally
                                if matches!(msg, ServerMessage::Heartbeat) {
                                    let mut sink_guard = sink.write().await;
                                    if let Some(s) = sink_guard.as_mut() {
                                        let pong = serde_json::to_string(&ClientMessage::Pong).unwrap();
                                        let _ = s.send(Message::Text(pong)).await;
                                    }
                                    continue;
                                }

                                if tx.send(msg).await.is_err() {
                                    debug!("Message receiver dropped");
                                    break;
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, text = %text, "Failed to parse server message");
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let mut sink_guard = sink.write().await;
                        if let Some(s) = sink_guard.as_mut() {
                            let _ = s.send(Message::Pong(data)).await;
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        debug!("WebSocket connection closed by server");
                        break;
                    }
                    Some(Err(e)) => {
                        warn!(error = %e, "WebSocket receive error");
                        break;
                    }
                    None => {
                        debug!("WebSocket stream ended");
                        break;
                    }
                    _ => {}
                }
            }
        });
    }

    /// Convert WsLeaderHint to SDK type
    fn convert_leader_hint(ws: WsLeaderHint) -> LeaderHint {
        LeaderHint {
            timestamp: ws.timestamp,
            slot: ws.slot,
            expires_at_slot: ws.expires_at_slot,
            preferred_region: ws.preferred_region,
            backup_regions: ws.backup_regions,
            confidence: ws.confidence,
            leader_pubkey: ws.leader_pubkey,
            metadata: ws.metadata.map(|m| LeaderHintMetadata {
                tpu_rtt_ms: m.tpu_rtt_ms,
                region_score: m.region_score,
            }).unwrap_or(LeaderHintMetadata {
                tpu_rtt_ms: 0,
                region_score: 0.0,
            }),
        }
    }

    /// Convert WsTipInstruction to SDK type
    fn convert_tip_instruction(ws: WsTipInstruction) -> TipInstruction {
        TipInstruction {
            timestamp: ws.timestamp,
            sender: ws.sender,
            sender_name: ws.sender_name,
            tip_wallet_address: ws.tip_wallet_address,
            tip_amount_sol: ws.tip_amount_sol,
            tip_tier: ws.tip_tier,
            expected_latency_ms: ws.expected_latency_ms,
            confidence: ws.confidence,
            valid_until_slot: ws.valid_until_slot,
            alternative_senders: ws.alternative_senders.into_iter().map(|a| AlternativeSender {
                sender: a.sender,
                tip_amount_sol: a.tip_amount_sol,
                confidence: a.confidence,
            }).collect(),
        }
    }

    /// Convert WsPriorityFee to SDK type
    fn convert_priority_fee(ws: WsPriorityFee) -> PriorityFee {
        PriorityFee {
            timestamp: ws.timestamp,
            speed: ws.speed,
            compute_unit_price: ws.compute_unit_price,
            compute_unit_limit: ws.compute_unit_limit,
            estimated_cost_sol: ws.estimated_cost_sol,
            landing_probability: ws.landing_probability,
            network_congestion: ws.network_congestion,
            recent_success_rate: ws.recent_success_rate,
        }
    }
}

impl Default for WebSocketTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Transport for WebSocketTransport {
    async fn connect(&mut self, config: &Config) -> Result<ConnectionInfo> {
        let endpoint_url = config.get_endpoint(Protocol::WebSocket);
        self.config = Some(config.clone());

        let ws_url = Self::parse_endpoint(&endpoint_url);
        debug!(endpoint = %ws_url, "Connecting via WebSocket");

        // Connect to WebSocket server
        let (ws_stream, _) = tokio::time::timeout(
            Duration::from_secs(10),
            connect_async(&ws_url),
        )
        .await
        .map_err(|_| SdkError::Timeout(Duration::from_secs(10)))?
        .map_err(|e| SdkError::connection(format!("WebSocket connection failed: {}", e)))?;

        let (sink, stream) = ws_stream.split();

        // Store sink
        {
            let mut sink_guard = self.sink.write().await;
            *sink_guard = Some(sink);
        }

        // Create message channel
        let (tx, rx) = mpsc::channel(256);
        self.message_tx = Some(tx.clone());
        {
            let mut rx_guard = self.message_rx.write().await;
            *rx_guard = Some(rx);
        }

        // Start receiver task
        Self::spawn_receiver(stream, tx.clone(), Arc::clone(&self.sink));

        // Send connect message
        self.send_message(ClientMessage::Connect {
            version: "1.0".to_string(),
            api_key: config.api_key.clone(),
            region: config.region.clone(),
        })
        .await?;

        // Wait for Connected response
        let response = {
            let mut rx_guard = self.message_rx.write().await;
            let rx = rx_guard.as_mut().ok_or(SdkError::Internal("No receiver".into()))?;

            tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .map_err(|_| SdkError::Timeout(Duration::from_secs(5)))?
                .ok_or_else(|| SdkError::connection("Connection closed before response"))?
        };

        match response {
            ServerMessage::Connected {
                session_id,
                region,
                server_time,
                features,
                rate_limit,
            } => {
                {
                    let mut session_guard = self.session_id.write().await;
                    *session_guard = Some(session_id.clone());
                }
                self.connected.store(true, Ordering::SeqCst);

                info!(
                    session_id = %session_id,
                    region = ?region,
                    "WebSocket transport connected"
                );

                Ok(ConnectionInfo {
                    session_id,
                    protocol: "websocket".to_string(),
                    region,
                    server_time,
                    features,
                    rate_limit: RateLimitInfo {
                        rps: rate_limit.rps,
                        burst: rate_limit.burst,
                    },
                })
            }
            ServerMessage::Error { code, message } => {
                Err(SdkError::auth(format!("Connection rejected: {} - {}", code, message)))
            }
            _ => Err(SdkError::protocol("Unexpected response to connect")),
        }
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);

        {
            let mut sink_guard = self.sink.write().await;
            if let Some(mut sink) = sink_guard.take() {
                let _ = sink.close().await;
            }
        }

        {
            let mut session_guard = self.session_id.write().await;
            *session_guard = None;
        }

        debug!("WebSocket transport disconnected");
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn protocol(&self) -> Protocol {
        Protocol::WebSocket
    }

    async fn submit_transaction(
        &self,
        transaction: &[u8],
        options: &SubmitOptions,
    ) -> Result<TransactionResult> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let request_id = uuid::Uuid::new_v4().to_string();
        let transaction_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, transaction);

        debug!(
            request_id = %request_id,
            tx_size = transaction.len(),
            "Submitting transaction via WebSocket"
        );

        // Send transaction
        self.send_message(ClientMessage::SubmitTransaction {
            request_id: request_id.clone(),
            transaction: transaction_b64,
            dedup_id: options.dedup_id.clone(),
            options: WsSubmitOptions {
                broadcast_mode: options.broadcast_mode,
                preferred_sender: options.preferred_sender.clone(),
                max_retries: options.max_retries,
                timeout_ms: options.timeout_ms,
            },
        })
        .await?;

        // Wait for response
        let timeout = Duration::from_millis(options.timeout_ms);
        let deadline = tokio::time::Instant::now() + timeout;

        let mut transaction_id = String::new();
        let mut signature = None;
        let mut slot = None;
        let mut status = TransactionStatus::Pending;
        let mut routing = None;
        let mut error = None;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(SdkError::Timeout(timeout));
            }

            let response = {
                let mut rx_guard = self.message_rx.write().await;
                let rx = rx_guard.as_mut().ok_or(SdkError::NotConnected)?;

                tokio::time::timeout(remaining, rx.recv())
                    .await
                    .map_err(|_| SdkError::Timeout(timeout))?
                    .ok_or_else(|| SdkError::connection("Connection closed"))?
            };

            match response {
                ServerMessage::TransactionAccepted { request_id: rid, transaction_id: tid } if rid == request_id => {
                    transaction_id = tid;
                    status = TransactionStatus::Processing;
                }
                ServerMessage::TransactionUpdate { request_id: rid, status: s, .. } if rid == request_id => {
                    status = match s.as_str() {
                        "pending" => TransactionStatus::Pending,
                        "processing" => TransactionStatus::Processing,
                        "sent" => TransactionStatus::Sent,
                        "confirmed" => TransactionStatus::Confirmed,
                        "failed" => TransactionStatus::Failed,
                        _ => TransactionStatus::Processing,
                    };
                }
                ServerMessage::TransactionConfirmed { request_id: rid, transaction_id: tid, signature: sig, slot: s, routing: r } if rid == request_id => {
                    transaction_id = tid;
                    signature = Some(sig);
                    slot = Some(s);
                    status = TransactionStatus::Confirmed;
                    routing = r.map(|ri| crate::types::RoutingInfo {
                        region: ri.region,
                        sender: ri.sender,
                        routing_latency_ms: ri.routing_latency_ms,
                        sender_latency_ms: ri.sender_latency_ms,
                        total_latency_ms: ri.total_latency_ms,
                    });
                    break;
                }
                ServerMessage::TransactionFailed { request_id: rid, transaction_id: tid, error: e } if rid == request_id => {
                    transaction_id = tid;
                    status = TransactionStatus::Failed;
                    error = Some(crate::types::TransactionError {
                        code: e.code,
                        message: e.message,
                        details: e.details,
                    });
                    break;
                }
                _ => {
                    // Ignore other messages
                }
            }
        }

        Ok(TransactionResult {
            request_id,
            transaction_id,
            signature,
            status,
            slot,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            routing,
            error,
        })
    }

    async fn subscribe_leader_hints(&self) -> Result<mpsc::Receiver<LeaderHint>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        self.send_message(ClientMessage::Subscribe {
            stream: "leader_hints".to_string(),
        })
        .await?;

        let (tx, rx) = mpsc::channel(32);
        let message_rx = Arc::clone(&self.message_rx);

        tokio::spawn(async move {
            loop {
                let msg = {
                    let mut rx_guard = message_rx.write().await;
                    match rx_guard.as_mut() {
                        Some(r) => r.recv().await,
                        None => break,
                    }
                };

                match msg {
                    Some(ServerMessage::LeaderHint(hint)) => {
                        let sdk_hint = Self::convert_leader_hint(hint);
                        if tx.send(sdk_hint).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                    _ => {}
                }
            }
        });

        Ok(rx)
    }

    async fn subscribe_tip_instructions(&self) -> Result<mpsc::Receiver<TipInstruction>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        self.send_message(ClientMessage::Subscribe {
            stream: "tip_instructions".to_string(),
        })
        .await?;

        let (tx, rx) = mpsc::channel(32);
        let message_rx = Arc::clone(&self.message_rx);

        tokio::spawn(async move {
            loop {
                let msg = {
                    let mut rx_guard = message_rx.write().await;
                    match rx_guard.as_mut() {
                        Some(r) => r.recv().await,
                        None => break,
                    }
                };

                match msg {
                    Some(ServerMessage::TipInstruction(tip)) => {
                        let sdk_tip = Self::convert_tip_instruction(tip);
                        if tx.send(sdk_tip).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                    _ => {}
                }
            }
        });

        Ok(rx)
    }

    async fn subscribe_priority_fees(&self) -> Result<mpsc::Receiver<PriorityFee>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        self.send_message(ClientMessage::Subscribe {
            stream: "priority_fees".to_string(),
        })
        .await?;

        let (tx, rx) = mpsc::channel(32);
        let message_rx = Arc::clone(&self.message_rx);

        tokio::spawn(async move {
            loop {
                let msg = {
                    let mut rx_guard = message_rx.write().await;
                    match rx_guard.as_mut() {
                        Some(r) => r.recv().await,
                        None => break,
                    }
                };

                match msg {
                    Some(ServerMessage::PriorityFee(fee)) => {
                        let sdk_fee = Self::convert_priority_fee(fee);
                        if tx.send(sdk_fee).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                    _ => {}
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
    fn test_websocket_transport_new() {
        let transport = WebSocketTransport::new();
        assert!(!transport.is_connected());
        assert_eq!(transport.protocol(), Protocol::WebSocket);
    }

    #[test]
    fn test_parse_endpoint() {
        assert_eq!(
            WebSocketTransport::parse_endpoint("ws://localhost:9000/ws"),
            "ws://localhost:9000/ws"
        );
        assert_eq!(
            WebSocketTransport::parse_endpoint("wss://api.slipstream.allenhark.com/ws"),
            "wss://api.slipstream.allenhark.com/ws"
        );
        assert_eq!(
            WebSocketTransport::parse_endpoint("https://api.slipstream.allenhark.com/ws"),
            "wss://api.slipstream.allenhark.com/ws"
        );
        assert_eq!(
            WebSocketTransport::parse_endpoint("localhost:9000/ws"),
            "wss://localhost:9000/ws"
        );
    }

    #[test]
    fn test_client_message_serialization() {
        let msg = ClientMessage::Connect {
            version: "1.0".to_string(),
            api_key: "sk_test123".to_string(),
            region: Some("us-east".to_string()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"connect\""));
        assert!(json.contains("\"api_key\":\"sk_test123\""));
    }

    #[test]
    fn test_server_message_deserialization() {
        let json = r#"{
            "type": "connected",
            "session_id": "sess-123",
            "region": "us-east",
            "server_time": 1706011200000,
            "features": ["streaming"],
            "rate_limit": {"rps": 100, "burst": 200}
        }"#;

        let msg: ServerMessage = serde_json::from_str(json).unwrap();
        match msg {
            ServerMessage::Connected { session_id, .. } => {
                assert_eq!(session_id, "sess-123");
            }
            _ => panic!("Wrong message type"),
        }
    }
}
