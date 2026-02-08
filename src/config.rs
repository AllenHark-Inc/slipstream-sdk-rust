//! Configuration types for the Slipstream SDK

use crate::error::{Result, SdkError};
pub use crate::types::{BackoffStrategy, PriorityFeeConfig, Protocol, WorkerEndpoint};
use std::time::Duration;

/// Default connection timeout (10 seconds)
const DEFAULT_CONNECTION_TIMEOUT_MS: u64 = 10_000;

/// Default max retries for transaction submission
const DEFAULT_MAX_RETRIES: u32 = 3;

/// Protocol timeout defaults
const QUIC_TIMEOUT_MS: u64 = 2_000;
const GRPC_TIMEOUT_MS: u64 = 3_000;
const WEBSOCKET_TIMEOUT_MS: u64 = 3_000;
const HTTP_TIMEOUT_MS: u64 = 5_000;

/// SDK Configuration
#[derive(Debug, Clone)]
pub struct Config {
    /// API key for authentication
    pub api_key: String,

    /// Target region (optional, auto-detect if not set)
    pub region: Option<String>,

    /// Custom endpoint URL (optional, auto-discover if not set)
    pub endpoint: Option<String>,

    /// Discovery service URL (defaults to https://discovery.slipstream.allenhark.com)
    pub discovery_url: String,

    /// Connection timeout
    pub connection_timeout: Duration,

    /// Maximum retries for transaction submission
    pub max_retries: u32,

    /// Enable leader hints streaming
    pub leader_hints: bool,

    /// Enable tip instruction streaming
    pub stream_tip_instructions: bool,

    /// Enable priority fee streaming
    pub stream_priority_fees: bool,

    /// Protocol-specific timeouts
    pub protocol_timeouts: ProtocolTimeouts,

    /// Preferred protocol (optional, auto-select if not set)
    pub preferred_protocol: Option<Protocol>,

    /// Selected worker endpoint (override default discovery)
    pub selected_worker: Option<WorkerEndpoint>,

    /// Priority fee configuration
    pub priority_fee: PriorityFeeConfig,

    /// Retry backoff strategy
    pub retry_backoff: BackoffStrategy,

    /// Minimum confidence threshold for leader hints (0-100)
    pub min_confidence: u32,

    /// Idle timeout for connection (None = no timeout)
    pub idle_timeout: Option<Duration>,

    /// Billing tier (free, standard, pro, enterprise). Default: "pro"
    pub tier: String,
}

/// Protocol timeout configuration
#[derive(Debug, Clone)]
pub struct ProtocolTimeouts {
    pub quic: Duration,
    pub grpc: Duration,
    pub websocket: Duration,
    pub http: Duration,
}

impl Default for ProtocolTimeouts {
    fn default() -> Self {
        Self {
            quic: Duration::from_millis(QUIC_TIMEOUT_MS),
            grpc: Duration::from_millis(GRPC_TIMEOUT_MS),
            websocket: Duration::from_millis(WEBSOCKET_TIMEOUT_MS),
            http: Duration::from_millis(HTTP_TIMEOUT_MS),
        }
    }
}



impl Config {
    /// Create a new configuration builder
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        if self.api_key.is_empty() {
            return Err(SdkError::config("api_key is required"));
        }
        if !self.api_key.starts_with("sk_") {
            return Err(SdkError::config("api_key must start with 'sk_'"));
        }
        Ok(())
    }

    /// Get the endpoint URL with protocol
    pub fn get_endpoint(&self, protocol: Protocol) -> String {
        // 1. Check if we have a resolved worker endpoint
        if let Some(ref worker) = self.selected_worker {
            if let Some(endpoint) = worker.get_endpoint(protocol) {
                return endpoint.to_string();
            }
        }

        // 2. Check for explicit endpoint override
        if let Some(ref endpoint) = self.endpoint {
            return endpoint.clone();
        }

        // 3. Default endpoints based on protocol
        match protocol {
            Protocol::Quic => "quic://localhost:4433".to_string(),
            Protocol::Grpc => "http://localhost:10000".to_string(),
            Protocol::WebSocket => "ws://localhost:9000/ws".to_string(),
            Protocol::Http => "http://localhost:9000".to_string(),
        }
    }
}

/// Configuration builder for ergonomic config creation
#[derive(Debug, Default)]
pub struct ConfigBuilder {
    api_key: Option<String>,
    region: Option<String>,
    endpoint: Option<String>,
    discovery_url: Option<String>,
    connection_timeout: Option<Duration>,
    max_retries: Option<u32>,
    leader_hints: Option<bool>,
    stream_tip_instructions: Option<bool>,
    stream_priority_fees: Option<bool>,
    protocol_timeouts: Option<ProtocolTimeouts>,
    preferred_protocol: Option<Protocol>,
    selected_worker: Option<WorkerEndpoint>,
    priority_fee: Option<PriorityFeeConfig>,
    retry_backoff: Option<BackoffStrategy>,
    min_confidence: Option<u32>,
    idle_timeout: Option<Duration>,
    tier: Option<String>,
}

impl ConfigBuilder {
    /// Set the API key (required)
    pub fn api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Set the target region
    pub fn region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Set a custom endpoint URL (overrides discovery)
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Set a custom discovery URL (default: https://discovery.slipstream.allenhark.com)
    pub fn discovery_url(mut self, url: impl Into<String>) -> Self {
        self.discovery_url = Some(url.into());
        self
    }

    /// Set connection timeout
    pub fn connection_timeout(mut self, timeout: Duration) -> Self {
        self.connection_timeout = Some(timeout);
        self
    }

    /// Set max retries for transaction submission
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.max_retries = Some(retries);
        self
    }

    /// Enable/disable leader hints streaming
    pub fn leader_hints(mut self, enabled: bool) -> Self {
        self.leader_hints = Some(enabled);
        self
    }

    /// Enable/disable tip instruction streaming
    pub fn stream_tip_instructions(mut self, enabled: bool) -> Self {
        self.stream_tip_instructions = Some(enabled);
        self
    }

    /// Enable/disable priority fee streaming
    pub fn stream_priority_fees(mut self, enabled: bool) -> Self {
        self.stream_priority_fees = Some(enabled);
        self
    }

    /// Set custom protocol timeouts
    pub fn protocol_timeouts(mut self, timeouts: ProtocolTimeouts) -> Self {
        self.protocol_timeouts = Some(timeouts);
        self
    }

    /// Set preferred protocol (skip fallback chain)
    pub fn preferred_protocol(mut self, protocol: Protocol) -> Self {
        self.preferred_protocol = Some(protocol);
        self
    }

    /// Set priority fee configuration
    pub fn priority_fee(mut self, config: PriorityFeeConfig) -> Self {
        self.priority_fee = Some(config);
        self
    }

    /// Set retry backoff strategy
    pub fn retry_backoff(mut self, strategy: BackoffStrategy) -> Self {
        self.retry_backoff = Some(strategy);
        self
    }

    /// Set minimum confidence threshold for leader hints
    pub fn min_confidence(mut self, confidence: u32) -> Self {
        self.min_confidence = Some(confidence);
        self
    }

    /// Set idle timeout for connection
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = Some(timeout);
        self
    }

    /// Set billing tier (free, standard, pro, enterprise)
    pub fn tier(mut self, tier: impl Into<String>) -> Self {
        self.tier = Some(tier.into());
        self
    }

    /// Build the configuration
    pub fn build(self) -> Result<Config> {
        let config = Config {
            api_key: self.api_key.ok_or_else(|| SdkError::config("api_key is required"))?,
            region: self.region,
            endpoint: self.endpoint,
            discovery_url: self.discovery_url.unwrap_or_else(|| {
                crate::discovery::DEFAULT_DISCOVERY_URL.to_string()
            }),
            connection_timeout: self
                .connection_timeout
                .unwrap_or_else(|| Duration::from_millis(DEFAULT_CONNECTION_TIMEOUT_MS)),
            max_retries: self.max_retries.unwrap_or(DEFAULT_MAX_RETRIES),
            leader_hints: self.leader_hints.unwrap_or(true),
            stream_tip_instructions: self.stream_tip_instructions.unwrap_or(false),
            stream_priority_fees: self.stream_priority_fees.unwrap_or(false),
            protocol_timeouts: self.protocol_timeouts.unwrap_or_default(),
            preferred_protocol: self.preferred_protocol,
            selected_worker: self.selected_worker,
            priority_fee: self.priority_fee.unwrap_or_default(),
            retry_backoff: self.retry_backoff.unwrap_or_default(),
            min_confidence: self.min_confidence.unwrap_or(70),
            idle_timeout: self.idle_timeout,
            tier: self.tier.unwrap_or_else(|| "pro".to_string()),
        };

        config.validate()?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let config = Config::builder()
            .api_key("sk_test_12345678")
            .region("us-west")
            .build()
            .unwrap();

        assert_eq!(config.api_key, "sk_test_12345678");
        assert_eq!(config.region, Some("us-west".to_string()));
        assert!(config.leader_hints);
        assert!(!config.stream_tip_instructions);
    }

    #[test]
    fn test_config_builder_missing_api_key() {
        let result = Config::builder().build();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("api_key"));
    }

    #[test]
    fn test_config_builder_invalid_api_key() {
        let result = Config::builder().api_key("invalid_key").build();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("sk_"));
    }

    #[test]
    fn test_protocol_fallback_order() {
        let order = Protocol::fallback_order();
        assert_eq!(order[0], Protocol::Quic);
        assert_eq!(order[1], Protocol::Grpc);
        assert_eq!(order[2], Protocol::WebSocket);
        assert_eq!(order[3], Protocol::Http);
    }

    #[test]
    fn test_config_get_endpoint() {
        let config = Config::builder()
            .api_key("sk_test_12345678")
            .build()
            .unwrap();

        assert!(config.get_endpoint(Protocol::Quic).contains("4433"));
        assert!(config.get_endpoint(Protocol::Grpc).contains("10000"));
        assert!(config.get_endpoint(Protocol::WebSocket).contains("ws://"));
        assert!(config.get_endpoint(Protocol::Http).contains("http://"));
    }

    #[test]
    fn test_config_custom_endpoint() {
        let config = Config::builder()
            .api_key("sk_test_12345678")
            .endpoint("https://custom.example.com")
            .build()
            .unwrap();

        assert_eq!(
            config.get_endpoint(Protocol::Http),
            "https://custom.example.com"
        );
    }
}
