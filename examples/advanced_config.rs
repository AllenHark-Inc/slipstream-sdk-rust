//! Advanced Configuration Example
//!
//! Demonstrates configuration options: timeouts, protocols, retry strategies.

use allenhark_slipstream::{
    BackoffStrategy, Config, PriorityFeeConfig, PriorityFeeSpeed,
    Protocol, ProtocolTimeouts, SlipstreamClient,
};
use std::env;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let api_key = env::var("SLIPSTREAM_API_KEY").unwrap_or("sk_test_12345678".into());

    println!("=== Advanced Configuration ===\n");

    // Production config
    let production = Config::builder()
        .api_key(&api_key)
        .region("us-east")
        .connection_timeout(Duration::from_secs(15))
        .max_retries(5)
        .retry_backoff(BackoffStrategy::Exponential)
        .idle_timeout(Duration::from_secs(300))
        .leader_hints(true)
        .stream_tip_instructions(true)
        .stream_priority_fees(true)
        .priority_fee(PriorityFeeConfig {
            enabled: true,
            speed: PriorityFeeSpeed::Fast,
            max_tip: Some(0.01),
        })
        .min_confidence(75)
        .build()?;

    println!("Region: {:?}", production.region);
    println!("Timeout: {:?}", production.connection_timeout);
    println!("Retries: {}, Backoff: {:?}", production.max_retries, production.retry_backoff);

    // Custom protocol timeouts
    let custom = Config::builder()
        .api_key(&api_key)
        .protocol_timeouts(ProtocolTimeouts {
            quic: Duration::from_millis(1500),
            grpc: Duration::from_millis(2500),
            websocket: Duration::from_millis(3000),
            http: Duration::from_millis(5000),
        })
        .build()?;

    println!("\nCustom Timeouts: QUIC={:?}, gRPC={:?}", 
        custom.protocol_timeouts.quic, custom.protocol_timeouts.grpc);

    // Force specific protocol
    let grpc_only = Config::builder()
        .api_key(&api_key)
        .preferred_protocol(Protocol::Grpc)
        .build()?;

    println!("Preferred Protocol: {:?}", grpc_only.preferred_protocol);

    // Connect
    match SlipstreamClient::connect(production).await {
        Ok(client) => {
            let status = client.connection_status().await;
            println!("\n✅ Connected: {:?} via {:?}", status.state, status.protocol);
            client.disconnect().await?;
        }
        Err(e) => println!("\n⚠️  Connection failed: {}", e),
    }

    Ok(())
}
