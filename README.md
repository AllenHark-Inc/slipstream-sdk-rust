# allenhark-slipstream

Rust SDK for Slipstream - Solana transaction relay. 

## Installation

```toml
[dependencies]
allenhark-slipstream = "0.1"
```

## Quick Start

```rust
use allenhark_slipstream::{SlipstreamClient, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = SlipstreamClient::new(Config {
        api_key: "sk_live_...".into(),
        region: Some("us-west".into()),
        ..Default::default()
    }).await?;
    
    // Subscribe to tip instructions
    let mut tips = client.subscribe_tips().await?;
    
    // Submit transaction
    let signature = client.submit_transaction(tx).await?;
    
    Ok(())
}
```

## Features

- **Persistent Connections** - Connect once, reuse
- **QUIC Primary** - Low latency with fallback
- **Ping-Based Selection** - Connect to fastest worker
- **Streaming Tips** - Real-time tip updates

## Documentation