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
    /// TPU round-trip time in milliseconds
    pub tpu_rtt_ms: u32,
    /// Region score
    pub region_score: f64,
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
}
