[![allenhark.com](https://allenhark.com/allenhark-logo.png)](https://allenhark.com)

# Slipstream Rust SDK

The official Rust client for **AllenHark Slipstream**, the high-performance Solana transaction relay and intelligence network.

[![Crates.io](https://img.shields.io/crates/v/allenhark-slipstream.svg)](https://crates.io/crates/allenhark-slipstream)
[![Documentation](https://docs.rs/allenhark-slipstream/badge.svg)](https://docs.rs/allenhark-slipstream)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

## Overview

The Slipstream SDK provides a robust, low-latency connection to the Slipstream Global Mesh. It allows Solana applications to:
- **Submit Transactions** with minimal latency using QUIC and IP-based routing.
- **Stream Intelligence** such as leader hints, priority fees, and tip instructions in real-time.
- **Ensure Reliability** through automatic protocol fallback and background health monitoring.


## Features

- **Multi-Protocol Support**: Primary **QUIC** transport for speed, falling back to **gRPC**, **WebSocket**, and **HTTP** to ensure connectivity in any environment.
- **Smart Worker Selection**: Automatically pings and selects the lowest-latency worker endpoint in the mesh (IP-based routing).
- **Resilience**: Integrated `HealthMonitor` checks connection status and reconnects transparently.
- **Real-Time Streams**: Subscribe to network insights:
    - `LeaderHint`: Which region/validator is leading the current slot.
    - `PriorityFee`: Dynamic compute unit pricing.
    - `TipInstruction`: Optimal tip amounts and destinations.

## Installation

Add the following to your `Cargo.toml`:

```toml
[dependencies]
allenhark-slipstream = "0.1"
tokio = { version = "1.0", features = ["full"] }
```

## Quick Start

### Connecting

The `SlipstreamClient` automatically discovers available workers via the discovery service and connects directly to the best worker's IP address. No manual endpoint configuration needed.

```rust
use allenhark_slipstream::{Config, SlipstreamClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Just an API key — discovery handles the rest
    let config = Config::builder()
        .api_key("sk_live_12345678")
        .build()?;

    let client = SlipstreamClient::connect(config).await?;

    println!("Connected via: {}", client.connection_info().protocol);

    Ok(())
}
```

Optionally prefer a specific region:

```rust
let config = Config::builder()
    .api_key("sk_live_12345678")
    .region("us-east")  // Optional: prefer this region
    .build()?;
```

### Submitting a Transaction

```rust
// Submit a raw signed transaction (serialized as bytes)
let tx_data: Vec<u8> = vec![...]; 

let result = client.submit_transaction(&tx_data).await?;

println!("Transaction ID: {}", result.transaction_id);
println!("Status: {:?}", result.status);
```

### Advanced Submission Options

You can control broadcast behavior and retry logic:

```rust
use allenhark_slipstream::SubmitOptions;

let options = SubmitOptions {
    broadcast_mode: true, // Fan-out to multiple regions
    max_retries: 5,
    ..Default::default()
};

let result = client.submit_transaction_with_options(&tx_data, &options).await?;
```

### Streaming Intelligence

Subscribe to real-time data feeds to optimize your trading or transaction strategy.

```rust
// 1. Leader Hints
let mut hints = client.subscribe_leader_hints().await?;
while let Some(hint) = hints.recv().await {
    println!("Current Leader Region: {}", hint.preferred_region);
}

// 2. Priority Fees
let mut fee_stream = client.subscribe_priority_fees().await?;
while let Some(fee) = fee_stream.recv().await {
    println!("Recommended Fee: {} micro-lamports", fee.compute_unit_price);
}
```

## Configuration

The `Config::builder()` provides a fluent interface for configuration:

| Option | Description | Default |
|--------|-------------|---------|
| `api_key` | **Required.** Your authentication key. | - |
| `region` | Preferred region (e.g., `us-east`, `eu-central`). | Auto-detect |
| `discovery_url` | Discovery service URL for worker lookup. | `https://discovery.slipstream.allenhark.com` |
| `endpoint` | Explicit URL override (disables discovery). | None |
| `leader_hints` | Enable auto-subscription to leader hints. | `true` |
| `protocol_timeouts` | Custom timeouts for QUIC, gRPC, etc. | Smart defaults |
| `priority_fee` | Priority fee configuration (`PriorityFeeConfig`). | Disabled |
| `retry_backoff` | Retry strategy: `Linear` or `Exponential`. | `Exponential` |
| `min_confidence` | Min confidence for leader hints (0-100). | `70` |
| `keepalive` | Enable background keep-alive ping. | `true` |
| `keepalive_interval` | Keep-alive ping interval in seconds. | `5` |
| `idle_timeout` | Connection idle timeout. | None (no timeout) |

### Priority Fee Configuration

```rust
use allenhark_slipstream::{PriorityFeeConfig, PriorityFeeSpeed};

let config = Config::builder()
    .api_key("sk_test_123")
    .priority_fee(PriorityFeeConfig {
        enabled: true,
        speed: PriorityFeeSpeed::UltraFast,
        max_tip: Some(0.01), // Max 0.01 SOL
    })
    .build()?;
```

### Helper Methods

```rust
// Get the latest tip instruction (cached)
if let Some(tip) = client.get_latest_tip().await {
    println!("Tip {} SOL to {}", tip.tip_amount_sol, tip.tip_wallet_address);
}

// Get connection status
let status = client.connection_status().await;
println!("State: {:?}, Protocol: {:?}", status.state, status.protocol);

// Get performance metrics
let metrics = client.metrics();
println!("Submitted: {}, Success Rate: {:.1}%", 
    metrics.transactions_submitted, metrics.success_rate * 100.0);
```

## Keep-Alive & Time Sync

The SDK includes a background keep-alive mechanism that also provides latency measurement and clock synchronization with the server using NTP-style calculation.

```rust
// Enabled by default (5s interval). Configure via:
let config = Config::builder()
    .api_key("sk_live_12345678")
    .keepalive(true)              // default: true
    .keepalive_interval(5)        // default: 5 seconds
    .build()?;

let client = SlipstreamClient::connect(config).await?;

// Manual ping
let ping = client.ping().await?;
println!("RTT: {}ms, Clock offset: {}ms", ping.rtt_ms, ping.clock_offset_ms);

// Latency (median one-way from sliding window of 10 samples)
if let Some(latency) = client.latency_ms() {
    println!("One-way latency: {}ms", latency);
}

// Clock offset (median from sliding window)
if let Some(offset) = client.clock_offset_ms() {
    println!("Clock offset: {}ms", offset);
}

// Server time (local clock corrected by offset)
let server_now = client.server_time();
println!("Server time: {} unix ms", server_now);
```

## Token Billing

The SDK provides methods to manage your token balance, view deposits, and track usage.

### Check Balance

```rust
let balance = client.get_balance().await?;
println!("Balance: {:.6} SOL ({} tokens)", balance.balance_sol, balance.balance_tokens);
println!("Grace remaining: {} tokens", balance.grace_remaining_tokens);
```

### Get Deposit Address

Get your deposit wallet to top up tokens by sending SOL:

```rust
let deposit = client.get_deposit_address().await?;
println!("Send SOL to: {}", deposit.deposit_wallet);
println!("Minimum: {:.4} SOL", deposit.min_amount_sol);
```

### Minimum Deposit

Deposits must be at least **$10 USD equivalent** in SOL before tokens are credited. Deposits below this threshold are stored as pending until the cumulative total reaches $10.

```rust
let min_usd = client.get_minimum_deposit_usd(); // Returns 10.0

// Check pending (uncredited) deposits
let pending = client.get_pending_deposit().await?;
println!("Pending: {:.6} SOL ({} deposits)", pending.pending_sol, pending.pending_count);
```

### Deposit History

View all SOL deposits with their credited/pending status:

```rust
use allenhark_slipstream::types::DepositHistoryOptions;

let deposits = client.get_deposit_history(DepositHistoryOptions {
    limit: Some(20),
    offset: None,
}).await?;

for d in &deposits {
    println!("{} | {:.6} SOL | ${:.2} | {}",
        &d.signature[..16], d.amount_sol,
        d.usd_value.unwrap_or(0.0),
        if d.credited { "CREDITED" } else { "PENDING" });
}
```

### Usage History

View token transaction history (debits, credits, deposits):

```rust
use allenhark_slipstream::types::UsageHistoryOptions;

let entries = client.get_usage_history(UsageHistoryOptions {
    limit: Some(50),
    offset: None,
}).await?;

for entry in &entries {
    println!("{}: {} lamports (balance: {})",
        entry.tx_type, entry.amount_lamports, entry.balance_after_lamports);
}
```

### Token Economics

| Unit | Value |
|------|-------|
| 1 token | 1 query |
| 1 token | 50,000 lamports |
| 1 token | 0.00005 SOL |
| Min deposit | $10 USD equivalent in SOL |

## Architecture

1.  **Discovery**: When you call `connect()`, the SDK calls the discovery service (`GET /v1/discovery`) to fetch available workers across all regions, including their IP addresses and ports.
2.  **Worker Selection**: The SDK filters workers by your preferred region (or uses the recommended region), then selects the best healthy worker.
3.  **Protocol Fallback**: The client attempts to connect using **QUIC**. If that fails (e.g., firewall blocked), it tries **gRPC**, then **WebSocket**, then **HTTP**.
4.  **Authentication**: All requests are signed and authenticated using your `api_key`.

## Multi-Region Routing

For optimal latency, use `MultiRegionClient` to automatically route transactions to the region closest to the current Solana leader:

```rust
use allenhark_slipstream::{Config, MultiRegionClient};

let config = Config::builder()
    .api_key("sk_live_12345678")
    .build()?;

// Discovers workers across all regions automatically
let multi = MultiRegionClient::connect(config).await?;

// Transactions are routed to the best region based on leader hints
let result = multi.submit_transaction(&tx_data).await?;

// Check current routing decision
if let Some(routing) = multi.get_current_routing() {
    println!("Routing to {} (confidence: {}%)", routing.best_region, routing.confidence);
}
```

## Examples

The `examples/` directory contains comprehensive, runnable examples:

| Example | Description |
|---------|-------------|
| `basic.rs` | Simple connection and disconnection |
| `submit_transaction.rs` | Transaction submission with options |
| `priority_fees.rs` | Priority fee configuration and streaming |
| `tip_instructions.rs` | Tip wallet and amount streaming |
| `leader_hints.rs` | Leader region hints for optimal routing |
| `broadcast_tx.rs` | Fan-out to multiple regions |
| `signer_integration.rs` | Keypair, Ledger, multi-sig, MPC signing |
| `deduplication.rs` | Prevent duplicate transaction submissions |
| `streaming_callbacks.rs` | All streaming subscriptions (Rust channels) |
| `advanced_config.rs` | Full configuration options |
| `billing.rs` | Token balance, deposits, usage history |

Run any example with:
```bash
SLIPSTREAM_API_KEY=sk_test_xxx cargo run --example priority_fees
```

## Streaming (Callbacks)

Rust uses async channels instead of JavaScript-style callbacks:

```rust
// Subscribe to streams
let mut hints = client.subscribe_leader_hints().await?;
let mut tips = client.subscribe_tip_instructions().await?;
let mut fees = client.subscribe_priority_fees().await?;

// Handle all streams concurrently
loop {
    tokio::select! {
        Some(hint) = hints.recv() => {
            println!("Leader in {}", hint.preferred_region);
        }
        Some(tip) = tips.recv() => {
            println!("Tip {} SOL", tip.tip_amount_sol);
        }
        Some(fee) = fees.recv() => {
            println!("Fee {} µL/CU", fee.compute_unit_price);
        }
    }
}
```

## Deduplication

Prevent duplicate submissions with `dedup_id`:

```rust
let options = SubmitOptions {
    dedup_id: Some("unique-tx-id-12345".to_string()),
    max_retries: 5,
    ..Default::default()
};

// Same dedup_id across retries = safe from double-spend
let result = client.submit_transaction_with_options(&tx, &options).await?;
```

## Error Handling

```rust
use allenhark_slipstream::SdkError;

match client.submit_transaction(&tx).await {
    Ok(result) => println!("Signature: {:?}", result.signature),
    Err(SdkError::Connection(msg)) => println!("Connection error: {}", msg),
    Err(SdkError::RateLimited) => println!("Rate limited, back off"),
    Err(e) => println!("Error: {}", e),
}
```

## Governance & Support

This SDK is community supported.
- **Issues**: Please file bug reports on GitHub.
- **Docs**: See `docs/ARCHITECTURE.md` for technical details.
- **Enterprise Support**: Available at [allenhark.com](https://allenhark.com).
