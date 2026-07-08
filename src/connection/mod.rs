//! Connection management and protocol implementations
//!
//! This module provides the transport abstraction and protocol-specific implementations.

pub mod grpc;
pub mod http;
pub mod health;
pub mod quic;
pub mod selector;
pub mod websocket;

use crate::config::{Config, ProtocolTimeouts};
use crate::error::{Result, SdkError};
use crate::types::{
    ConnectionInfo, LatestBlockhash, LatestSlot, LeaderHint, PingResult, PriorityFee, Protocol,
    SubmitOptions, TipInstruction, TransactionResult,
};
use async_trait::async_trait;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Re-export async_trait for Transport implementors
pub use async_trait::async_trait as transport_trait;

/// Transport abstraction for different protocol backends
#[async_trait]
pub trait Transport: Send + Sync {
    /// Connect to the server
    async fn connect(&mut self, config: &Config) -> Result<ConnectionInfo>;

    /// Disconnect from the server
    async fn disconnect(&mut self) -> Result<()>;

    /// Check if connected
    fn is_connected(&self) -> bool;

    /// Get the protocol type
    fn protocol(&self) -> Protocol;

    /// Submit a transaction
    async fn submit_transaction(
        &self,
        transaction: &[u8],
        options: &SubmitOptions,
    ) -> Result<TransactionResult>;

    /// Subscribe to leader hints stream
    async fn subscribe_leader_hints(&self) -> Result<mpsc::Receiver<LeaderHint>>;

    /// Subscribe to tip instructions stream
    async fn subscribe_tip_instructions(&self) -> Result<mpsc::Receiver<TipInstruction>>;

    /// Subscribe to priority fees stream
    async fn subscribe_priority_fees(&self) -> Result<mpsc::Receiver<PriorityFee>>;

    /// Subscribe to latest blockhash stream
    async fn subscribe_latest_blockhash(&self) -> Result<mpsc::Receiver<LatestBlockhash>>;

    /// Subscribe to latest slot stream
    async fn subscribe_latest_slot(&self) -> Result<mpsc::Receiver<LatestSlot>>;

    /// Send a ping and measure RTT + clock offset
    async fn ping(&self) -> Result<PingResult> {
        Err(SdkError::connection("ping not supported on this transport"))
    }
}

/// Fallback chain for protocol selection
pub struct FallbackChain {
    timeouts: ProtocolTimeouts,
}

impl FallbackChain {
    /// Create a new fallback chain with custom timeouts
    pub fn new(timeouts: ProtocolTimeouts) -> Self {
        Self { timeouts }
    }

    /// Get the timeout for a protocol
    pub fn timeout_for(&self, protocol: Protocol) -> Duration {
        match protocol {
            Protocol::Quic => self.timeouts.quic,
            Protocol::Grpc => self.timeouts.grpc,
            Protocol::WebSocket => self.timeouts.websocket,
            Protocol::Http => self.timeouts.http,
        }
    }

    /// Attempt to connect using the fallback chain
    pub async fn connect(&self, config: &Config) -> Result<Box<dyn Transport>> {
        // Resolve worker if no explicit endpoint is set
        let mut resolved_config = config.clone();
        
        if config.endpoint.is_none() && config.selected_worker.is_none() {
             debug!("No endpoint configured, performing worker selection");
             // In a real scenario, these would be bootstrap nodes
             let selector = selector::WorkerSelectorBuilder::new()
                .add_worker_host("local-1", "us-east", "127.0.0.1")
                .build();
            
            match selector.select_best().await {
                Ok(worker) => {
                    info!(worker = %worker.id, "Selected best worker");
                    resolved_config.selected_worker = Some(worker.clone());
                }
                Err(e) => {
                    warn!(error = %e, "Worker selection failed, falling back to defaults");
                }
            }
        }
        
        let config = &resolved_config;

        // If a preferred protocol is set, only try that one
        if let Some(preferred) = config.preferred_protocol {
            return self.try_protocol(config, preferred).await;
        }

        // Try protocols in fallback order, skipping those without configured endpoints
        let mut last_error = None;
        for protocol in Protocol::fallback_order() {
            // Skip protocols that have no endpoint configured on the selected worker
            if let Some(ref worker) = config.selected_worker {
                if worker.get_endpoint(*protocol).is_none() {
                    debug!(protocol = ?protocol, "No endpoint configured, skipping");
                    continue;
                }
            }
            match self.try_protocol(config, *protocol).await {
                Ok(transport) => {
                    info!(protocol = ?protocol, "Connected successfully");
                    return Ok(transport);
                }
                Err(e) => {
                    warn!(protocol = ?protocol, error = %e, "Protocol failed, trying next");
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or(SdkError::AllProtocolsFailed))
    }

    /// Try to connect with a specific protocol.
    ///
    /// Dials the worker's PRIMARY endpoint and, only if that connect fails with
    /// a transport-establishment error while the worker advertises a legacy
    /// endpoint (port migration), retries ONCE on the legacy endpoint. Uses the
    /// real transports via [`DefaultConnector`].
    async fn try_protocol(&self, config: &Config, protocol: Protocol) -> Result<Box<dyn Transport>> {
        self.try_protocol_with(&DefaultConnector, config, protocol)
            .await
    }

    /// Prefer-primary / single-legacy-fallback connect, parameterized over a
    /// [`Connector`] so the fallback semantics can be unit-tested with a stub.
    ///
    /// Semantics:
    /// - PRIMARY first. On success, return immediately — the legacy endpoint is
    ///   never constructed or dialed.
    /// - Fall back to the legacy endpoint ONCE, and only when (a) the primary
    ///   connect failed with a *connect/transport* error (see
    ///   [`Self::is_connect_failure`]) AND (b) the worker advertises a legacy
    ///   endpoint for this protocol.
    /// - No legacy endpoint present ⇒ single attempt, error surfaced unchanged
    ///   (byte-for-byte today's behavior).
    async fn try_protocol_with(
        &self,
        connector: &dyn Connector,
        config: &Config,
        protocol: Protocol,
    ) -> Result<Box<dyn Transport>> {
        let timeout = self.timeout_for(protocol);
        debug!(protocol = ?protocol, timeout_ms = timeout.as_millis(), "Attempting protocol");

        // PRIMARY attempt.
        let primary_err = match Self::attempt(connector, config, protocol, timeout).await {
            Ok(transport) => return Ok(transport),
            Err(e) => e,
        };

        // Only a connect/transport failure is worth retrying on a different
        // port; application errors (auth rejected, bad config, protocol errors)
        // would fail identically on the legacy port, so they are surfaced as-is.
        if Self::is_connect_failure(&primary_err) {
            if let Some(legacy_config) = config.with_legacy_endpoint(protocol) {
                warn!(
                    protocol = ?protocol,
                    error = %primary_err,
                    "Primary endpoint connect failed, retrying legacy endpoint (port migration)"
                );
                return Self::attempt(connector, &legacy_config, protocol, timeout).await;
            }
        }

        Err(primary_err)
    }

    /// Single connect attempt against `config`'s endpoint for `protocol`,
    /// bounded by the protocol timeout.
    async fn attempt(
        connector: &dyn Connector,
        config: &Config,
        protocol: Protocol,
        timeout: Duration,
    ) -> Result<Box<dyn Transport>> {
        match tokio::time::timeout(timeout, connector.connect(config, protocol)).await {
            Ok(result) => result,
            Err(_) => Err(SdkError::Timeout(timeout)),
        }
    }

    /// Classify whether an error is a connect/transport-establishment failure
    /// (worth retrying on a different port) versus an application error.
    ///
    /// Connect failures — connection refused, transport errors, DNS failures
    /// ([`SdkError::Connection`]) and connect timeouts ([`SdkError::Timeout`]) —
    /// could plausibly succeed on the legacy port. Everything else (auth
    /// rejection, config/protocol errors, post-connect request failures) is an
    /// application error and must NOT trigger a legacy fallback.
    fn is_connect_failure(err: &SdkError) -> bool {
        matches!(err, SdkError::Connection(_) | SdkError::Timeout(_))
    }
}

/// Abstraction over establishing a single connection for a protocol, so the
/// prefer-primary / legacy-fallback logic can be exercised with a stub.
#[async_trait]
trait Connector: Send + Sync {
    /// Connect to `config`'s endpoint for `protocol`, returning a live transport.
    async fn connect(&self, config: &Config, protocol: Protocol) -> Result<Box<dyn Transport>>;
}

/// Production connector: instantiates and connects the real per-protocol transports.
struct DefaultConnector;

#[async_trait]
impl Connector for DefaultConnector {
    async fn connect(&self, config: &Config, protocol: Protocol) -> Result<Box<dyn Transport>> {
        match protocol {
            Protocol::Http => {
                let mut transport = http::HttpTransport::new();
                transport.connect(config).await?;
                Ok(Box::new(transport))
            }
            Protocol::WebSocket => {
                let mut transport = websocket::WebSocketTransport::new();
                transport.connect(config).await?;
                Ok(Box::new(transport))
            }
            Protocol::Grpc => {
                let mut transport = grpc::GrpcTransport::new();
                transport.connect(config).await?;
                Ok(Box::new(transport))
            }
            Protocol::Quic => {
                let mut transport = quic::QuicTransport::new();
                transport.connect(config).await?;
                Ok(Box::new(transport))
            }
        }
    }
}

impl Default for FallbackChain {
    fn default() -> Self {
        Self::new(ProtocolTimeouts::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WorkerEndpoint;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_fallback_chain_timeouts() {
        let chain = FallbackChain::default();
        assert_eq!(chain.timeout_for(Protocol::Quic), Duration::from_millis(2000));
        assert_eq!(chain.timeout_for(Protocol::Grpc), Duration::from_millis(3000));
        assert_eq!(chain.timeout_for(Protocol::WebSocket), Duration::from_millis(3000));
        assert_eq!(chain.timeout_for(Protocol::Http), Duration::from_millis(5000));
    }

    // ---- Legacy-port connect-fallback tests (Task 2) -----------------------

    const PRIMARY_QUIC: &str = "10.0.0.1:4435";
    const LEGACY_QUIC: &str = "10.0.0.1:4433";

    /// Per-endpoint outcome the stub connector should produce.
    #[derive(Clone, Copy)]
    enum Outcome {
        Ok,
        /// Transport-establishment failure → fallback-eligible.
        ConnectFail,
        /// Application error (auth rejected) → must NOT trigger fallback.
        AppError,
    }

    /// Minimal Transport stub. `try_protocol_with` only returns the box; none of
    /// these methods are ever invoked in these tests.
    struct StubTransport {
        protocol: Protocol,
    }

    #[async_trait]
    impl Transport for StubTransport {
        async fn connect(&mut self, _config: &Config) -> Result<ConnectionInfo> {
            unimplemented!("stub")
        }
        async fn disconnect(&mut self) -> Result<()> {
            Ok(())
        }
        fn is_connected(&self) -> bool {
            true
        }
        fn protocol(&self) -> Protocol {
            self.protocol
        }
        async fn submit_transaction(
            &self,
            _transaction: &[u8],
            _options: &SubmitOptions,
        ) -> Result<TransactionResult> {
            unimplemented!("stub")
        }
        async fn subscribe_leader_hints(&self) -> Result<mpsc::Receiver<LeaderHint>> {
            unimplemented!("stub")
        }
        async fn subscribe_tip_instructions(&self) -> Result<mpsc::Receiver<TipInstruction>> {
            unimplemented!("stub")
        }
        async fn subscribe_priority_fees(&self) -> Result<mpsc::Receiver<PriorityFee>> {
            unimplemented!("stub")
        }
        async fn subscribe_latest_blockhash(&self) -> Result<mpsc::Receiver<LatestBlockhash>> {
            unimplemented!("stub")
        }
        async fn subscribe_latest_slot(&self) -> Result<mpsc::Receiver<LatestSlot>> {
            unimplemented!("stub")
        }
    }

    /// Connector that records every dialed endpoint and returns a configured
    /// outcome depending on whether the primary or legacy endpoint was dialed.
    struct MockConnector {
        primary_endpoint: String,
        primary_outcome: Outcome,
        legacy_outcome: Outcome,
        dialed: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Connector for MockConnector {
        async fn connect(
            &self,
            config: &Config,
            protocol: Protocol,
        ) -> Result<Box<dyn Transport>> {
            let endpoint = config.get_endpoint(protocol);
            self.dialed.lock().unwrap().push(endpoint.clone());
            let outcome = if endpoint == self.primary_endpoint {
                self.primary_outcome
            } else {
                self.legacy_outcome
            };
            match outcome {
                Outcome::Ok => Ok(Box::new(StubTransport { protocol })),
                Outcome::ConnectFail => Err(SdkError::connection("connection refused")),
                Outcome::AppError => Err(SdkError::auth("api key rejected")),
            }
        }
    }

    /// Build a config whose selected worker exposes a primary QUIC endpoint and,
    /// optionally, a legacy QUIC endpoint.
    fn config_with_worker(primary: &str, legacy: Option<&str>) -> Config {
        let worker = WorkerEndpoint::with_endpoints(
            "worker-1",
            "us-east",
            Some(primary.to_string()),
            None,
            None,
            None,
        )
        .with_legacy(legacy.map(|s| s.to_string()), None, None);

        let mut config = Config::builder()
            .api_key("sk_test_12345678")
            .build()
            .unwrap();
        config.selected_worker = Some(worker);
        config
    }

    #[tokio::test]
    async fn primary_success_never_attempts_legacy() {
        let dialed = Arc::new(Mutex::new(Vec::new()));
        let connector = MockConnector {
            primary_endpoint: PRIMARY_QUIC.to_string(),
            primary_outcome: Outcome::Ok,
            legacy_outcome: Outcome::Ok,
            dialed: dialed.clone(),
        };
        let config = config_with_worker(PRIMARY_QUIC, Some(LEGACY_QUIC));
        let chain = FallbackChain::default();

        let transport = chain
            .try_protocol_with(&connector, &config, Protocol::Quic)
            .await
            .expect("primary should connect");

        assert_eq!(transport.protocol(), Protocol::Quic);
        // Only the primary endpoint was dialed — legacy never constructed/dialed.
        assert_eq!(*dialed.lock().unwrap(), vec![PRIMARY_QUIC.to_string()]);
    }

    #[tokio::test]
    async fn primary_connect_failure_falls_back_to_legacy() {
        let dialed = Arc::new(Mutex::new(Vec::new()));
        let connector = MockConnector {
            primary_endpoint: PRIMARY_QUIC.to_string(),
            primary_outcome: Outcome::ConnectFail,
            legacy_outcome: Outcome::Ok,
            dialed: dialed.clone(),
        };
        let config = config_with_worker(PRIMARY_QUIC, Some(LEGACY_QUIC));
        let chain = FallbackChain::default();

        let transport = chain
            .try_protocol_with(&connector, &config, Protocol::Quic)
            .await
            .expect("legacy fallback should connect");

        assert_eq!(transport.protocol(), Protocol::Quic);
        // Primary tried first, then the legacy endpoint — exactly one fallback.
        assert_eq!(
            *dialed.lock().unwrap(),
            vec![PRIMARY_QUIC.to_string(), LEGACY_QUIC.to_string()]
        );
    }

    #[tokio::test]
    async fn primary_connect_failure_without_legacy_surfaces_error() {
        let dialed = Arc::new(Mutex::new(Vec::new()));
        let connector = MockConnector {
            primary_endpoint: PRIMARY_QUIC.to_string(),
            primary_outcome: Outcome::ConnectFail,
            legacy_outcome: Outcome::Ok,
            dialed: dialed.clone(),
        };
        // No legacy endpoint → today's single-attempt behavior.
        let config = config_with_worker(PRIMARY_QUIC, None);
        let chain = FallbackChain::default();

        let result = chain
            .try_protocol_with(&connector, &config, Protocol::Quic)
            .await;

        assert!(matches!(result, Err(SdkError::Connection(_))));
        // Exactly one attempt — no fallback.
        assert_eq!(*dialed.lock().unwrap(), vec![PRIMARY_QUIC.to_string()]);
    }

    #[tokio::test]
    async fn primary_app_error_does_not_fall_back() {
        let dialed = Arc::new(Mutex::new(Vec::new()));
        let connector = MockConnector {
            primary_endpoint: PRIMARY_QUIC.to_string(),
            primary_outcome: Outcome::AppError,
            legacy_outcome: Outcome::Ok,
            dialed: dialed.clone(),
        };
        // Legacy IS present, but an application (auth) error must not use it.
        let config = config_with_worker(PRIMARY_QUIC, Some(LEGACY_QUIC));
        let chain = FallbackChain::default();

        let result = chain
            .try_protocol_with(&connector, &config, Protocol::Quic)
            .await;

        assert!(matches!(result, Err(SdkError::Auth(_))));
        // Legacy never dialed despite being available.
        assert_eq!(*dialed.lock().unwrap(), vec![PRIMARY_QUIC.to_string()]);
    }

    #[test]
    fn is_connect_failure_classification() {
        assert!(FallbackChain::is_connect_failure(&SdkError::connection("x")));
        assert!(FallbackChain::is_connect_failure(&SdkError::Timeout(
            Duration::from_millis(1)
        )));
        assert!(!FallbackChain::is_connect_failure(&SdkError::auth("x")));
        assert!(!FallbackChain::is_connect_failure(&SdkError::config("x")));
        assert!(!FallbackChain::is_connect_failure(&SdkError::protocol("x")));
    }

    #[test]
    fn with_legacy_endpoint_none_when_absent() {
        // No legacy endpoint ⇒ None ⇒ no fallback is ever attempted.
        let config = config_with_worker(PRIMARY_QUIC, None);
        assert!(config.with_legacy_endpoint(Protocol::Quic).is_none());
    }

    #[test]
    fn with_legacy_endpoint_swaps_primary_for_legacy() {
        let config = config_with_worker(PRIMARY_QUIC, Some(LEGACY_QUIC));
        let legacy_config = config
            .with_legacy_endpoint(Protocol::Quic)
            .expect("legacy variant");
        assert_eq!(legacy_config.get_endpoint(Protocol::Quic), LEGACY_QUIC);
        // Original config is untouched.
        assert_eq!(config.get_endpoint(Protocol::Quic), PRIMARY_QUIC);
    }
}
