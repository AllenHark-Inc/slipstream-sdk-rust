use allenhark_slipstream::{
    Config, SubmitOptions, TransactionBuilder, TransactionStatus,
};
use std::time::Duration;

// =============================================================================
// Config Builder Tests
// =============================================================================

#[tokio::test]
async fn test_client_config_validation() {
    let config = Config::builder().api_key("sk_test_123").build();

    assert!(config.is_ok());

    let bad_config = Config::builder().api_key("invalid_prefix").build();

    assert!(bad_config.is_err());
}

#[tokio::test]
async fn test_client_connect_failure_no_server() {
    // Attempt to connect to a random port that definitely doesn't have a server
    let config = Config::builder()
        .api_key("sk_test_123")
        .endpoint("http://localhost:54321")
        .build()
        .unwrap();

    let result = allenhark_slipstream::SlipstreamClient::connect(config).await;

    // Should fail with connection refused or timeout
    assert!(result.is_err());
}

#[test]
fn test_config_builder_with_region() {
    let config = Config::builder()
        .api_key("sk_test_abc")
        .region("us-west")
        .build()
        .unwrap();

    assert_eq!(config.region, Some("us-west".to_string()));
    assert_eq!(config.api_key, "sk_test_abc");
}

#[test]
fn test_config_builder_with_all_options() {
    let config = Config::builder()
        .api_key("sk_test_full")
        .region("eu-central")
        .endpoint("http://localhost:9000")
        .connection_timeout(Duration::from_secs(20))
        .max_retries(5)
        .leader_hints(true)
        .stream_tip_instructions(true)
        .stream_priority_fees(true)
        .keepalive(false)
        .tier("enterprise")
        .build()
        .unwrap();

    assert_eq!(config.region, Some("eu-central".to_string()));
    assert_eq!(config.endpoint, Some("http://localhost:9000".to_string()));
    assert_eq!(config.connection_timeout, Duration::from_secs(20));
    assert_eq!(config.max_retries, 5);
    assert!(config.leader_hints);
    assert!(config.stream_tip_instructions);
    assert!(config.stream_priority_fees);
    assert!(!config.keepalive);
    assert_eq!(config.tier, "enterprise");
}

#[test]
fn test_config_builder_empty_api_key_fails() {
    let result = Config::builder().api_key("").build();
    assert!(result.is_err());
}

#[test]
fn test_config_builder_protocol_timeouts() {
    let config = Config::builder().api_key("sk_test_123").build().unwrap();

    assert_eq!(config.protocol_timeouts.quic, Duration::from_millis(2000));
    assert_eq!(config.protocol_timeouts.grpc, Duration::from_millis(3000));
    assert_eq!(
        config.protocol_timeouts.websocket,
        Duration::from_millis(3000)
    );
    assert_eq!(config.protocol_timeouts.http, Duration::from_millis(5000));
}

// =============================================================================
// SubmitOptions Tests
// =============================================================================

#[test]
fn test_submit_options_default() {
    let options = SubmitOptions::default();
    assert!(!options.broadcast_mode);
    assert_eq!(options.max_retries, 2);
    assert_eq!(options.timeout_ms, 30_000);
    assert!(options.dedup_id.is_none());
    assert!(options.preferred_sender.is_none());
    assert!(options.retry.is_none());
}

#[test]
fn test_submit_options_serialize_roundtrip() {
    let options = SubmitOptions {
        broadcast_mode: true,
        preferred_sender: Some("jito".to_string()),
        max_retries: 5,
        timeout_ms: 60_000,
        dedup_id: Some("unique-123".to_string()),
        retry: None,
    };

    let json = serde_json::to_string(&options).unwrap();
    let deserialized: SubmitOptions = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.broadcast_mode, true);
    assert_eq!(deserialized.preferred_sender, Some("jito".to_string()));
    assert_eq!(deserialized.max_retries, 5);
    assert_eq!(deserialized.timeout_ms, 60_000);
    assert_eq!(deserialized.dedup_id, Some("unique-123".to_string()));
}

// =============================================================================
// TransactionBuilder Tests
// =============================================================================

#[test]
fn test_transaction_builder_default() {
    let builder = TransactionBuilder::new();
    assert!(builder.tip_info().is_none());
    assert!(builder.fee_info().is_none());
}

#[test]
fn test_transaction_builder_tip() {
    let builder = TransactionBuilder::new()
        .tip_wallet("7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU")
        .tip_lamports(50_000);

    let (wallet, amount) = builder.tip_info().unwrap();
    assert_eq!(wallet, "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU");
    assert_eq!(amount, 50_000);
}

#[test]
fn test_transaction_builder_tip_sol() {
    let builder = TransactionBuilder::new()
        .tip_wallet("TestWallet")
        .tip_sol(0.001);

    let (_, amount) = builder.tip_info().unwrap();
    assert_eq!(amount, 1_000_000); // 0.001 SOL = 1,000,000 lamports
}

#[test]
fn test_transaction_builder_from_tip_instruction() {
    use allenhark_slipstream::TipInstruction;

    let tip = TipInstruction {
        timestamp: 1000,
        sender: "jito".to_string(),
        sender_name: "Jito".to_string(),
        tip_wallet_address: "JitoWallet123".to_string(),
        tip_amount_sol: 0.0001,
        tip_tier: "standard".to_string(),
        expected_latency_ms: 50,
        confidence: 90,
        valid_until_slot: 12345,
        alternative_senders: vec![],
    };

    let builder = TransactionBuilder::new().from_tip_instruction(&tip);

    let (wallet, amount) = builder.tip_info().unwrap();
    assert_eq!(wallet, "JitoWallet123");
    assert_eq!(amount, 100_000); // 0.0001 SOL

    let options = builder.build_options();
    assert_eq!(options.preferred_sender, Some("jito".to_string()));
}

#[test]
fn test_transaction_builder_priority_fee() {
    let builder = TransactionBuilder::new()
        .compute_unit_price(10_000)
        .compute_unit_limit(200_000);

    let (price, limit) = builder.fee_info().unwrap();
    assert_eq!(price, 10_000);
    assert_eq!(limit, 200_000);
}

#[test]
fn test_transaction_builder_fee_default_limit() {
    let builder = TransactionBuilder::new().compute_unit_price(5_000);

    let (price, limit) = builder.fee_info().unwrap();
    assert_eq!(price, 5_000);
    assert_eq!(limit, 200_000); // default limit
}

#[test]
fn test_transaction_builder_from_priority_fee() {
    use allenhark_slipstream::PriorityFee;

    let fee = PriorityFee {
        timestamp: 1000,
        speed: "fast".to_string(),
        compute_unit_price: 25_000,
        compute_unit_limit: 300_000,
        estimated_cost_sol: 0.0001,
        landing_probability: 95,
        network_congestion: "low".to_string(),
        recent_success_rate: 0.99,
    };

    let builder = TransactionBuilder::new().from_priority_fee(&fee);

    let (price, limit) = builder.fee_info().unwrap();
    assert_eq!(price, 25_000);
    assert_eq!(limit, 300_000);
}

#[test]
fn test_transaction_builder_build_options() {
    let builder = TransactionBuilder::new()
        .broadcast(true)
        .preferred_sender("0slot")
        .dedup_id("dedup-abc")
        .max_retries(5)
        .timeout_ms(60_000);

    let options = builder.build_options();
    assert!(options.broadcast_mode);
    assert_eq!(options.preferred_sender, Some("0slot".to_string()));
    assert_eq!(options.dedup_id, Some("dedup-abc".to_string()));
    assert_eq!(options.max_retries, 5);
    assert_eq!(options.timeout_ms, 60_000);
}

#[test]
fn test_transaction_builder_chaining() {
    let builder = TransactionBuilder::new()
        .tip_wallet("Wallet1")
        .tip_lamports(100_000)
        .compute_unit_price(50_000)
        .compute_unit_limit(400_000)
        .broadcast(true)
        .max_retries(3);

    let (wallet, tip) = builder.tip_info().unwrap();
    assert_eq!(wallet, "Wallet1");
    assert_eq!(tip, 100_000);

    let (price, limit) = builder.fee_info().unwrap();
    assert_eq!(price, 50_000);
    assert_eq!(limit, 400_000);

    let options = builder.build_options();
    assert!(options.broadcast_mode);
    assert_eq!(options.max_retries, 3);
}

// =============================================================================
// TransactionStatus Tests
// =============================================================================

#[test]
fn test_transaction_status_terminal() {
    assert!(TransactionStatus::Confirmed.is_terminal());
    assert!(TransactionStatus::Failed.is_terminal());
    assert!(TransactionStatus::Duplicate.is_terminal());
    assert!(TransactionStatus::RateLimited.is_terminal());
    assert!(TransactionStatus::InsufficientTokens.is_terminal());

    assert!(!TransactionStatus::Pending.is_terminal());
    assert!(!TransactionStatus::Processing.is_terminal());
    assert!(!TransactionStatus::Sent.is_terminal());
}

#[test]
fn test_transaction_status_success() {
    assert!(TransactionStatus::Confirmed.is_success());
    assert!(!TransactionStatus::Failed.is_success());
    assert!(!TransactionStatus::Pending.is_success());
    assert!(!TransactionStatus::Sent.is_success());
}

// =============================================================================
// Stream Type Deserialization Tests
// =============================================================================

#[test]
fn test_leader_hint_deserialize() {
    use allenhark_slipstream::LeaderHint;

    let json = r#"{
        "timestamp": 1706011200000,
        "slot": 12345678,
        "expiresAtSlot": 12345682,
        "preferredRegion": "us-west",
        "backupRegions": ["eu-central", "ap-northeast"],
        "confidence": 87,
        "leaderPubkey": "Vote111111111111111111111111111111111111111",
        "metadata": {
            "tpuRttMs": 12,
            "regionScore": 0.85
        }
    }"#;

    let hint: LeaderHint = serde_json::from_str(json).unwrap();
    assert_eq!(hint.preferred_region, "us-west");
    assert_eq!(hint.confidence, 87);
    assert_eq!(hint.backup_regions.len(), 2);
    assert_eq!(hint.metadata.tpu_rtt_ms, 12);
}

#[test]
fn test_priority_fee_deserialize() {
    use allenhark_slipstream::PriorityFee;

    let json = r#"{
        "timestamp": 1706011200000,
        "speed": "fast",
        "computeUnitPrice": 50000,
        "computeUnitLimit": 200000,
        "estimatedCostSol": 0.00001,
        "landingProbability": 95,
        "networkCongestion": "low",
        "recentSuccessRate": 0.99
    }"#;

    let fee: PriorityFee = serde_json::from_str(json).unwrap();
    assert_eq!(fee.speed, "fast");
    assert_eq!(fee.compute_unit_price, 50000);
    assert_eq!(fee.landing_probability, 95);
}

#[test]
fn test_latest_blockhash_deserialize() {
    use allenhark_slipstream::LatestBlockhash;

    let json = r#"{
        "blockhash": "7Xq3JcEBR1sVmAHGgn3Dz3C96DRfz7RgXWbvJqLbMp3",
        "lastValidBlockHeight": 12345700,
        "timestamp": 1706011200000
    }"#;

    let bh: LatestBlockhash = serde_json::from_str(json).unwrap();
    assert!(!bh.blockhash.is_empty());
    assert_eq!(bh.last_valid_block_height, 12345700);
}

#[test]
fn test_latest_slot_deserialize() {
    use allenhark_slipstream::LatestSlot;

    let json = r#"{
        "slot": 12345678,
        "timestamp": 1706011200000
    }"#;

    let slot: LatestSlot = serde_json::from_str(json).unwrap();
    assert_eq!(slot.slot, 12345678);
}

// =============================================================================
// Transaction Result Tests
// =============================================================================

#[test]
fn test_transaction_result_deserialize() {
    use allenhark_slipstream::TransactionResult;

    let json = r#"{
        "requestId": "req-001",
        "transactionId": "tx-001",
        "signature": "5KrDECTGk8SyD1Y5DBbFw3NqVxKY7W4gVjHZBhGjY7xz",
        "status": "confirmed",
        "slot": 12345678,
        "timestamp": 1706011200000,
        "routing": {
            "region": "us-west",
            "sender": "jito",
            "routingLatencyMs": 2,
            "senderLatencyMs": 8,
            "totalLatencyMs": 10
        },
        "error": null
    }"#;

    let result: TransactionResult = serde_json::from_str(json).unwrap();
    assert_eq!(result.request_id, "req-001");
    assert_eq!(result.status, TransactionStatus::Confirmed);
    assert!(result.signature.is_some());
    assert_eq!(result.slot, Some(12345678));

    let routing = result.routing.unwrap();
    assert_eq!(routing.region, "us-west");
    assert_eq!(routing.sender, "jito");
    assert_eq!(routing.total_latency_ms, 10);
}

#[test]
fn test_transaction_result_failed() {
    use allenhark_slipstream::TransactionResult;

    let json = r#"{
        "requestId": "req-002",
        "transactionId": "tx-002",
        "signature": null,
        "status": "failed",
        "slot": null,
        "timestamp": 1706011200000,
        "routing": null,
        "error": {
            "code": "SENDER_ERROR",
            "message": "Sender returned error",
            "details": null
        }
    }"#;

    let result: TransactionResult = serde_json::from_str(json).unwrap();
    assert_eq!(result.status, TransactionStatus::Failed);
    assert!(result.signature.is_none());
    assert!(result.error.is_some());
    assert_eq!(result.error.unwrap().code, "SENDER_ERROR");
}

// =============================================================================
// Error Type Tests
// =============================================================================

#[test]
fn test_error_types() {
    use allenhark_slipstream::SdkError;

    let err = SdkError::config("missing api_key");
    assert!(err.to_string().contains("missing api_key"));

    let err = SdkError::connection("refused");
    assert!(err.to_string().contains("refused"));

    let err = SdkError::auth("invalid key");
    assert!(err.to_string().contains("invalid key"));

    let err = SdkError::transaction("failed");
    assert!(err.to_string().contains("failed"));

    let err = SdkError::Timeout(Duration::from_secs(5));
    assert!(err.to_string().contains("5s"));
}

// =============================================================================
// Webhook Types Tests
// =============================================================================

#[test]
fn test_webhook_event_strings() {
    use allenhark_slipstream::WebhookEvent;

    assert_eq!(WebhookEvent::TransactionSent.as_str(), "transaction.sent");
    assert_eq!(
        WebhookEvent::TransactionConfirmed.as_str(),
        "transaction.confirmed"
    );
    assert_eq!(
        WebhookEvent::BillingLowBalance.as_str(),
        "billing.low_balance"
    );
}

// =============================================================================
// Protocol Tests
// =============================================================================

#[test]
fn test_protocol_fallback_order() {
    use allenhark_slipstream::Protocol;

    let order = Protocol::fallback_order();
    assert_eq!(order.len(), 4);
    assert_eq!(order[0], Protocol::Quic);
    assert_eq!(order[1], Protocol::Grpc);
    assert_eq!(order[2], Protocol::WebSocket);
    assert_eq!(order[3], Protocol::Http);
}

// =============================================================================
// Bundle Result Tests
// =============================================================================

#[test]
fn test_bundle_result_deserialize() {
    use allenhark_slipstream::BundleResult;

    let json = r#"{
        "bundleId": "bundle-001",
        "accepted": true,
        "signatures": ["sig1", "sig2", "sig3"],
        "senderId": "jito",
        "error": null
    }"#;

    let result: BundleResult = serde_json::from_str(json).unwrap();
    assert_eq!(result.bundle_id, "bundle-001");
    assert!(result.accepted);
    assert_eq!(result.signatures.len(), 3);
    assert_eq!(result.sender_id, Some("jito".to_string()));
}

// =============================================================================
// Connection Timeout Tests
// =============================================================================

#[tokio::test]
async fn test_connect_timeout_respected() {
    let start = std::time::Instant::now();

    let config = Config::builder()
        .api_key("sk_test_123")
        .endpoint("http://192.0.2.1:9999") // non-routable address
        .connection_timeout(Duration::from_secs(2))
        .preferred_protocol(allenhark_slipstream::Protocol::Http)
        .build()
        .unwrap();

    let result = allenhark_slipstream::SlipstreamClient::connect(config).await;
    let elapsed = start.elapsed();

    assert!(result.is_err());
    // Should timeout within HTTP timeout (5s) not connection_timeout because
    // protocol timeout takes precedence. Just verify it didn't hang.
    assert!(elapsed < Duration::from_secs(15));
}

// =============================================================================
// Fallback Chain Tests
// =============================================================================

#[test]
fn test_fallback_chain_timeouts() {
    use allenhark_slipstream::{Protocol, ProtocolTimeouts};

    let timeouts = ProtocolTimeouts::default();
    assert_eq!(timeouts.quic, Duration::from_millis(2000));
    assert_eq!(timeouts.grpc, Duration::from_millis(3000));
    assert_eq!(timeouts.websocket, Duration::from_millis(3000));
    assert_eq!(timeouts.http, Duration::from_millis(5000));
}

// =============================================================================
// Multi-Region Config Tests
// =============================================================================

#[test]
fn test_multi_region_config_default() {
    use allenhark_slipstream::MultiRegionConfig;

    let config = MultiRegionConfig::default();
    assert!(config.auto_follow_leader);
    assert_eq!(config.min_switch_confidence, 60);
    assert_eq!(config.switch_cooldown_ms, 500);
    assert!(!config.broadcast_high_priority);
    assert_eq!(config.max_broadcast_regions, 3);
}

// =============================================================================
// Landing Rate Tests
// =============================================================================

#[test]
fn test_landing_rate_stats_deserialize() {
    use allenhark_slipstream::LandingRateStats;

    let json = r#"{
        "period": { "start": "2026-01-01T00:00:00Z", "end": "2026-01-02T00:00:00Z" },
        "total_sent": 1000,
        "total_landed": 950,
        "landing_rate": 0.95,
        "by_sender": [
            { "sender": "jito", "total_sent": 500, "total_landed": 490, "landing_rate": 0.98 }
        ],
        "by_region": [
            { "region": "us-west", "total_sent": 600, "total_landed": 580, "landing_rate": 0.967 }
        ]
    }"#;

    let stats: LandingRateStats = serde_json::from_str(json).unwrap();
    assert_eq!(stats.total_sent, 1000);
    assert_eq!(stats.total_landed, 950);
    assert!((stats.landing_rate - 0.95).abs() < 0.001);
    assert_eq!(stats.by_sender.len(), 1);
    assert_eq!(stats.by_region.len(), 1);
}

// =============================================================================
// RPC Response Tests
// =============================================================================

#[test]
fn test_rpc_response_success() {
    use allenhark_slipstream::RpcResponse;

    let json = r#"{
        "jsonrpc": "2.0",
        "id": 1,
        "result": { "blockhash": "abc123", "lastValidBlockHeight": 12345 }
    }"#;

    let resp: RpcResponse = serde_json::from_str(json).unwrap();
    assert!(resp.result.is_some());
    assert!(resp.error.is_none());
}

#[test]
fn test_rpc_response_error() {
    use allenhark_slipstream::RpcResponse;

    let json = r#"{
        "jsonrpc": "2.0",
        "id": 1,
        "error": { "code": -32601, "message": "Method not found" }
    }"#;

    let resp: RpcResponse = serde_json::from_str(json).unwrap();
    assert!(resp.result.is_none());
    let err = resp.error.unwrap();
    assert_eq!(err.code, -32601);
    assert_eq!(err.message, "Method not found");
}
