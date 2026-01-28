# Slipstream SDK Architecture

This document describes the internal architecture of the Slipstream Rust SDK, focusing on how it ensures low-latency, reliable connections to the global mesh.

## Table of Contents

1. [Connection Lifecycle](#1-connection-lifecycle)
2. [State Machine](#2-state-machine)
3. [Configuration Reference](#3-configuration-reference)
4. [Streaming & Subscriptions](#4-streaming--subscriptions)
5. [Transaction Submission](#5-transaction-submission)
6. [Error Handling](#6-error-handling)
7. [Class Structure](#7-class-structure)
8. [Security](#8-security)

---

## 1. Connection Lifecycle

When `SlipstreamClient::connect()` is called, the following process occurs:

### A. Worker Selection (Discovery Phase)

Unless a specific `endpoint` is provided in the config:

1. The SDK initializes the `WorkerSelector`.
2. It loads a list of available worker nodes (bootstrap list or cached).
3. **Parallel Pinging**: The SDK sends concurrent HTTP `HEAD` requests to the `/health` endpoint of known workers.
4. **Selection**: It calculates the RTT (Round Trip Time) for each and selects the worker with the lowest latency.
5. **Region Preference**: If `region` is configured (e.g., "us-east"), it prioritizes workers in that region.

### B. Protocol Negotiation (Fallback Chain)

Once a target worker (IP/Host) is selected, the `FallbackChain` attempts to establish a connection using protocols in the following order:

| Protocol | Port | Timeout | Pros | Cons |
|----------|------|---------|------|------|
| **QUIC** | 4433 | 2s | Lowest latency, 0-RTT resumption | May be blocked by firewalls |
| **gRPC** | 10000 | 3s | Multiplexing, strict typing | Requires HTTP/2 support |
| **WebSocket** | 9000 | 3s | Full-duplex, ubiquitous | Higher latency than QUIC |
| **HTTP** | 9000 | 5s | Works everywhere | Polling-based, high latency |

The first protocol to successfully handshake is used for the session.

### C. Health Monitoring

The `HealthMonitor` runs in a background Tokio task:
- Periodically checks connection status
- **Auto-Reconnect**: If the connection drops, triggers Discovery and Negotiation again
- Respects `idle_timeout` configuration

---

## 2. State Machine

The SDK follows this state machine:

```
UNINITIALIZED → INITIALIZED → CONNECTING → CONNECTED ↔ STREAMING/SUBMITTING
                                    ↓
                               DISCONNECTED ← DISCONNECTING ← ERROR
```

### States

| State | Description | Transitions |
|-------|-------------|-------------|
| `Disconnected` | Initial state, no connection | → `Connecting` via `connect()` |
| `Connecting` | Protocol fallback in progress | → `Connected` on success, → `Error` on failure |
| `Connected` | Active connection established | → `Streaming`, `Submitting`, `Disconnecting` |
| `Error` | Connection failed | → `Connecting` on retry |

Access current state via:
```rust
let status = client.connection_status().await;
println!("State: {:?}, Protocol: {:?}", status.state, status.protocol);
```

---

## 3. Configuration Reference

### Authentication

| Option | Type | Required | Default | Description |
|--------|------|----------|---------|-------------|
| `api_key` | String | ✅ | - | Your Slipstream API key |

### Network

| Option | Type | Required | Default | Description |
|--------|------|----------|---------|-------------|
| `region` | String | ❌ | auto-detect | Prefer workers in this region |
| `endpoint` | String | ❌ | - | Override worker selection with explicit URL |

### Connection

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `connection_timeout` | Duration | 10s | Max time to establish connection |
| `max_retries` | u32 | 3 | Retry count for failed submissions |
| `retry_backoff` | BackoffStrategy | Exponential | `Linear` or `Exponential` |
| `idle_timeout` | Duration | None | Disconnect after idle period |
| `protocol_timeouts` | ProtocolTimeouts | See below | Per-protocol timeout overrides |

#### Protocol Timeouts

```rust
ProtocolTimeouts {
    quic: Duration::from_millis(2000),
    grpc: Duration::from_millis(3000),
    websocket: Duration::from_millis(3000),
    http: Duration::from_millis(5000),
}
```

### Features

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `leader_hints` | bool | true | Stream leader region hints |
| `stream_tip_instructions` | bool | false | Stream tip wallet/amount updates |
| `stream_priority_fees` | bool | false | Stream priority fee recommendations |
| `min_confidence` | u32 | 70 | Minimum confidence % for hints |

### Priority Fee

```rust
priority_fee: PriorityFeeConfig {
    enabled: bool,           // Enable auto fee optimization
    speed: PriorityFeeSpeed, // Slow, Fast, UltraFast
    max_tip: Option<f64>,    // Maximum tip in SOL
}
```

---

## 4. Streaming & Subscriptions

The SDK provides three real-time data streams:

### Leader Hints

```rust
let mut hints = client.subscribe_leader_hints().await?;
while let Some(hint) = hints.recv().await {
    println!("Leader in {} ({}% confidence)", 
        hint.preferred_region, hint.confidence);
}
```

**Fields:**
- `preferred_region`: Optimal region for current leader
- `backup_regions`: Fallback regions in preference order
- `confidence`: 0-100% confidence score
- `expires_at_slot`: Slot when hint becomes stale
- `leader_pubkey`: Validator's public key (optional)

### Tip Instructions

```rust
let mut tips = client.subscribe_tip_instructions().await?;
while let Some(tip) = tips.recv().await {
    println!("Tip {} SOL to {}", tip.tip_amount_sol, tip.tip_wallet_address);
}
```

**Fields:**
- `tip_wallet_address`: Base58 address to send tip
- `tip_amount_sol`: Recommended tip amount
- `sender`: Sender ID for routing
- `tip_tier`: Service tier name
- `expected_latency_ms`: Expected landing latency
- `alternative_senders`: Backup sender options

### Priority Fees

```rust
let mut fees = client.subscribe_priority_fees().await?;
while let Some(fee) = fees.recv().await {
    println!("Fee: {} µL/CU", fee.compute_unit_price);
}
```

**Fields:**
- `compute_unit_price`: Price in micro-lamports per CU
- `compute_unit_limit`: Recommended CU limit
- `landing_probability`: Estimated landing %
- `network_congestion`: low/medium/high

### Cached Access

```rust
// Get latest tip without streaming
if let Some(tip) = client.get_latest_tip().await {
    // Use for transaction building
}
```

---

## 5. Transaction Submission

### Basic Submission

```rust
let result = client.submit_transaction(&signed_tx_bytes).await?;
println!("ID: {}, Status: {:?}", result.transaction_id, result.status);
```

### With Options

```rust
let options = SubmitOptions {
    broadcast_mode: true,  // Fan-out to all regions
    preferred_sender: None,
    max_retries: 5,
    timeout_ms: 60_000,
    dedup_id: Some("unique-id".to_string()),
};

let result = client.submit_transaction_with_options(&tx, &options).await?;
```

### Deduplication

Use `dedup_id` to prevent duplicate submissions:

```rust
// SAME dedup_id across retries = safe from double-spend
let dedup_id = format!("tx-{}-{}", user_id, nonce);

for attempt in 1..=3 {
    let options = SubmitOptions {
        dedup_id: Some(dedup_id.clone()),
        ..Default::default()
    };
    
    match client.submit_transaction_with_options(&tx, &options).await {
        Ok(r) if r.status.is_terminal() => return Ok(r),
        Err(_) => continue,
        _ => continue,
    }
}
```

### Transaction Result

```rust
TransactionResult {
    request_id: String,      // SDK request ID
    transaction_id: String,  // Server-assigned ID
    status: TransactionStatus,
    signature: Option<String>,
    routing: Option<RoutingInfo>,
    error: Option<TransactionError>,
}
```

---

## 6. Error Handling

The SDK uses `Result<T, SdkError>` for all operations:

```rust
use allenhark_slipstream::SdkError;

match client.submit_transaction(&tx).await {
    Ok(result) => { /* success */ }
    Err(SdkError::Connection(msg)) => { /* network issues */ }
    Err(SdkError::Protocol(msg)) => { /* protocol-level error */ }
    Err(SdkError::Config(msg)) => { /* configuration invalid */ }
    Err(SdkError::RateLimited) => { /* back off and retry */ }
    Err(SdkError::Timeout) => { /* operation timed out */ }
    Err(e) => { /* other errors */ }
}
```

---

## 7. Class Structure

```
SlipstreamClient (high-level facade)
├── Config (immutable configuration)
├── Transport (trait, Arc<RwLock<>>)
│   ├── QuicTransport
│   ├── GrpcTransport
│   ├── WebSocketTransport
│   └── HttpTransport
├── WorkerSelector (latency-based selection)
├── HealthMonitor (background reconnection)
└── Internal State
    ├── latest_tip: Arc<RwLock<Option<TipInstruction>>>
    └── metrics: Arc<ClientMetrics>
```

### Thread Safety

- `SlipstreamClient` is `Clone` and `Send + Sync`
- Safe to share across Tokio tasks
- Uses `Arc<RwLock<>>` internally

---

## 8. Security

### Encryption

| Protocol | Encryption |
|----------|-----------|
| QUIC | TLS 1.3 (built-in) |
| gRPC | TLS 1.3 over HTTP/2 |
| WebSocket | WSS (TLS) |
| HTTP | HTTPS (TLS) |

### Authentication

API keys are transmitted via:
- **QUIC**: Initial handshake frame
- **gRPC**: `x-api-key` metadata header
- **WebSocket/HTTP**: `Authorization` header or `x-api-key` header

### Best Practices

1. **Never commit API keys** to version control
2. **Use environment variables** for key storage
3. **Rotate keys regularly** via the Slipstream dashboard
4. **Monitor usage** to detect unauthorized access

---

## Related Resources

- [README.md](../README.md) - Quick start guide
- [Examples](../examples/) - Runnable code samples
- [Architecture Spec](../../../../architecture/11-sdk-architecture.md) - System design
