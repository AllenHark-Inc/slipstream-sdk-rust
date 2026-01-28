//! Basic Connection Example
//!
//! This example demonstrates the simplest way to connect to Slipstream,
//! check connection status, and disconnect cleanly.
//!
//! # Running this example
//!
//! ```bash
//! SLIPSTREAM_API_KEY=sk_test_xxx cargo run --example basic
//! ```

use allenhark_slipstream::{Config, SlipstreamClient};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging (optional but helpful for debugging)
    tracing_subscriber::fmt::init();

    println!("=== Slipstream Basic Connection Example ===\n");

    // Get API key from environment variable
    let api_key = env::var("SLIPSTREAM_API_KEY")
        .unwrap_or_else(|_| "sk_test_12345678".to_string());

    // =========================================================================
    // 1. Build Configuration
    // =========================================================================
    println!("1. Building Configuration");
    println!("   -----------------------");

    let config = Config::builder()
        .api_key(&api_key)          // Required: your API key
        .region("us-east")          // Optional: prefer this region
        .leader_hints(true)         // Optional: enable leader hints
        .build()?;

    println!("   API Key: {}...", &api_key[..12.min(api_key.len())]);
    println!("   Region: {:?}", config.region);
    println!("   Leader Hints: {}", config.leader_hints);
    println!();

    // =========================================================================
    // 2. Connect to Slipstream
    // =========================================================================
    println!("2. Connecting to Slipstream");
    println!("   -------------------------");

    match SlipstreamClient::connect(config).await {
        Ok(client) => {
            println!("   ✅ Connected successfully!\n");

            // =========================================================================
            // 3. Check Connection Info
            // =========================================================================
            println!("3. Connection Information");
            println!("   -----------------------");

            let info = client.connection_info();
            println!("   Session ID: {}", info.session_id);
            println!("   Region: {:?}", info.region);
            println!("   Protocol: {}", info.protocol);
            println!();

            // =========================================================================
            // 4. Check Connection Status
            // =========================================================================
            println!("4. Connection Status");
            println!("   ------------------");

            let status = client.connection_status().await;
            println!("   State: {:?}", status.state);
            println!("   Protocol: {:?}", status.protocol);
            println!("   Region: {:?}", status.region);
            println!();

            // =========================================================================
            // 5. Check Metrics
            // =========================================================================
            println!("5. Performance Metrics");
            println!("   -------------------");

            let metrics = client.metrics();
            println!("   Transactions Submitted: {}", metrics.transactions_submitted);
            println!("   Transactions Confirmed: {}", metrics.transactions_confirmed);
            println!("   Success Rate: {:.1}%", metrics.success_rate * 100.0);
            println!("   Average Latency: {:.1}ms", metrics.average_latency_ms);
            println!();

            // =========================================================================
            // 6. Disconnect
            // =========================================================================
            println!("6. Disconnecting");
            println!("   -------------");

            client.disconnect().await?;
            println!("   ✅ Disconnected cleanly");
        }
        Err(e) => {
            println!("   ⚠️  Connection failed: {}", e);
            println!("   This is expected if running without a valid API key.");
        }
    }

    println!("\n=== Basic Example Complete ===");

    Ok(())
}
