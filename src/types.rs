//! Message types for the Slipstream SDK
//!
//! These types mirror the server-side types for stream messages and transaction results.

use serde::{Deserialize, Serialize};

/// Leader region hint for routing decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaderHint {
    /// Current timestamp (unix millis)
    pub timestamp: u64,
    /// Current slot
    pub slot: u64,
    /// Slot when this hint expires
    pub expires_at_slot: u64,
    /// Preferred region ID
    pub preferred_region: String,
    /// Backup region IDs
    pub backup_regions: Vec<String>,
    /// Confidence score (0-100)
    pub confidence: u32,
    /// Leader validator pubkey (optional)
    pub leader_pubkey: Option<String>,
    /// Additional metadata
    pub metadata: LeaderHintMetadata,
}

/// Leader hint metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaderHintMetadata {
    /// TPU round-trip time in milliseconds (from preferred region)
    pub tpu_rtt_ms: u32,
    /// Region score
    pub region_score: f64,
    /// Leader validator's TPU address (ip:port)
    #[serde(default)]
    pub leader_tpu_address: Option<String>,
    /// Per-region RTT to the leader (region_id -> rtt_ms)
    #[serde(default)]
    pub region_rtt_ms: Option<std::collections::HashMap<String, u32>>,
}

/// Tip instruction for transaction building
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TipInstruction {
    /// Timestamp (unix millis)
    pub timestamp: u64,
    /// Sender ID
    pub sender: String,
    /// Human-readable sender name
    pub sender_name: String,
    /// Tip wallet address (base58)
    pub tip_wallet_address: String,
    /// Tip amount in SOL
    pub tip_amount_sol: f64,
    /// Tip tier name
    pub tip_tier: String,
    /// Expected latency in milliseconds
    pub expected_latency_ms: u32,
    /// Confidence score (0-100)
    pub confidence: u32,
    /// Slot until which this tip is valid
    pub valid_until_slot: u64,
    /// Alternative sender options
    #[serde(default)]
    pub alternative_senders: Vec<AlternativeSender>,
}

/// Alternative sender option
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlternativeSender {
    /// Sender ID
    pub sender: String,
    /// Tip amount in SOL
    pub tip_amount_sol: f64,
    /// Confidence score
    pub confidence: u32,
}

/// Priority fee recommendation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriorityFee {
    /// Timestamp (unix millis)
    pub timestamp: u64,
    /// Fee speed tier (low, medium, high)
    pub speed: String,
    /// Compute unit price in micro-lamports
    pub compute_unit_price: u64,
    /// Compute unit limit
    pub compute_unit_limit: u32,
    /// Estimated cost in SOL
    pub estimated_cost_sol: f64,
    /// Estimated landing probability (0-100)
    pub landing_probability: u32,
    /// Network congestion level
    pub network_congestion: String,
    /// Recent success rate
    pub recent_success_rate: f64,
}

/// Transaction submission result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionResult {
    /// Request ID from submission
    pub request_id: String,
    /// Internal transaction ID
    pub transaction_id: String,
    /// Transaction signature (base58)
    pub signature: Option<String>,
    /// Current status
    pub status: TransactionStatus,
    /// Slot where transaction landed (if confirmed)
    pub slot: Option<u64>,
    /// Timestamp
    pub timestamp: u64,
    /// Routing details (if available)
    pub routing: Option<RoutingInfo>,
    /// Error information (if failed)
    pub error: Option<TransactionError>,
}

/// Transaction status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionStatus {
    /// Transaction accepted for processing
    Pending,
    /// Transaction being processed
    Processing,
    /// Transaction sent to network
    Sent,
    /// Transaction confirmed on-chain
    Confirmed,
    /// Transaction failed
    Failed,
    /// Duplicate transaction detected
    Duplicate,
    /// Rate limited
    RateLimited,
    /// Insufficient token balance
    InsufficientTokens,
}

impl TransactionStatus {
    /// Check if this is a terminal status
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TransactionStatus::Confirmed
                | TransactionStatus::Failed
                | TransactionStatus::Duplicate
                | TransactionStatus::RateLimited
                | TransactionStatus::InsufficientTokens
        )
    }

    /// Check if this is a success status
    pub fn is_success(&self) -> bool {
        matches!(self, TransactionStatus::Confirmed)
    }
}

/// Routing information for a transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingInfo {
    /// Region used
    pub region: String,
    /// Sender used
    pub sender: String,
    /// Time spent in routing (ms)
    pub routing_latency_ms: u32,
    /// Time spent in sender (ms)
    pub sender_latency_ms: u32,
    /// Total latency (ms)
    pub total_latency_ms: u32,
}

/// Transaction error details
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionError {
    /// Error code
    pub code: String,
    /// Error message
    pub message: String,
    /// Additional details
    pub details: Option<serde_json::Value>,
}

/// Transaction submission options
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitOptions {
    /// Broadcast to all regions (fan-out)
    #[serde(default)]
    pub broadcast_mode: bool,
    /// Preferred sender ID
    #[serde(default)]
    pub preferred_sender: Option<String>,
    /// Maximum retries
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Timeout in milliseconds
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// Deduplication ID (optional)
    #[serde(default)]
    pub dedup_id: Option<String>,
}

impl Default for SubmitOptions {
    fn default() -> Self {
        Self {
            broadcast_mode: false,
            preferred_sender: None,
            max_retries: default_max_retries(),
            timeout_ms: default_timeout_ms(),
            dedup_id: None,
        }
    }
}

fn default_max_retries() -> u32 {
    2
}

fn default_timeout_ms() -> u64 {
    30_000
}

/// Connection information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    /// Session ID
    pub session_id: String,
    /// Connected protocol
    pub protocol: String,
    /// Connected region
    pub region: Option<String>,
    /// Server time at connection
    pub server_time: u64,
    /// Available features
    pub features: Vec<String>,
    /// Rate limit information
    pub rate_limit: RateLimitInfo,
}

/// Rate limit information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitInfo {
    /// Requests per second
    pub rps: u32,
    /// Burst size
    pub burst: u32,
}

/// Available protocols
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// QUIC - primary high-performance protocol
    Quic,
    /// gRPC - fallback protocol
    Grpc,
    /// WebSocket - streaming fallback
    WebSocket,
    /// HTTP - polling fallback
    Http,
}

impl Protocol {
    /// Get all protocols in fallback order
    pub fn fallback_order() -> &'static [Protocol] {
        &[Protocol::Quic, Protocol::Grpc, Protocol::WebSocket, Protocol::Http]
    }
}

/// Worker endpoint configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerEndpoint {
    /// Unique identifier for this worker
    pub id: String,
    /// Region identifier (e.g., "us-east", "eu-central")
    pub region: String,
    /// QUIC endpoint (e.g., "quic://worker1.us-east.slipstream.allenhark.com:4433")
    pub quic: Option<String>,
    /// gRPC endpoint (e.g., "http://worker1.us-east.slipstream.allenhark.com:10000")
    pub grpc: Option<String>,
    /// WebSocket endpoint (e.g., "wss://worker1.us-east.slipstream.allenhark.com/ws")
    pub websocket: Option<String>,
    /// HTTP endpoint (e.g., "https://worker1.us-east.slipstream.allenhark.com")
    pub http: Option<String>,
}

impl WorkerEndpoint {
    /// Create a new worker endpoint with all protocols at the same IP/host
    /// Uses standard ports: QUIC=4433, gRPC=10000, WebSocket=9000, HTTP=9000
    pub fn new(id: &str, region: &str, ip: &str) -> Self {
        Self {
            id: id.to_string(),
            region: region.to_string(),
            quic: Some(format!("{}:4433", ip)),
            grpc: Some(format!("http://{}:10000", ip)),
            websocket: Some(format!("ws://{}:9000/ws", ip)),
            http: Some(format!("http://{}:9000", ip)),
        }
    }

    /// Create a worker endpoint with custom ports
    pub fn with_ports(
        id: &str,
        region: &str,
        ip: &str,
        quic_port: u16,
        grpc_port: u16,
        ws_port: u16,
        http_port: u16,
    ) -> Self {
        Self {
            id: id.to_string(),
            region: region.to_string(),
            quic: Some(format!("{}:{}", ip, quic_port)),
            grpc: Some(format!("http://{}:{}", ip, grpc_port)),
            websocket: Some(format!("ws://{}:{}/ws", ip, ws_port)),
            http: Some(format!("http://{}:{}", ip, http_port)),
        }
    }

    /// Create a worker endpoint with explicit endpoints
    pub fn with_endpoints(
        id: &str,
        region: &str,
        quic: Option<String>,
        grpc: Option<String>,
        websocket: Option<String>,
        http: Option<String>,
    ) -> Self {
        Self {
            id: id.to_string(),
            region: region.to_string(),
            quic,
            grpc,
            websocket,
            http,
        }
    }

    /// Get endpoint for a specific protocol
    pub fn get_endpoint(&self, protocol: Protocol) -> Option<&str> {
        match protocol {
            Protocol::Quic => self.quic.as_deref(),
            Protocol::Grpc => self.grpc.as_deref(),
            Protocol::WebSocket => self.websocket.as_deref(),
            Protocol::Http => self.http.as_deref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_leader_hint_deserialize() {
        let json = r#"{
            "timestamp": 1706011200000,
            "slot": 12345678,
            "expiresAtSlot": 12345682,
            "preferredRegion": "us-west",
            "backupRegions": ["eu-central"],
            "confidence": 87,
            "leaderPubkey": null,
            "metadata": {
                "tpuRttMs": 12,
                "regionScore": 0.85
            }
        }"#;

        let hint: LeaderHint = serde_json::from_str(json).unwrap();
        assert_eq!(hint.preferred_region, "us-west");
        assert_eq!(hint.confidence, 87);
        assert_eq!(hint.metadata.tpu_rtt_ms, 12);
        assert!(hint.metadata.leader_tpu_address.is_none());
        assert!(hint.metadata.region_rtt_ms.is_none());
    }

    #[test]
    fn test_leader_hint_with_extended_metadata() {
        let json = r#"{
            "timestamp": 1706011200000,
            "slot": 12345678,
            "expiresAtSlot": 12345682,
            "preferredRegion": "us-west",
            "backupRegions": ["eu-central", "asia-east"],
            "confidence": 92,
            "leaderPubkey": "Vote111111111111111111111111111111111111111",
            "metadata": {
                "tpuRttMs": 8,
                "regionScore": 0.92,
                "leaderTpuAddress": "192.168.1.100:8004",
                "regionRttMs": {"us-west": 8, "eu-central": 45, "asia-east": 120}
            }
        }"#;

        let hint: LeaderHint = serde_json::from_str(json).unwrap();
        assert_eq!(hint.preferred_region, "us-west");
        assert_eq!(hint.confidence, 92);
        assert_eq!(hint.leader_pubkey, Some("Vote111111111111111111111111111111111111111".to_string()));
        assert_eq!(hint.metadata.leader_tpu_address, Some("192.168.1.100:8004".to_string()));
        let region_rtt = hint.metadata.region_rtt_ms.unwrap();
        assert_eq!(region_rtt.get("us-west"), Some(&8));
        assert_eq!(region_rtt.get("eu-central"), Some(&45));
    }

    #[test]
    fn test_tip_instruction_deserialize() {
        let json = r#"{
            "timestamp": 1706011200000,
            "sender": "0slot",
            "senderName": "0Slot",
            "tipWalletAddress": "So11111111111111111111111111111111111111112",
            "tipAmountSol": 0.0001,
            "tipTier": "standard",
            "expectedLatencyMs": 100,
            "confidence": 95,
            "validUntilSlot": 12345700,
            "alternativeSenders": []
        }"#;

        let tip: TipInstruction = serde_json::from_str(json).unwrap();
        assert_eq!(tip.sender, "0slot");
        assert_eq!(tip.tip_amount_sol, 0.0001);
    }

    #[test]
    fn test_transaction_status() {
        assert!(TransactionStatus::Confirmed.is_terminal());
        assert!(TransactionStatus::Failed.is_terminal());
        assert!(!TransactionStatus::Processing.is_terminal());

        assert!(TransactionStatus::Confirmed.is_success());
        assert!(!TransactionStatus::Failed.is_success());
    }

    #[test]
    fn test_submit_options_default() {
        let options = SubmitOptions::default();
        assert!(!options.broadcast_mode);
        assert_eq!(options.max_retries, 2);
        assert_eq!(options.timeout_ms, 30_000);
    }

    #[test]
    fn test_routing_recommendation_deserialize() {
        let json = r#"{
            "bestRegion": "us-west",
            "leaderPubkey": "Vote111111111111111111111111111111111111111",
            "slot": 12345678,
            "confidence": 85,
            "expectedRttMs": 12,
            "fallbackRegions": ["eu-central", "asia-east"],
            "fallbackStrategy": "sequential",
            "validForMs": 400
        }"#;

        let rec: RoutingRecommendation = serde_json::from_str(json).unwrap();
        assert_eq!(rec.best_region, "us-west");
        assert_eq!(rec.confidence, 85);
        assert_eq!(rec.fallback_strategy, FallbackStrategy::Sequential);
        assert_eq!(rec.fallback_regions.len(), 2);
    }

    #[test]
    fn test_multi_region_config_default() {
        let config = MultiRegionConfig::default();
        assert!(config.auto_follow_leader);
        assert_eq!(config.min_switch_confidence, 60);
        assert_eq!(config.switch_cooldown_ms, 500);
        assert!(!config.broadcast_high_priority);
        assert_eq!(config.max_broadcast_regions, 3);
    }

    #[test]
    fn test_fallback_strategy_default() {
        let strategy = FallbackStrategy::default();
        assert_eq!(strategy, FallbackStrategy::Sequential);
    }
}

// ============================================================================
// Multi-Region Routing Types
// ============================================================================

/// Routing recommendation for leader-aware transaction submission
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingRecommendation {
    /// Best region for current leader
    pub best_region: String,
    /// Current leader validator pubkey
    pub leader_pubkey: Option<String>,
    /// Current slot
    pub slot: u64,
    /// Confidence in recommendation (0-100)
    pub confidence: u32,
    /// Expected RTT to leader TPU from best region (ms)
    pub expected_rtt_ms: Option<u32>,
    /// Fallback regions in priority order
    pub fallback_regions: Vec<String>,
    /// Fallback strategy recommendation
    pub fallback_strategy: FallbackStrategy,
    /// Time until this recommendation expires (ms)
    pub valid_for_ms: u64,
}

/// Strategy for handling fallback when primary region fails
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackStrategy {
    /// Use next region in fallback list
    Sequential,
    /// Broadcast to all regions simultaneously
    Broadcast,
    /// Retry same region with exponential backoff
    Retry,
    /// No fallback - fail immediately
    None,
}

impl Default for FallbackStrategy {
    fn default() -> Self {
        FallbackStrategy::Sequential
    }
}

/// Configuration for multi-region client behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiRegionConfig {
    /// Whether to automatically follow leader hints
    #[serde(default = "default_auto_follow")]
    pub auto_follow_leader: bool,
    /// Minimum confidence to switch regions (0-100)
    #[serde(default = "default_min_confidence")]
    pub min_switch_confidence: u32,
    /// Cooldown between region switches (ms)
    #[serde(default = "default_switch_cooldown")]
    pub switch_cooldown_ms: u64,
    /// Whether to use broadcast mode for high-priority transactions
    #[serde(default)]
    pub broadcast_high_priority: bool,
    /// Maximum regions to use in broadcast mode
    #[serde(default = "default_max_broadcast_regions")]
    pub max_broadcast_regions: usize,
}

fn default_auto_follow() -> bool {
    true
}

fn default_min_confidence() -> u32 {
    60
}

fn default_switch_cooldown() -> u64 {
    500
}

fn default_max_broadcast_regions() -> usize {
    3
}

impl Default for MultiRegionConfig {
    fn default() -> Self {
        Self {
            auto_follow_leader: default_auto_follow(),
            min_switch_confidence: default_min_confidence(),
            switch_cooldown_ms: default_switch_cooldown(),
            broadcast_high_priority: false,
            max_broadcast_regions: default_max_broadcast_regions(),
        }
    }
}

/// Region status for multi-region routing decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionStatus {
    /// Region identifier
    pub region_id: String,
    /// Whether region is currently available
    pub available: bool,
    /// Current latency to region (ms)
    pub latency_ms: Option<u32>,
    /// Estimated RTT to current leader from this region (ms)
    pub leader_rtt_ms: Option<u32>,
    /// Region score (0.0 - 1.0)
    pub score: Option<f64>,
    /// Number of available workers in region
    pub worker_count: u32,
}

// ============================================================================
// Token Billing Types
// ============================================================================

/// Token balance information for the authenticated API key
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Balance {
    /// Balance in SOL
    pub balance_sol: f64,
    /// Balance in tokens (1 token = 1 query)
    pub balance_tokens: i64,
    /// Balance in lamports
    pub balance_lamports: i64,
    /// Grace period remaining in tokens (negative = in grace period)
    pub grace_remaining_tokens: i64,
}

/// Deposit address for topping up token balance
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TopUpInfo {
    /// Solana deposit wallet address (base58)
    pub deposit_wallet: String,
    /// Minimum top-up amount in SOL
    pub min_amount_sol: f64,
    /// Minimum top-up amount in lamports
    pub min_amount_lamports: u64,
}

/// A single usage/billing ledger entry
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UsageEntry {
    /// Entry timestamp (unix millis)
    pub timestamp: u64,
    /// Transaction type (e.g. "usage_debit", "admin_credit", "deposit")
    pub tx_type: String,
    /// Amount in lamports (positive for credits, negative for debits)
    pub amount_lamports: i64,
    /// Balance after this transaction in lamports
    pub balance_after_lamports: i64,
    /// Human-readable description
    pub description: Option<String>,
}

/// Options for querying usage history
#[derive(Debug, Clone, Default)]
pub struct UsageHistoryOptions {
    /// Maximum number of entries to return (default: 50, max: 100)
    pub limit: Option<u32>,
    /// Offset for pagination
    pub offset: Option<u32>,
}

/// A single deposit history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepositEntry {
    /// On-chain transaction signature
    pub signature: String,
    /// Deposit amount in lamports
    pub amount_lamports: i64,
    /// Deposit amount in SOL
    pub amount_sol: f64,
    /// USD value at time of deposit
    pub usd_value: Option<f64>,
    /// SOL/USD price at time of deposit
    pub sol_usd_price: Option<f64>,
    /// Whether tokens have been credited for this deposit
    pub credited: bool,
    /// When tokens were credited (if credited)
    pub credited_at: Option<String>,
    /// Solana slot of the deposit
    pub slot: i64,
    /// When the deposit was detected
    pub detected_at: String,
    /// On-chain block timestamp
    pub block_time: Option<String>,
}

/// Options for querying deposit history
#[derive(Debug, Clone, Default)]
pub struct DepositHistoryOptions {
    /// Maximum number of entries to return (default: 50, max: 100)
    pub limit: Option<u32>,
    /// Offset for pagination
    pub offset: Option<u32>,
}

/// Free tier daily usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreeTierUsage {
    /// Number of transactions used today
    pub used: u32,
    /// Remaining transactions today
    pub remaining: u32,
    /// Daily transaction limit
    pub limit: u32,
    /// When the counter resets (UTC midnight, RFC3339)
    pub resets_at: String,
}

/// Pending (uncredited) deposit summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingDeposit {
    /// Total pending lamports
    pub pending_lamports: i64,
    /// Total pending SOL
    pub pending_sol: f64,
    /// Number of uncredited deposits
    pub pending_count: i64,
    /// Minimum deposit in USD to trigger crediting
    pub minimum_deposit_usd: f64,
}

// ============================================================================
// Additional Types for Architecture Compliance
// ============================================================================

/// Priority fee configuration (Architecture: Section 9)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriorityFeeConfig {
    /// Whether priority fee optimization is enabled
    pub enabled: bool,
    /// Speed tier: slow, fast, ultra_fast
    pub speed: PriorityFeeSpeed,
    /// Maximum tip in SOL (optional cap)
    pub max_tip: Option<f64>,
}

impl Default for PriorityFeeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            speed: PriorityFeeSpeed::Fast,
            max_tip: None,
        }
    }
}

/// Priority fee speed tier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriorityFeeSpeed {
    /// Lower fees, slower landing
    Slow,
    /// Balanced fees and speed
    Fast,
    /// Highest fees, fastest landing
    UltraFast,
}

/// Connection status for state tracking (Architecture: Section 9)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    /// Current connection state
    pub state: ConnectionState,
    /// Active protocol
    pub protocol: Protocol,
    /// Current latency in milliseconds
    pub latency_ms: u64,
    /// Connected region
    pub region: Option<String>,
}

/// Connection state machine states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// Not yet connected
    Disconnected,
    /// Attempting to connect
    Connecting,
    /// Successfully connected
    Connected,
    /// Connection error occurred
    Error,
}

/// Retry backoff strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackoffStrategy {
    /// Linear backoff (delay * attempt)
    Linear,
    /// Exponential backoff (delay * 2^attempt)
    Exponential,
}

impl Default for BackoffStrategy {
    fn default() -> Self {
        BackoffStrategy::Exponential
    }
}

/// Performance metrics for SDK operations (Architecture: Section 9)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceMetrics {
    /// Total transactions submitted
    pub transactions_submitted: u64,
    /// Total transactions confirmed
    pub transactions_confirmed: u64,
    /// Average submission latency in ms
    pub average_latency_ms: f64,
    /// Success rate (0.0 - 1.0)
    pub success_rate: f64,
}
