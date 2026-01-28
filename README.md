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

The `SlipstreamClient` handles connection logic, authentication, and worker selection automatically.

```rust
use allenhark_slipstream::{Config, SlipstreamClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Basic connectivity
    let config = Config::builder()
        .api_key("sk_test_12345678") // Your API Key
        .region("us-east")           // Optional: Prefer a specific region
        .build()?;

    let client = SlipstreamClient::connect(config).await?;
    
    println!("Connected via: {}", client.connection_info().protocol);
    
    Ok(())
}
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
| `endpoint` | Explicit URL override (disables worker selection). | None |
| `leader_hints` | Enable auto-subscription to leader hints. | `true` |
| `protocol_timeouts` | Custom timeouts for QUIC, gRPC, etc. | Smart defaults |

## Architecture

1.  **Worker Selection**: When you call `connect()`, the SDK queries the mesh for available workers and pings them. It selects the one with the lowest Round-Trip Time (RTT).
2.  **Protocol Fallback**: The client attempts to connect using **QUIC**. If that fails (e.g., firewall blocked), it tries **gRPC**, then **WebSocket**, then **HTTP**.
3.  **Authentication**: All requests are signed and authenticated using your `api_key`.

## Examples

Check the `examples/` directory for complete, runable code:

- `examples/basic.rs`: Simple connection demo.
- `examples/submit_transaction.rs`: submitting transactions with options.

To run an example:
```bash
cargo run --example basic
```

## Governance & Support

This SDK is community supported.
- **Issues**: Please file bug reports on GitHub.
- **Enterprise Support**: Available at [allenhark.com](https://allenhark.com).