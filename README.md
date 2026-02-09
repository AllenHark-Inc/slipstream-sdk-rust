[![allenhark.com](https://allenhark.com/allenhark-logo.png)](https://allenhark.com)

# Slipstream Rust SDK

The official Rust client for **AllenHark Slipstream**, the high-performance Solana transaction relay and intelligence network.

[![Crates.io](https://img.shields.io/crates/v/allenhark-slipstream.svg)](https://crates.io/crates/allenhark-slipstream)
[![Documentation](https://docs.rs/allenhark-slipstream/badge.svg)](https://docs.rs/allenhark-slipstream)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

## Features

- **Discovery-based connection** -- auto-discovers workers, no manual endpoint configuration
- **Leader-proximity routing** -- real-time leader hints route transactions to the lowest-latency region
- **Multi-region support** -- connect to workers across regions, auto-route based on leader schedule
- **Multi-protocol** -- QUIC (primary), gRPC, WebSocket, HTTP with automatic fallback
- **6 real-time streams** -- leader hints, tip instructions, priority fees, latest blockhash, latest slot, transaction updates
- **Stream billing** -- each stream costs 1 token; 1-hour reconnect grace period
- **Billing tiers** -- Free (100 tx/day), Standard, Pro, Enterprise with tier-specific rate limits
- **Token billing** -- check balance, deposit SOL, view usage and deposit history
- **Keep-alive & time sync** -- background ping with RTT measurement and NTP-style clock synchronization
- **Deduplication** -- prevent duplicate submissions with custom dedup IDs

## Installation

```toml
[dependencies]
allenhark-slipstream = "0.1"
tokio = { version = "1.0", features = ["full"] }
```

## Quick Start

```rust
use allenhark_slipstream::{Config, SlipstreamClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect with just an API key -- discovery handles the rest
    let config = Config::builder()
        .api_key("sk_live_12345678")
        .build()?;

    let client = SlipstreamClient::connect(config).await?;

    // Submit a signed transaction
    let result = client.submit_transaction(&tx_bytes).await?;
    println!("TX: {} ({})", result.transaction_id, result.status);

    // Check balance
    let balance = client.get_balance().await?;
    println!("Balance: {} tokens", balance.balance_tokens);

    Ok(())
}
```

---

## Configuration

### ConfigBuilder Reference

Use `Config::builder()` for fluent configuration. Only `api_key` is required.

```rust
use allenhark_slipstream::{Config, PriorityFeeConfig, PriorityFeeSpeed, BackoffStrategy};
use std::time::Duration;

let config = Config::builder()
    .api_key("sk_live_12345678")
    .region("us-east")
    .tier("pro")
    .min_confidence(80)
    .build()?;
```

| Method | Type | Default | Description |
|--------|------|---------|-------------|
| `api_key(key)` | `&str` | **required** | API key (must start with `sk_`) |
| `region(region)` | `&str` | auto | Preferred region (e.g., `"us-east"`, `"eu-central"`) |
| `endpoint(url)` | `&str` | auto | Override discovery with explicit worker endpoint |
| `discovery_url(url)` | `&str` | `https://discovery.slipstream.allenhark.com` | Custom discovery service URL |
| `tier(tier)` | `&str` | `"pro"` | Billing tier: `"free"`, `"standard"`, `"pro"`, `"enterprise"` |
| `connection_timeout(dur)` | `Duration` | 10s | Connection timeout |
| `max_retries(n)` | `u32` | `3` | Maximum retry attempts for failed requests |
| `leader_hints(bool)` | `bool` | `true` | Auto-subscribe to leader hint stream on connect |
| `stream_tip_instructions(bool)` | `bool` | `false` | Auto-subscribe to tip instruction stream on connect |
| `stream_priority_fees(bool)` | `bool` | `false` | Auto-subscribe to priority fee stream on connect |
| `stream_latest_blockhash(bool)` | `bool` | `false` | Auto-subscribe to latest blockhash stream on connect |
| `stream_latest_slot(bool)` | `bool` | `false` | Auto-subscribe to latest slot stream on connect |
| `protocol_timeouts(timeouts)` | `ProtocolTimeouts` | QUIC=2s, gRPC=3s, WS=3s, HTTP=5s | Per-protocol connection timeouts |
| `preferred_protocol(proto)` | `Protocol` | auto | Force a specific protocol (skip fallback chain) |
| `priority_fee(config)` | `PriorityFeeConfig` | disabled | Priority fee optimization settings (see below) |
| `retry_backoff(strategy)` | `BackoffStrategy` | `Exponential` | Retry backoff: `Linear` or `Exponential` |
| `min_confidence(n)` | `u32` | `70` | Minimum confidence (0-100) for leader hint routing |
| `keepalive(bool)` | `bool` | `true` | Enable background keep-alive ping loop |
| `keepalive_interval(secs)` | `u64` | `5` | Keep-alive ping interval in seconds |
| `idle_timeout(dur)` | `Duration` | none | Disconnect after idle period |

### Billing Tiers

Each API key has a billing tier that determines transaction cost, rate limits, and priority queuing weight. Set the tier to match your API key's assigned tier:

```rust
let config = Config::builder()
    .api_key("sk_live_12345678")
    .tier("pro")
    .build()?;
```

| Tier | Cost per TX | Cost per Stream | Rate Limit | Burst | Priority Slots | Daily Limit |
|------|------------|-----------------|------------|-------|----------------|-------------|
| **Free** | 0 (counter) | 0 (counter) | 5 rps | 10 | 5 concurrent | 100 tx/day |
| **Standard** | 50,000 lamports (0.00005 SOL) | 50,000 lamports | 5 rps | 10 | 10 concurrent | Unlimited |
| **Pro** | 100,000 lamports (0.0001 SOL) | 50,000 lamports | 20 rps | 50 | 50 concurrent | Unlimited |
| **Enterprise** | 1,000,000 lamports (0.001 SOL) | 50,000 lamports | 100 rps | 200 | 200 concurrent | Unlimited |

- **Free tier**: Uses a daily counter instead of token billing. Transactions and stream subscriptions both decrement the counter. Resets at UTC midnight.
- **Standard/Pro/Enterprise**: Deducted from token balance per transaction. Stream subscriptions cost 1 token each with a 1-hour reconnect grace period.

### PriorityFeeConfig

Controls automatic priority fee optimization for transactions.

```rust
use allenhark_slipstream::{PriorityFeeConfig, PriorityFeeSpeed};

let config = Config::builder()
    .api_key("sk_live_12345678")
    .priority_fee(PriorityFeeConfig {
        enabled: true,
        speed: PriorityFeeSpeed::UltraFast,
        max_tip: Some(0.01),
    })
    .build()?;
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable automatic priority fee optimization |
| `speed` | `PriorityFeeSpeed` | `Fast` | Fee tier: `Slow`, `Fast`, or `UltraFast` |
| `max_tip` | `Option<f64>` | `None` | Maximum tip in SOL (caps the priority fee) |

**PriorityFeeSpeed tiers:**

| Speed | Compute Unit Price | Landing Probability | Use Case |
|-------|-------------------|--------------------|---------|
| `Slow` | Low | ~60-70% | Cost-sensitive, non-urgent transactions |
| `Fast` | Medium | ~85-90% | Default balance of cost and speed |
| `UltraFast` | High | ~95-99% | Time-critical trading, MEV protection |

### ProtocolTimeouts

```rust
use allenhark_slipstream::ProtocolTimeouts;
use std::time::Duration;

let config = Config::builder()
    .api_key("sk_live_12345678")
    .protocol_timeouts(ProtocolTimeouts {
        quic: Duration::from_millis(2000),
        grpc: Duration::from_millis(3000),
        websocket: Duration::from_millis(3000),
        http: Duration::from_millis(5000),
    })
    .build()?;
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `quic` | `Duration` | 2s | QUIC connection timeout |
| `grpc` | `Duration` | 3s | gRPC connection timeout |
| `websocket` | `Duration` | 3s | WebSocket connection timeout |
| `http` | `Duration` | 5s | HTTP request timeout |

### Protocol Fallback Chain

The SDK tries protocols in order until one succeeds:

**QUIC** (2s) -> **gRPC** (3s) -> **WebSocket** (3s) -> **HTTP** (5s)

Override with `preferred_protocol()` to skip the fallback chain:

```rust
use allenhark_slipstream::Protocol;

let config = Config::builder()
    .api_key("sk_live_12345678")
    .preferred_protocol(Protocol::WebSocket)
    .build()?;
```

---

## Connecting

### Auto-Discovery (Recommended)

```rust
use allenhark_slipstream::{Config, SlipstreamClient};

// Minimal -- discovery finds the best worker
let config = Config::builder().api_key("sk_live_xxx").build()?;
let client = SlipstreamClient::connect(config).await?;

// With region preference
let config = Config::builder()
    .api_key("sk_live_xxx")
    .region("us-east")
    .build()?;
let client = SlipstreamClient::connect(config).await?;
```

### Direct Endpoint (Advanced)

Bypass discovery and connect to a specific worker:

```rust
let config = Config::builder()
    .api_key("sk_live_xxx")
    .endpoint("http://worker-ip:9000")
    .build()?;
let client = SlipstreamClient::connect(config).await?;
```

### Connection Info

```rust
let info = client.connection_info();
println!("Session: {}", info.session_id);
println!("Protocol: {}", info.protocol);  // "quic", "grpc", "ws", "http"
println!("Region: {:?}", info.region);
println!("Rate limit: {} rps (burst: {})", info.rate_limit.rps, info.rate_limit.burst);
```

---

## Transaction Submission

### Basic Submit

```rust
let result = client.submit_transaction(&tx_bytes).await?;
println!("TX ID: {}", result.transaction_id);
println!("Status: {:?}", result.status);
if let Some(sig) = &result.signature {
    println!("Signature: {}", sig);
}
```

### Submit with Options

```rust
use allenhark_slipstream::SubmitOptions;

let result = client.submit_transaction_with_options(&tx_bytes, &SubmitOptions {
    broadcast_mode: true,
    preferred_sender: Some("nozomi".to_string()),
    max_retries: 5,
    timeout_ms: 10_000,
    dedup_id: Some("my-unique-id".to_string()),
}).await?;
```

#### SubmitOptions Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `broadcast_mode` | `bool` | `false` | Fan-out to multiple regions simultaneously |
| `preferred_sender` | `Option<String>` | `None` | Prefer a specific sender (e.g., `"nozomi"`, `"0slot"`) |
| `max_retries` | `u32` | `2` | Retry attempts on failure |
| `timeout_ms` | `u64` | `30000` | Timeout per attempt in milliseconds |
| `dedup_id` | `Option<String>` | `None` | Custom deduplication ID (prevents double-submit) |

### TransactionResult Fields

| Field | Type | Description |
|-------|------|-------------|
| `request_id` | `String` | Internal request ID |
| `transaction_id` | `String` | Slipstream transaction ID |
| `signature` | `Option<String>` | Solana transaction signature (base58, when confirmed) |
| `status` | `TransactionStatus` | Current status (see table below) |
| `slot` | `Option<u64>` | Solana slot (when confirmed) |
| `timestamp` | `u64` | Unix timestamp in milliseconds |
| `routing` | `Option<RoutingInfo>` | Routing details (region, sender, latencies) |
| `error` | `Option<TransactionError>` | Error details (on failure) |

### TransactionStatus Values

| Status | Description |
|--------|-------------|
| `Pending` | Received, not yet processed |
| `Processing` | Being validated and routed |
| `Sent` | Forwarded to sender |
| `Confirmed` | Confirmed on Solana |
| `Failed` | Failed permanently |
| `Duplicate` | Deduplicated (already submitted) |
| `RateLimited` | Rate limit exceeded for your tier |
| `InsufficientTokens` | Token balance too low (or free tier daily limit reached) |

### RoutingInfo Fields

| Field | Type | Description |
|-------|------|-------------|
| `region` | `String` | Region that handled the transaction |
| `sender` | `String` | Sender service used |
| `routing_latency_ms` | `u32` | Time spent in routing logic (ms) |
| `sender_latency_ms` | `u32` | Time spent in sender submission (ms) |
| `total_latency_ms` | `u32` | Total end-to-end latency (ms) |

---

## Streaming

Real-time data feeds over QUIC (binary) or WebSocket (JSON).

**Billing:** Each stream subscription costs **1 token (0.00005 SOL)**. If the SDK reconnects within 1 hour for the same stream, no re-billing occurs (reconnect grace period). Free-tier keys deduct from the daily counter instead of tokens.

### Leader Hints

Which region is closest to the current Solana leader. Emitted every 250ms when confidence >= threshold.

```rust
let mut hints = client.subscribe_leader_hints().await?;
while let Some(hint) = hints.recv().await {
    println!("Slot {}: route to {}", hint.slot, hint.preferred_region);
    println!("  Leader: {}", hint.leader_pubkey);
    println!("  Confidence: {}%", hint.confidence);
    println!("  TPU RTT: {}ms", hint.metadata.tpu_rtt_ms);
    println!("  Backups: {:?}", hint.backup_regions);
}
```

#### LeaderHint Fields

| Field | Type | Description |
|-------|------|-------------|
| `timestamp` | `u64` | Unix millis |
| `slot` | `u64` | Current slot |
| `expires_at_slot` | `u64` | Slot when this hint expires |
| `preferred_region` | `String` | Best region for current leader |
| `backup_regions` | `Vec<String>` | Fallback regions in priority order |
| `confidence` | `u32` | Confidence score (0-100) |
| `leader_pubkey` | `String` | Current leader validator pubkey |
| `metadata.tpu_rtt_ms` | `u32` | RTT to leader's TPU from preferred region |
| `metadata.region_score` | `f64` | Region quality score |
| `metadata.leader_tpu_address` | `Option<String>` | Leader's TPU address (ip:port) |
| `metadata.region_rtt_ms` | `Option<HashMap<String, u32>>` | Per-region RTT to leader |

### Tip Instructions

Wallet address and tip amount for building transactions in streaming tip mode.

```rust
let mut tips = client.subscribe_tip_instructions().await?;
while let Some(tip) = tips.recv().await {
    println!("Sender: {} ({})", tip.sender_name, tip.sender);
    println!("  Wallet: {}", tip.tip_wallet_address);
    println!("  Amount: {} SOL (tier: {})", tip.tip_amount_sol, tip.tip_tier);
    println!("  Latency: {}ms, Confidence: {}%", tip.expected_latency_ms, tip.confidence);
    for alt in &tip.alternative_senders {
        println!("  Alt: {} @ {} SOL", alt.sender, alt.tip_amount_sol);
    }
}

// Latest cached tip (no subscription required)
if let Some(tip) = client.get_latest_tip().await {
    println!("Tip {} SOL to {}", tip.tip_amount_sol, tip.tip_wallet_address);
}
```

#### TipInstruction Fields

| Field | Type | Description |
|-------|------|-------------|
| `timestamp` | `u64` | Unix millis |
| `sender` | `String` | Sender ID |
| `sender_name` | `String` | Human-readable sender name |
| `tip_wallet_address` | `String` | Tip wallet address (base58) |
| `tip_amount_sol` | `f64` | Required tip amount in SOL |
| `tip_tier` | `String` | Tip tier name |
| `expected_latency_ms` | `u32` | Expected submission latency (ms) |
| `confidence` | `u32` | Confidence score (0-100) |
| `valid_until_slot` | `u64` | Slot until which this tip is valid |
| `alternative_senders` | `Vec<AlternativeSender>` | Alternative sender options |

### Priority Fees

Network-condition-based fee recommendations, updated every second.

```rust
let mut fees = client.subscribe_priority_fees().await?;
while let Some(fee) = fees.recv().await {
    println!("Speed: {}", fee.speed);
    println!("  CU price: {} micro-lamports", fee.compute_unit_price);
    println!("  CU limit: {}", fee.compute_unit_limit);
    println!("  Est cost: {} SOL", fee.estimated_cost_sol);
    println!("  Landing probability: {}%", fee.landing_probability);
    println!("  Congestion: {}", fee.network_congestion);
    println!("  Recent success rate: {:.1}%", fee.recent_success_rate * 100.0);
}
```

#### PriorityFee Fields

| Field | Type | Description |
|-------|------|-------------|
| `timestamp` | `u64` | Unix millis |
| `speed` | `String` | Fee speed tier (`"slow"`, `"fast"`, `"ultra_fast"`) |
| `compute_unit_price` | `u64` | Compute unit price in micro-lamports |
| `compute_unit_limit` | `u32` | Recommended compute unit limit |
| `estimated_cost_sol` | `f64` | Estimated total priority fee in SOL |
| `landing_probability` | `u32` | Estimated landing probability (0-100) |
| `network_congestion` | `String` | Network congestion level (`"low"`, `"medium"`, `"high"`) |
| `recent_success_rate` | `f64` | Recent success rate (0.0-1.0) |

### Latest Blockhash

Streams the latest blockhash every 2 seconds. Build transactions without a separate RPC call.

```rust
let mut bh_stream = client.subscribe_latest_blockhash().await?;
while let Some(bh) = bh_stream.recv().await {
    println!("Blockhash: {}", bh.blockhash);
    println!("  Valid until block height: {}", bh.last_valid_block_height);
}
```

#### LatestBlockhash Fields

| Field | Type | Description |
|-------|------|-------------|
| `blockhash` | `String` | Latest blockhash (base58) |
| `last_valid_block_height` | `u64` | Last valid block height for this blockhash |
| `timestamp` | `u64` | Unix millis when fetched |

### Latest Slot

Streams the current confirmed slot on every slot change (~400ms).

```rust
let mut slot_stream = client.subscribe_latest_slot().await?;
while let Some(s) = slot_stream.recv().await {
    println!("Current slot: {}", s.slot);
}
```

#### LatestSlot Fields

| Field | Type | Description |
|-------|------|-------------|
| `slot` | `u64` | Current confirmed slot number |
| `timestamp` | `u64` | Unix millis |

### Transaction Updates

Real-time status updates for submitted transactions.

```rust
// Transaction updates arrive automatically after submission
// Access via the returned TransactionResult or stream
```

### Auto-Subscribe on Connect

Enable streams at configuration time so they activate immediately:

```rust
let config = Config::builder()
    .api_key("sk_live_12345678")
    .leader_hints(true)                  // default: true
    .stream_tip_instructions(true)       // default: false
    .stream_priority_fees(true)          // default: false
    .stream_latest_blockhash(true)       // default: false
    .stream_latest_slot(true)            // default: false
    .build()?;

let client = SlipstreamClient::connect(config).await?;
// All 5 streams are active immediately -- just consume channels
```

### Handling Multiple Streams

Use `tokio::select!` to handle all streams concurrently:

```rust
let mut hints = client.subscribe_leader_hints().await?;
let mut tips = client.subscribe_tip_instructions().await?;
let mut fees = client.subscribe_priority_fees().await?;
let mut bh = client.subscribe_latest_blockhash().await?;
let mut slots = client.subscribe_latest_slot().await?;

loop {
    tokio::select! {
        Some(hint) = hints.recv() => {
            println!("Leader in {} ({}%)", hint.preferred_region, hint.confidence);
        }
        Some(tip) = tips.recv() => {
            println!("Tip {} SOL to {}", tip.tip_amount_sol, tip.tip_wallet_address);
        }
        Some(fee) = fees.recv() => {
            println!("Fee {} µL/CU ({}% landing)", fee.compute_unit_price, fee.landing_probability);
        }
        Some(blockhash) = bh.recv() => {
            println!("Blockhash: {}", blockhash.blockhash);
        }
        Some(slot) = slots.recv() => {
            println!("Slot: {}", slot.slot);
        }
    }
}
```

---

## Keep-Alive & Time Sync

Background keep-alive mechanism providing latency measurement and NTP-style clock synchronization.

```rust
// Enabled by default (5s interval)
let config = Config::builder()
    .api_key("sk_live_12345678")
    .keepalive(true)              // default: true
    .keepalive_interval(5)        // default: 5 seconds
    .build()?;

let client = SlipstreamClient::connect(config).await?;

// Manual ping
let ping = client.ping().await?;
println!("RTT: {}ms, Clock offset: {}ms, Server time: {}",
    ping.rtt_ms, ping.clock_offset_ms, ping.server_time);

// Derived measurements (median from sliding window of 10 samples)
if let Some(latency) = client.latency_ms() {
    println!("One-way latency: {}ms", latency);     // RTT / 2
}
if let Some(offset) = client.clock_offset_ms() {
    println!("Clock offset: {}ms", offset);          // server - client time difference
}
let server_now = client.server_time();                // local time + offset
```

#### PingResult Fields

| Field | Type | Description |
|-------|------|-------------|
| `seq` | `u32` | Sequence number |
| `rtt_ms` | `u64` | Round-trip time in milliseconds |
| `clock_offset_ms` | `i64` | Clock offset: `server_time - (client_send_time + rtt/2)` (can be negative) |
| `server_time` | `u64` | Server timestamp at time of pong (unix millis) |

---

## Token Billing

Token-based billing system. Paid tiers (Standard/Pro/Enterprise) deduct tokens per transaction and stream subscription. Free tier uses a daily counter.

### Token Economics

| Unit | Value |
|------|-------|
| 1 token | 1 transaction (Standard tier) |
| 1 token | 50,000 lamports |
| 1 token | 0.00005 SOL |
| 1 stream subscription | 1 token (with 1-hour reconnect grace) |
| Min deposit | $10 USD equivalent in SOL |
| Initial balance (new key) | 0.01 SOL (200 tokens) |
| Grace period | -0.001 SOL (-20 tokens) before hard block |

### Check Balance

```rust
let balance = client.get_balance().await?;
println!("SOL:    {:.6}", balance.balance_sol);
println!("Tokens: {}", balance.balance_tokens);
println!("Lamports: {}", balance.balance_lamports);
println!("Grace remaining: {} tokens", balance.grace_remaining_tokens);
```

#### Balance Fields

| Field | Type | Description |
|-------|------|-------------|
| `balance_sol` | `f64` | Balance in SOL |
| `balance_tokens` | `i64` | Balance in tokens (1 token = 1 query) |
| `balance_lamports` | `i64` | Balance in lamports |
| `grace_remaining_tokens` | `i64` | Grace tokens remaining before hard block |

### Get Deposit Address

```rust
let deposit = client.get_deposit_address().await?;
println!("Send SOL to: {}", deposit.deposit_wallet);
println!("Minimum: {:.4} SOL", deposit.min_amount_sol);
```

### Minimum Deposit

Deposits must reach **$10 USD equivalent** in SOL before tokens are credited. Deposits below this threshold accumulate as pending.

```rust
let min_usd = client.get_minimum_deposit_usd(); // 10.0

let pending = client.get_pending_deposit().await?;
println!("Pending: {:.6} SOL ({} deposits)", pending.pending_sol, pending.pending_count);
```

### Deposit History

```rust
use allenhark_slipstream::types::DepositHistoryOptions;

let deposits = client.get_deposit_history(DepositHistoryOptions {
    limit: Some(20),
    offset: None,
}).await?;

for d in &deposits {
    println!("{:.6} SOL | ${:.2} USD | {}",
        d.amount_sol,
        d.usd_value.unwrap_or(0.0),
        if d.credited { "CREDITED" } else { "PENDING" });
}
```

### Usage History

```rust
use allenhark_slipstream::types::UsageHistoryOptions;

let entries = client.get_usage_history(UsageHistoryOptions {
    limit: Some(50),
    offset: None,
}).await?;

for entry in &entries {
    println!("{}: {} lamports (balance after: {})",
        entry.tx_type, entry.amount_lamports, entry.balance_after_lamports);
}
```

### Free Tier Usage

For free-tier API keys, check the daily usage counter:

```rust
let usage = client.get_free_tier_usage().await?;
println!("Used: {}/{}", usage.used, usage.limit);       // e.g. 42/100
println!("Remaining: {}", usage.remaining);              // e.g. 58
println!("Resets at: {}", usage.resets_at);              // UTC midnight ISO string
```

#### FreeTierUsage Fields

| Field | Type | Description |
|-------|------|-------------|
| `used` | `u32` | Transactions used today |
| `remaining` | `u32` | Remaining transactions today |
| `limit` | `u32` | Daily transaction limit (100) |
| `resets_at` | `String` | UTC midnight reset time (RFC 3339) |

---

## Multi-Region Routing

`MultiRegionClient` connects to workers across multiple regions and automatically routes transactions to the region closest to the current Solana leader.

### Auto-Discovery

```rust
use allenhark_slipstream::{Config, MultiRegionClient};

let config = Config::builder()
    .api_key("sk_live_12345678")
    .build()?;

let multi = MultiRegionClient::connect(config).await?;

// Transactions auto-route to the best region based on leader hints
let result = multi.submit_transaction(&tx_bytes).await?;

// Check current routing decision
if let Some(routing) = multi.get_current_routing() {
    println!("Best region: {} (confidence: {}%)", routing.best_region, routing.confidence);
    println!("Leader: {}", routing.leader_pubkey);
    println!("Expected RTT: {:?}ms", routing.expected_rtt_ms);
    println!("Fallbacks: {:?}", routing.fallback_regions);
}

println!("Connected regions: {:?}", multi.connected_regions());
```

### Manual Worker Configuration

```rust
use allenhark_slipstream::{MultiRegionClient, Config, WorkerEndpoint, MultiRegionConfig};

let workers = vec![
    WorkerEndpoint::new("w1", "us-east", "10.0.1.1"),
    WorkerEndpoint::new("w2", "eu-central", "10.0.2.1"),
    WorkerEndpoint::new("w3", "asia-pacific", "10.0.3.1"),
];

let multi = MultiRegionClient::create(
    Config::builder().api_key("sk_live_xxx").build()?,
    workers,
    MultiRegionConfig {
        auto_follow_leader: true,
        min_switch_confidence: 70,
        switch_cooldown_ms: 5000,
        broadcast_high_priority: false,
        max_broadcast_regions: 3,
    },
).await?;
```

#### MultiRegionConfig Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `auto_follow_leader` | `bool` | `true` | Auto-switch region based on leader hints |
| `min_switch_confidence` | `u32` | `60` | Minimum confidence (0-100) to trigger region switch |
| `switch_cooldown_ms` | `u64` | `500` | Cooldown between region switches (ms) |
| `broadcast_high_priority` | `bool` | `false` | Broadcast high-priority transactions to all regions |
| `max_broadcast_regions` | `usize` | `3` | Maximum regions for broadcast mode |

### Routing Recommendation

Ask the server for the current best region:

```rust
let rec = client.get_routing_recommendation().await?;
println!("Best: {} ({}%)", rec.best_region, rec.confidence);
println!("Leader: {}", rec.leader_pubkey);
println!("Fallbacks: {:?} (strategy: {:?})", rec.fallback_regions, rec.fallback_strategy);
println!("Valid for: {}ms", rec.valid_for_ms);
```

#### RoutingRecommendation Fields

| Field | Type | Description |
|-------|------|-------------|
| `best_region` | `String` | Recommended region |
| `leader_pubkey` | `String` | Current leader validator pubkey |
| `slot` | `u64` | Current slot |
| `confidence` | `u32` | Confidence score (0-100) |
| `expected_rtt_ms` | `Option<u32>` | Expected RTT to leader from best region |
| `fallback_regions` | `Vec<String>` | Fallback regions in priority order |
| `fallback_strategy` | `FallbackStrategy` | `Sequential`, `Broadcast`, `Retry`, or `None` |
| `valid_for_ms` | `u64` | Time until this recommendation expires |

---

## Deduplication

Prevent duplicate submissions with `dedup_id`:

```rust
let options = SubmitOptions {
    dedup_id: Some("unique-tx-id-12345".to_string()),
    max_retries: 5,
    ..Default::default()
};

// Same dedup_id across retries = safe from double-spend
let result = client.submit_transaction_with_options(&tx_bytes, &options).await?;
```

---

## Connection Status & Metrics

```rust
// Connection status
let status = client.connection_status().await;
println!("State: {:?}", status.state);       // Connected, Disconnected, etc.
println!("Protocol: {:?}", status.protocol); // Quic, Grpc, WebSocket, Http
println!("Region: {:?}", status.region);
println!("Latency: {}ms", status.latency_ms);

// Performance metrics
let metrics = client.metrics();
println!("Submitted: {}", metrics.transactions_submitted);
println!("Confirmed: {}", metrics.transactions_confirmed);
println!("Avg latency: {:.1}ms", metrics.average_latency_ms);
println!("Success rate: {:.1}%", metrics.success_rate * 100.0);
```

---

## Error Handling

```rust
use allenhark_slipstream::SdkError;

match client.submit_transaction(&tx_bytes).await {
    Ok(result) => println!("Signature: {:?}", result.signature),
    Err(SdkError::Connection(msg)) => println!("Connection error: {}", msg),
    Err(SdkError::Auth(msg)) => println!("Auth error: {}", msg),
    Err(SdkError::RateLimited) => println!("Rate limited -- back off"),
    Err(SdkError::InsufficientTokens) => {
        let balance = client.get_balance().await?;
        let deposit = client.get_deposit_address().await?;
        println!("Low balance: {} tokens", balance.balance_tokens);
        println!("Deposit to: {}", deposit.deposit_wallet);
    }
    Err(SdkError::Timeout) => println!("Request timed out"),
    Err(e) => println!("Error: {}", e),
}
```

### Error Variants

| Variant | Description |
|---------|-------------|
| `Config(msg)` | Invalid configuration |
| `Connection(msg)` | Connection failure |
| `Auth(msg)` | Authentication failure (invalid API key) |
| `Protocol(msg)` | Protocol-level error |
| `Transaction(msg)` | Transaction submission error |
| `Timeout` | Operation timed out |
| `RateLimited` | Rate limit exceeded for your tier |
| `NotConnected` | Client not connected |
| `StreamClosed` | Stream closed unexpectedly |
| `InsufficientTokens` | Token balance too low |
| `AllProtocolsFailed` | All connection protocols failed |
| `Internal(msg)` | Internal SDK error |

---

## Examples

| Example | Description |
|---------|-------------|
| `basic.rs` | Simple connection and disconnection |
| `submit_transaction.rs` | Transaction submission with options |
| `streaming_callbacks.rs` | All streaming subscriptions with `tokio::select!` |
| `priority_fees.rs` | Priority fee configuration and streaming |
| `leader_hints.rs` | Leader region hints for optimal routing |
| `billing.rs` | Token balance, deposits, usage history |
| `deduplication.rs` | Prevent duplicate transaction submissions |
| `advanced_config.rs` | Full configuration options |
| `broadcast_tx.rs` | Fan-out to multiple regions |
| `signer_integration.rs` | Keypair, Ledger, multi-sig, MPC signing |

```bash
SLIPSTREAM_API_KEY=sk_test_xxx cargo run --example streaming_callbacks
```

## License

Apache-2.0
