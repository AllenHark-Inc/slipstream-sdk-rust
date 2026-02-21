//! QUIC transport implementation using quinn
//!
//! This is the primary transport for low-latency connections.

use super::Transport;
use crate::config::{Config, Protocol};
use crate::error::{Result, SdkError};
use crate::types::{
    ConnectionInfo, LatestBlockhash, LatestSlot, LeaderHint, PingResult, PriorityFee, RateLimitInfo,
    SubmitOptions, TipInstruction, TransactionResult, TransactionStatus,
};
use async_trait::async_trait;
use quinn::{ClientConfig, Connection, Endpoint};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

/// Stream type identifiers (must match server)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    TransactionSubmit = 0x01,
    LeaderHints = 0x02,
    TipInstructions = 0x03,
    PriorityFees = 0x04,
    Metrics = 0x05,
    LatestBlockhash = 0x06,
    LatestSlot = 0x07,
    Ping = 0x08,
}

/// Transaction response status codes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseStatus {
    Accepted = 0x01,
    Duplicate = 0x02,
    RateLimited = 0x03,
    ServerError = 0x04,
}

impl ResponseStatus {
    fn from_byte(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(ResponseStatus::Accepted),
            0x02 => Some(ResponseStatus::Duplicate),
            0x03 => Some(ResponseStatus::RateLimited),
            0x04 => Some(ResponseStatus::ServerError),
            _ => None,
        }
    }
}

/// QUIC transport implementation
pub struct QuicTransport {
    endpoint: Option<Endpoint>,
    connection: RwLock<Option<Connection>>,
    config: Option<Config>,
    connected: AtomicBool,
    session_id: RwLock<Option<String>>,
    request_counter: AtomicU32,
    ping_seq: AtomicU32,
}

impl QuicTransport {
    /// Create a new QUIC transport
    pub fn new() -> Self {
        Self {
            endpoint: None,
            connection: RwLock::new(None),
            config: None,
            connected: AtomicBool::new(false),
            session_id: RwLock::new(None),
            request_counter: AtomicU32::new(0),
            ping_seq: AtomicU32::new(0),
        }
    }

    /// Generate a unique request ID
    fn next_request_id(&self) -> u32 {
        self.request_counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Build QUIC client configuration with custom TLS
    fn build_client_config() -> Result<ClientConfig> {
        // Create rustls config that accepts any certificate (for development)
        // In production, you'd want proper certificate validation
        let mut crypto = rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
            .with_no_client_auth();
        crypto.alpn_protocols = vec![b"slipstream".to_vec()];

        let mut client_config = ClientConfig::new(Arc::new(crypto));

        // Configure transport parameters
        let mut transport_config = quinn::TransportConfig::default();
        transport_config.keep_alive_interval(Some(Duration::from_secs(5)));
        transport_config.max_idle_timeout(Some(
            quinn::IdleTimeout::try_from(Duration::from_secs(30))
                .map_err(|e| SdkError::protocol(format!("Invalid timeout: {}", e)))?,
        ));
        client_config.transport_config(Arc::new(transport_config));

        Ok(client_config)
    }

    /// Parse endpoint URL to socket address
    fn parse_endpoint(url: &str) -> Result<(SocketAddr, String)> {
        // Expected format: quic://host:port or just host:port
        let url = url.trim_start_matches("quic://");

        // Split host and port
        let parts: Vec<&str> = url.rsplitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(SdkError::config("Invalid QUIC endpoint format, expected host:port"));
        }

        let port: u16 = parts[0]
            .parse()
            .map_err(|_| SdkError::config("Invalid port number"))?;
        let host = parts[1];

        // Resolve to socket address (use first resolved address)
        let addr_str = format!("{}:{}", host, port);
        let addr: SocketAddr = addr_str
            .parse()
            .or_else(|_| {
                // Try DNS resolution
                use std::net::ToSocketAddrs;
                addr_str
                    .to_socket_addrs()
                    .map_err(|e| SdkError::connection(format!("DNS resolution failed: {}", e)))?
                    .next()
                    .ok_or_else(|| SdkError::connection("No addresses found"))
            })?;

        Ok((addr, host.to_string()))
    }

    /// Perform authentication on the connection
    async fn authenticate(
        &self,
        connection: &Connection,
        api_key: &str,
    ) -> Result<String> {
        // Open first bi-directional stream for authentication
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| SdkError::connection(format!("Failed to open auth stream: {}", e)))?;

        // Build auth frame: 16-byte key prefix + optional version string
        // Key prefix is first 16 chars of the API key (e.g. "sk_live_MPn7cQvz")
        let key_prefix = &api_key[..16.min(api_key.len())];

        let mut auth_frame = [0u8; 64];
        let prefix_bytes = key_prefix.as_bytes();
        let prefix_len = prefix_bytes.len().min(16);
        auth_frame[..prefix_len].copy_from_slice(&prefix_bytes[..prefix_len]);

        // Add client version string after 16-byte key prefix
        let version = b"rust-sdk-v0.1";
        auth_frame[16..16 + version.len()].copy_from_slice(version);

        // Send auth frame
        send.write_all(&auth_frame[..16 + version.len()])
            .await
            .map_err(|e| SdkError::connection(format!("Failed to send auth: {}", e)))?;

        // Read auth response
        let mut response_buf = [0u8; 256];
        let n = recv
            .read(&mut response_buf)
            .await
            .map_err(|e| SdkError::connection(format!("Failed to read auth response: {}", e)))?
            .ok_or_else(|| SdkError::auth("Connection closed during authentication"))?;

        if n < 1 {
            return Err(SdkError::auth("Empty auth response"));
        }

        // Parse response: first byte is status (0x01 = success, 0x00 = error)
        let status = response_buf[0];
        let message = String::from_utf8_lossy(&response_buf[1..n]).to_string();

        if status == 0x01 {
            debug!(message = %message, "QUIC authentication successful");
            Ok(message)
        } else {
            Err(SdkError::auth(format!("Authentication failed: {}", message)))
        }
    }

    /// Subscribe to a stream type.
    ///
    /// Sends a subscription request via a uni-stream, then accepts the server's
    /// response uni-stream and reads length-prefixed messages from it in a loop.
    async fn subscribe_stream<T, F>(
        &self,
        stream_type: StreamType,
        decoder: F,
    ) -> Result<mpsc::Receiver<T>>
    where
        T: Send + 'static,
        F: Fn(&[u8]) -> Option<T> + Send + Sync + 'static,
    {
        let connection = {
            let guard = self.connection.read().await;
            guard
                .as_ref()
                .ok_or(SdkError::NotConnected)?
                .clone()
        };

        let (tx, rx) = mpsc::channel(32);

        // Open a uni-directional stream to request subscription
        let mut send = connection
            .open_uni()
            .await
            .map_err(|e| SdkError::connection(format!("Failed to open subscription stream: {}", e)))?;

        // Send stream type byte
        send.write_all(&[stream_type as u8])
            .await
            .map_err(|e| SdkError::connection(format!("Failed to send subscription request: {}", e)))?;

        // Close our send side. The server may send STOP_SENDING after reading
        // the subscription byte, which is harmless — just means it's done receiving.
        if let Err(e) = send.finish().await {
            debug!(
                stream_type = ?stream_type,
                error = %e,
                "Stream finish returned error (harmless — server acknowledged subscription)"
            );
        }

        debug!(
            stream_type = ?stream_type,
            "Subscription request sent, waiting for server stream"
        );

        // Accept the server's response uni-stream for this subscription.
        // The server opens one persistent uni-stream per subscription type.
        let conn = connection.clone();
        tokio::spawn(async move {
            // Accept the server's response stream
            let mut recv = match conn.accept_uni().await {
                Ok(r) => {
                    debug!(
                        stream_type = ?stream_type,
                        "Accepted server subscription stream"
                    );
                    r
                }
                Err(quinn::ConnectionError::ApplicationClosed(_)) => {
                    debug!("Connection closed before subscription stream accepted");
                    return;
                }
                Err(e) => {
                    warn!(
                        stream_type = ?stream_type,
                        error = %e,
                        "Failed to accept subscription stream from server"
                    );
                    return;
                }
            };

            // Read length-prefixed messages from the persistent stream.
            // Wire format: [2-byte big-endian length] [message bytes]
            let mut msg_count: u64 = 0;
            loop {
                // Read 2-byte length prefix
                let mut len_buf = [0u8; 2];
                match recv.read_exact(&mut len_buf).await {
                    Ok(()) => {}
                    Err(e) => {
                        // ReadExactError means stream ended or connection closed
                        if msg_count == 0 {
                            warn!(
                                stream_type = ?stream_type,
                                error = %e,
                                "Subscription stream closed before any data received"
                            );
                        } else {
                            debug!(
                                stream_type = ?stream_type,
                                messages_received = msg_count,
                                "Subscription stream ended"
                            );
                        }
                        break;
                    }
                }

                let msg_len = u16::from_be_bytes(len_buf) as usize;
                if msg_len == 0 || msg_len > 4096 {
                    warn!(
                        stream_type = ?stream_type,
                        msg_len = msg_len,
                        "Invalid message length, closing subscription"
                    );
                    break;
                }

                // Read the message body
                let mut msg_buf = vec![0u8; msg_len];
                match recv.read_exact(&mut msg_buf).await {
                    Ok(()) => {}
                    Err(e) => {
                        warn!(
                            stream_type = ?stream_type,
                            error = %e,
                            "Failed to read message body"
                        );
                        break;
                    }
                }

                msg_count += 1;

                // Decode and send to channel
                if let Some(data) = decoder(&msg_buf) {
                    if tx.send(data).await.is_err() {
                        debug!(
                            stream_type = ?stream_type,
                            "Receiver dropped, stopping subscription"
                        );
                        break;
                    }
                } else if msg_count <= 3 {
                    debug!(
                        stream_type = ?stream_type,
                        msg_len = msg_len,
                        first_byte = msg_buf.first().copied().unwrap_or(0),
                        "Failed to decode subscription message"
                    );
                }
            }
        });

        Ok(rx)
    }
}

impl Default for QuicTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Transport for QuicTransport {
    async fn connect(&mut self, config: &Config) -> Result<ConnectionInfo> {
        // If already connected, return existing connection info instead of creating a second connection
        if self.is_connected() {
            let session_id = self.session_id.read().await.clone().unwrap_or_default();
            debug!(session_id = %session_id, "QUIC already connected, reusing existing connection");
            return Ok(ConnectionInfo {
                session_id,
                protocol: "quic".to_string(),
                region: None,
                server_time: 0,
                features: vec![],
                rate_limit: crate::types::RateLimitInfo { rps: 0, burst: 0 },
            });
        }

        let endpoint_url = config.get_endpoint(Protocol::Quic);
        self.config = Some(config.clone());

        debug!(endpoint = %endpoint_url, "Connecting via QUIC");

        // Parse endpoint
        let (server_addr, server_name) = Self::parse_endpoint(&endpoint_url)?;

        // Build client config
        let client_config = Self::build_client_config()?;

        // Create endpoint (bind to any available port)
        let mut endpoint = Endpoint::client("0.0.0.0:0".parse().unwrap())
            .map_err(|e| SdkError::connection(format!("Failed to create endpoint: {}", e)))?;

        endpoint.set_default_client_config(client_config);

        // Connect to server
        let connection = endpoint
            .connect(server_addr, &server_name)
            .map_err(|e| SdkError::connection(format!("Failed to initiate connection: {}", e)))?
            .await
            .map_err(|e| SdkError::connection(format!("Connection failed: {}", e)))?;

        debug!(
            remote = %connection.remote_address(),
            "QUIC connection established"
        );

        // Authenticate
        let auth_result = self.authenticate(&connection, &config.api_key).await?;

        // Store connection
        self.endpoint = Some(endpoint);
        {
            let mut conn_guard = self.connection.write().await;
            *conn_guard = Some(connection.clone());
        }

        // Generate session ID from auth result
        let session_id = if auth_result.starts_with("ok:") {
            format!("quic-{}", &auth_result[3..])
        } else {
            uuid::Uuid::new_v4().to_string()
        };

        {
            let mut session_guard = self.session_id.write().await;
            *session_guard = Some(session_id.clone());
        }

        self.connected.store(true, Ordering::SeqCst);

        info!(
            session_id = %session_id,
            remote = %connection.remote_address(),
            "QUIC transport connected"
        );

        Ok(ConnectionInfo {
            session_id,
            protocol: "quic".to_string(),
            region: config.region.clone(),
            server_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            features: vec!["streaming".to_string(), "bidirectional".to_string()],
            rate_limit: RateLimitInfo { rps: 1000, burst: 2000 },
        })
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);

        {
            let mut conn_guard = self.connection.write().await;
            if let Some(conn) = conn_guard.take() {
                conn.close(0u32.into(), b"client_disconnect");
            }
        }

        {
            let mut session_guard = self.session_id.write().await;
            *session_guard = None;
        }

        if let Some(endpoint) = self.endpoint.take() {
            endpoint.wait_idle().await;
        }

        debug!("QUIC transport disconnected");
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn protocol(&self) -> Protocol {
        Protocol::Quic
    }

    async fn submit_transaction(
        &self,
        transaction: &[u8],
        _options: &SubmitOptions,
    ) -> Result<TransactionResult> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let connection = {
            let guard = self.connection.read().await;
            guard
                .as_ref()
                .ok_or(SdkError::NotConnected)?
                .clone()
        };

        let request_id = self.next_request_id();

        debug!(
            request_id = request_id,
            tx_size = transaction.len(),
            "Submitting transaction via QUIC"
        );

        // Open bi-directional stream for transaction
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| SdkError::connection(format!("Failed to open transaction stream: {}", e)))?;

        // Build transaction frame: stream_type (1 byte) + transaction data
        let mut frame = Vec::with_capacity(1 + transaction.len());
        frame.push(StreamType::TransactionSubmit as u8);
        frame.extend_from_slice(transaction);

        // Send transaction
        send.write_all(&frame)
            .await
            .map_err(|e| SdkError::connection(format!("Failed to send transaction: {}", e)))?;

        // Close send side
        send.finish()
            .await
            .map_err(|e| SdkError::connection(format!("Failed to finish send: {}", e)))?;

        // Read response
        let mut response_buf = vec![0u8; 256];
        let n = recv
            .read(&mut response_buf)
            .await
            .map_err(|e| SdkError::connection(format!("Failed to read response: {}", e)))?
            .ok_or_else(|| SdkError::transaction("Connection closed before response"))?;

        if n < 6 {
            return Err(SdkError::protocol("Response too short"));
        }

        // Parse response:
        // - 4 bytes: request_id (u32 BE)
        // - 1 byte: status
        // - 1 byte: has_signature
        // - 64 bytes: signature (if has_signature)
        // - 2 bytes: error_len (u16 BE)
        // - N bytes: error message

        let resp_request_id = u32::from_be_bytes([
            response_buf[0],
            response_buf[1],
            response_buf[2],
            response_buf[3],
        ]);
        let status_byte = response_buf[4];
        let has_signature = response_buf[5] != 0;

        let mut offset = 6;

        // Parse signature if present
        let signature = if has_signature && n >= offset + 64 {
            let sig = &response_buf[offset..offset + 64];
            offset += 64;
            Some(bs58::encode(sig).into_string())
        } else {
            None
        };

        // Parse error message
        let error = if n >= offset + 2 {
            let error_len = u16::from_be_bytes([response_buf[offset], response_buf[offset + 1]]) as usize;
            offset += 2;
            if error_len > 0 && n >= offset + error_len {
                Some(String::from_utf8_lossy(&response_buf[offset..offset + error_len]).to_string())
            } else {
                None
            }
        } else {
            None
        };

        // Map status to TransactionStatus
        let status = match ResponseStatus::from_byte(status_byte) {
            Some(ResponseStatus::Accepted) => TransactionStatus::Sent,
            Some(ResponseStatus::Duplicate) => TransactionStatus::Duplicate,
            Some(ResponseStatus::RateLimited) => TransactionStatus::RateLimited,
            Some(ResponseStatus::ServerError) => TransactionStatus::Failed,
            None => TransactionStatus::Failed,
        };

        let transaction_id = format!("tx-{}", request_id);

        Ok(TransactionResult {
            request_id: format!("req-{}", resp_request_id),
            transaction_id,
            signature,
            status,
            slot: None,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            routing: None,
            error: error.map(|msg| crate::types::TransactionError {
                code: "QUIC_ERROR".to_string(),
                message: msg,
                details: None,
            }),
        })
    }

    async fn subscribe_leader_hints(&self) -> Result<mpsc::Receiver<LeaderHint>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        self.subscribe_stream(StreamType::LeaderHints, |data| {
            // Parse LeaderHint wire format:
            // - 1 byte: stream type (0x02)
            // - 1 byte: region_id length
            // - N bytes: region_id
            // - 2 bytes: confidence (u16, scaled 0-10000)
            // - 4 bytes: slots_remaining (u32 BE)
            // - 1 byte: leader_pubkey length
            // - N bytes: leader_pubkey
            // - 8 bytes: timestamp (u64 BE)

            if data.len() < 2 || data[0] != StreamType::LeaderHints as u8 {
                return None;
            }

            let mut offset = 1;
            if offset >= data.len() { return None; }
            let region_len = data[offset] as usize;
            offset += 1;

            if data.len() < offset + region_len + 7 { // min: region + conf(2) + slots(4) + pubkey_len(1)
                return None;
            }

            let preferred_region = String::from_utf8_lossy(&data[offset..offset + region_len]).to_string();
            offset += region_len;

            let confidence_raw = u16::from_be_bytes([data[offset], data[offset + 1]]);
            // Scale from 0-10000 to 0-100
            let confidence = (confidence_raw / 100) as u32;
            offset += 2;

            let slots_remaining = u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            offset += 4;

            // Leader pubkey (length-prefixed)
            if offset >= data.len() { return None; }
            let pubkey_len = data[offset] as usize;
            offset += 1;
            let leader_pubkey = if pubkey_len > 0 && offset + pubkey_len <= data.len() {
                let pk = String::from_utf8_lossy(&data[offset..offset + pubkey_len]).to_string();
                offset += pubkey_len;
                pk
            } else {
                offset += pubkey_len.min(data.len().saturating_sub(offset));
                String::new()
            };

            if data.len() < offset + 8 { return None; }
            let timestamp = u64::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);

            // Estimate current slot from timestamp (approx 400ms per slot)
            let slot = timestamp / 400;

            Some(LeaderHint {
                timestamp,
                slot,
                expires_at_slot: slot + slots_remaining as u64,
                preferred_region,
                backup_regions: vec![],
                confidence,
                leader_pubkey,
                metadata: crate::types::LeaderHintMetadata {
                    tpu_rtt_ms: 0,
                    region_score: confidence as f64 / 100.0,
                    leader_tpu_address: None,
                    region_rtt_ms: None,
                },
            })
        })
        .await
    }

    async fn subscribe_tip_instructions(&self) -> Result<mpsc::Receiver<TipInstruction>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        self.subscribe_stream(StreamType::TipInstructions, |data| {
            // Parse TipInstruction wire format:
            // - 1 byte: stream type (0x03)
            // - 1 byte: sender_id length
            // - N bytes: sender_id
            // - 1 byte: tip_wallet length
            // - N bytes: tip_wallet
            // - 8 bytes: tip_amount_lamports (u64 BE)
            // - 1 byte: tier length
            // - N bytes: tier
            // - 4 bytes: expected_latency_ms (u32 BE)
            // - 4 bytes: confidence (u32 BE)
            // - 8 bytes: valid_until_slot (u64 BE)
            // - 8 bytes: timestamp (u64 BE)

            if data.len() < 2 || data[0] != StreamType::TipInstructions as u8 {
                return None;
            }

            let mut offset = 1;

            // Sender ID
            let sender_len = data[offset] as usize;
            offset += 1;
            if data.len() < offset + sender_len {
                return None;
            }
            let sender = String::from_utf8_lossy(&data[offset..offset + sender_len]).to_string();
            offset += sender_len;

            // Tip wallet
            if data.len() < offset + 1 {
                return None;
            }
            let wallet_len = data[offset] as usize;
            offset += 1;
            if data.len() < offset + wallet_len {
                return None;
            }
            let tip_wallet_address = String::from_utf8_lossy(&data[offset..offset + wallet_len]).to_string();
            offset += wallet_len;

            // Tip amount (in lamports, convert to SOL)
            if data.len() < offset + 8 {
                return None;
            }
            let tip_amount_lamports = u64::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);
            let tip_amount_sol = tip_amount_lamports as f64 / 1_000_000_000.0;
            offset += 8;

            // Tier
            if data.len() < offset + 1 {
                return None;
            }
            let tier_len = data[offset] as usize;
            offset += 1;
            if data.len() < offset + tier_len {
                return None;
            }
            let tip_tier = String::from_utf8_lossy(&data[offset..offset + tier_len]).to_string();
            offset += tier_len;

            // Expected latency
            if data.len() < offset + 4 {
                return None;
            }
            let expected_latency_ms = u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            offset += 4;

            // Confidence (0-100)
            if data.len() < offset + 4 {
                return None;
            }
            let confidence = u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            offset += 4;

            // Valid until slot
            if data.len() < offset + 8 {
                return None;
            }
            let valid_until_slot = u64::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);
            offset += 8;

            // Timestamp
            if data.len() < offset + 8 {
                return None;
            }
            let timestamp = u64::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);

            Some(TipInstruction {
                timestamp,
                sender: sender.clone(),
                sender_name: sender,
                tip_wallet_address,
                tip_amount_sol,
                tip_tier,
                expected_latency_ms,
                confidence,
                valid_until_slot,
                alternative_senders: vec![],
            })
        })
        .await
    }

    async fn subscribe_priority_fees(&self) -> Result<mpsc::Receiver<PriorityFee>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        self.subscribe_stream(StreamType::PriorityFees, |data| {
            // Parse PriorityFee wire format:
            // - 1 byte: stream type (0x04)
            // - 8 bytes: micro_lamports_per_cu (u64 BE)
            // - 1 byte: percentile
            // - 4 bytes: sample_count (u32 BE)
            // - 8 bytes: timestamp (u64 BE)

            if data.len() < 22 || data[0] != StreamType::PriorityFees as u8 {
                return None;
            }

            let compute_unit_price = u64::from_be_bytes([
                data[1], data[2], data[3], data[4], data[5], data[6], data[7], data[8],
            ]);

            let percentile = data[9];

            let sample_count = u32::from_be_bytes([data[10], data[11], data[12], data[13]]);

            let timestamp = u64::from_be_bytes([
                data[14], data[15], data[16], data[17], data[18], data[19], data[20], data[21],
            ]);

            // Map percentile to speed tier
            let speed = match percentile {
                0..=50 => "low",
                51..=75 => "medium",
                _ => "high",
            }
            .to_string();

            // Default compute unit limit (200k is typical)
            let compute_unit_limit = 200_000u32;

            // Estimate cost: (price * limit) / 1_000_000 (micro-lamports to lamports) / 1e9 (to SOL)
            let estimated_cost_sol = (compute_unit_price * compute_unit_limit as u64) as f64 / 1e15;

            // Map percentile to landing probability
            let landing_probability = match percentile {
                0..=25 => 50,
                26..=50 => 70,
                51..=75 => 85,
                76..=90 => 95,
                _ => 99,
            };

            Some(PriorityFee {
                timestamp,
                speed,
                compute_unit_price,
                compute_unit_limit,
                estimated_cost_sol,
                landing_probability,
                network_congestion: if compute_unit_price > 100_000 { "high" } else if compute_unit_price > 10_000 { "medium" } else { "low" }.to_string(),
                recent_success_rate: sample_count as f64 / 100.0, // Approximate
            })
        })
        .await
    }

    async fn subscribe_latest_blockhash(&self) -> Result<mpsc::Receiver<LatestBlockhash>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        self.subscribe_stream(StreamType::LatestBlockhash, |data| {
            // Parse LatestBlockhash wire format:
            // - 1 byte: stream type (0x06)
            // - 1 byte: blockhash length
            // - N bytes: blockhash (base58)
            // - 8 bytes: last_valid_block_height (u64 BE)
            // - 8 bytes: timestamp (u64 BE)

            if data.len() < 2 || data[0] != StreamType::LatestBlockhash as u8 {
                return None;
            }

            let mut offset = 1;
            let hash_len = data[offset] as usize;
            offset += 1;

            if data.len() < offset + hash_len + 16 {
                return None;
            }

            let blockhash = String::from_utf8_lossy(&data[offset..offset + hash_len]).to_string();
            offset += hash_len;

            let last_valid_block_height = u64::from_be_bytes([
                data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
                data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
            ]);
            offset += 8;

            let timestamp = u64::from_be_bytes([
                data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
                data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
            ]);

            Some(LatestBlockhash {
                blockhash,
                last_valid_block_height,
                timestamp,
            })
        })
        .await
    }

    async fn subscribe_latest_slot(&self) -> Result<mpsc::Receiver<LatestSlot>> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        self.subscribe_stream(StreamType::LatestSlot, |data| {
            // Parse LatestSlot wire format:
            // - 1 byte: stream type (0x07)
            // - 8 bytes: slot (u64 BE)
            // - 8 bytes: timestamp (u64 BE)

            if data.len() < 17 || data[0] != StreamType::LatestSlot as u8 {
                return None;
            }

            let slot = u64::from_be_bytes([
                data[1], data[2], data[3], data[4],
                data[5], data[6], data[7], data[8],
            ]);

            let timestamp = u64::from_be_bytes([
                data[9], data[10], data[11], data[12],
                data[13], data[14], data[15], data[16],
            ]);

            Some(LatestSlot {
                slot,
                timestamp,
            })
        })
        .await
    }

    async fn ping(&self) -> Result<PingResult> {
        if !self.is_connected() {
            return Err(SdkError::NotConnected);
        }

        let connection = {
            let guard = self.connection.read().await;
            guard
                .as_ref()
                .ok_or(SdkError::NotConnected)?
                .clone()
        };

        let seq = self.ping_seq.fetch_add(1, Ordering::Relaxed);
        let client_send_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Open bidi stream and send ping frame
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| SdkError::connection(format!("Failed to open ping stream: {}", e)))?;

        let mut frame = [0u8; 13];
        frame[0] = StreamType::Ping as u8;
        frame[1..5].copy_from_slice(&seq.to_be_bytes());
        frame[5..13].copy_from_slice(&client_send_time.to_be_bytes());

        send.write_all(&frame)
            .await
            .map_err(|e| SdkError::connection(format!("Failed to send ping: {}", e)))?;
        let _ = send.finish();

        // Read pong response (21 bytes)
        let mut response = [0u8; 21];
        recv.read_exact(&mut response)
            .await
            .map_err(|e| SdkError::connection(format!("Failed to read pong: {}", e)))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Parse response
        let _resp_type = response[0]; // 0x08
        let _resp_seq = u32::from_be_bytes([response[1], response[2], response[3], response[4]]);
        let _resp_client_time = u64::from_be_bytes(response[5..13].try_into().unwrap());
        let server_time = u64::from_be_bytes(response[13..21].try_into().unwrap());

        let rtt_ms = now.saturating_sub(client_send_time);
        let clock_offset_ms = server_time as i64 - (client_send_time as i64 + rtt_ms as i64 / 2);

        Ok(PingResult {
            seq,
            rtt_ms,
            clock_offset_ms,
            server_time,
        })
    }
}

/// Custom certificate verifier that skips verification (for development)
/// In production, use proper certificate validation
#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> std::result::Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quic_transport_new() {
        let transport = QuicTransport::new();
        assert!(!transport.is_connected());
        assert_eq!(transport.protocol(), Protocol::Quic);
    }

    #[test]
    fn test_parse_endpoint() {
        // Test with quic:// prefix
        let result = QuicTransport::parse_endpoint("quic://127.0.0.1:4433");
        assert!(result.is_ok());
        let (addr, host) = result.unwrap();
        assert_eq!(addr.port(), 4433);
        assert_eq!(host, "127.0.0.1");

        // Test without prefix
        let result = QuicTransport::parse_endpoint("127.0.0.1:4433");
        assert!(result.is_ok());

        // Test invalid format
        let result = QuicTransport::parse_endpoint("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_request_id_generation() {
        let transport = QuicTransport::new();
        let id1 = transport.next_request_id();
        let id2 = transport.next_request_id();
        assert_eq!(id1, 0);
        assert_eq!(id2, 1);
    }

    #[test]
    fn test_stream_type_values() {
        assert_eq!(StreamType::TransactionSubmit as u8, 0x01);
        assert_eq!(StreamType::LeaderHints as u8, 0x02);
        assert_eq!(StreamType::TipInstructions as u8, 0x03);
        assert_eq!(StreamType::PriorityFees as u8, 0x04);
        assert_eq!(StreamType::Metrics as u8, 0x05);
        assert_eq!(StreamType::LatestBlockhash as u8, 0x06);
        assert_eq!(StreamType::LatestSlot as u8, 0x07);
    }

    #[test]
    fn test_response_status_parsing() {
        assert_eq!(ResponseStatus::from_byte(0x01), Some(ResponseStatus::Accepted));
        assert_eq!(ResponseStatus::from_byte(0x02), Some(ResponseStatus::Duplicate));
        assert_eq!(ResponseStatus::from_byte(0x03), Some(ResponseStatus::RateLimited));
        assert_eq!(ResponseStatus::from_byte(0x04), Some(ResponseStatus::ServerError));
        assert_eq!(ResponseStatus::from_byte(0x00), None);
        assert_eq!(ResponseStatus::from_byte(0x05), None);
    }
}
