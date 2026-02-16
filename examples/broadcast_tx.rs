//! Broadcast Mode Example
//!
//! This example demonstrates how to use broadcast mode for transaction
//! submission. Broadcast mode sends your transaction to multiple regions
//! simultaneously (fan-out), maximizing the chance of fast inclusion.
//!
//! # Key Concepts
//!
//! - **Broadcast Mode**: Fan-out transaction to all available regions
//! - **Preferred Sender**: Optionally specify a preferred sender
//! - **Deduplication ID**: Prevent duplicate submissions
//! - **Max Retries**: Configure retry behavior
//!
//! # When to Use Broadcast Mode
//!
//! - Time-sensitive arbitrage opportunities
//! - High-value transactions where speed is critical
//! - NFT mints or token launches
//! - When network is congested and you need maximum coverage
//!
//! # Running this example
//!
//! ```bash
//! SLIPSTREAM_API_KEY=sk_test_xxx cargo run --example broadcast_tx
//! ```

use allenhark_slipstream::{Config, SlipstreamClient, SubmitOptions};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    let api_key = env::var("SLIPSTREAM_API_KEY")
        .unwrap_or_else(|_| "sk_test_12345678".to_string());

    println!("=== Slipstream Broadcast Mode Example ===\n");

    // =========================================================================
    // EXAMPLE 1: Default Submit (Single Region)
    // =========================================================================
    println!("1. Default Transaction Submission");
    println!("   -------------------------------");
    println!("   By default, transactions are routed to the optimal region");
    println!("   based on leader hints and sender scoring.\n");

    let default_options = SubmitOptions::default();
    println!("   Broadcast Mode: {}", default_options.broadcast_mode);
    println!("   Preferred Sender: {:?}", default_options.preferred_sender);
    println!("   Max Retries: {}", default_options.max_retries);
    println!("   Timeout: {}ms\n", default_options.timeout_ms);

    // =========================================================================
    // EXAMPLE 2: Broadcast Mode (Fan-Out)
    // =========================================================================
    println!("2. Broadcast Mode Configuration");
    println!("   -----------------------------");

    let broadcast_options = SubmitOptions {
        broadcast_mode: true,           // Enable fan-out to all regions
        preferred_sender: None,         // Let Slipstream choose
        max_retries: 3,                 // More retries for critical tx
        timeout_ms: 60_000,             // Longer timeout
        dedup_id: Some("my-unique-tx-id-12345".to_string()), // Prevent duplicates
        retry: None,
    };

    println!("   Broadcast Mode: {} (fan-out enabled)", broadcast_options.broadcast_mode);
    println!("   Max Retries: {}", broadcast_options.max_retries);
    println!("   Timeout: {}ms", broadcast_options.timeout_ms);
    println!("   Dedup ID: {:?}\n", broadcast_options.dedup_id);

    // =========================================================================
    // EXAMPLE 3: Preferred Sender
    // =========================================================================
    println!("3. Using a Preferred Sender");
    println!("   -------------------------");
    println!("   You can specify a preferred sender if you have a relationship");
    println!("   with a specific block producer or want guaranteed routing.\n");

    let preferred_options = SubmitOptions {
        broadcast_mode: false,
        preferred_sender: Some("0slot".to_string()), // Route through 0slot
        max_retries: 2,
        timeout_ms: 30_000,
        dedup_id: None,
        retry: None,
    };

    println!("   Preferred Sender: {:?}", preferred_options.preferred_sender);
    println!("   Note: Transaction will ONLY be sent to this sender\n");

    // =========================================================================
    // EXAMPLE 4: Submitting with Broadcast Mode
    // =========================================================================
    println!("4. Submitting with Broadcast Mode");
    println!("   -------------------------------");

    let config = Config::builder()
        .api_key(&api_key)
        .build()?;

    match SlipstreamClient::connect(config).await {
        Ok(client) => {
            println!("   Connected to Slipstream!");

            // Create a dummy transaction (in real code, this would be a signed tx)
            let dummy_tx = vec![0u8; 100]; // Placeholder

            // Submit with broadcast mode
            println!("   Submitting transaction with broadcast mode...\n");

            match client.submit_transaction_with_options(&dummy_tx, &broadcast_options).await {
                Ok(result) => {
                    println!("   ✅ Transaction Submitted!");
                    println!("      Request ID: {}", result.request_id);
                    println!("      Transaction ID: {}", result.transaction_id);
                    println!("      Status: {:?}", result.status);
                    if let Some(sig) = result.signature {
                        println!("      Signature: {}", sig);
                    }
                    if let Some(routing) = result.routing {
                        println!("      Region: {}", routing.region);
                        println!("      Sender: {}", routing.sender);
                        println!("      Total Latency: {}ms", routing.total_latency_ms);
                    }
                }
                Err(e) => {
                    println!("   ⚠️  Submission failed: {}", e);
                }
            }

            client.disconnect().await?;
        }
        Err(e) => {
            println!("   ⚠️  Could not connect (expected in test): {}\n", e);
        }
    }

    // =========================================================================
    // EXAMPLE 5: Best Practices
    // =========================================================================
    println!("\n5. Best Practices for Broadcast Mode");
    println!("   ----------------------------------");
    println!();
    println!("   ✓ Use broadcast mode for time-sensitive transactions");
    println!("   ✓ Always set a dedup_id to prevent double-spending");
    println!("   ✓ Increase timeout for broadcast (more regions = more time)");
    println!("   ✓ Monitor results to understand which regions are fastest");
    println!();
    println!("   ✗ Don't use broadcast mode for every transaction (wastes resources)");
    println!("   ✗ Don't set preferred_sender AND broadcast_mode (they conflict)");
    println!();

    // =========================================================================
    // EXAMPLE 6: Code Pattern
    // =========================================================================
    println!("6. Complete Code Pattern");
    println!("   ----------------------\n");

    println!(r#"
   use allenhark_slipstream::{{Config, SlipstreamClient, SubmitOptions}};
   use uuid::Uuid;

   async fn submit_critical_tx(tx_bytes: &[u8]) -> Result<String, Error> {{
       let client = SlipstreamClient::connect(
           Config::builder()
               .api_key("sk_live_...")
               .build()?
       ).await?;

       let options = SubmitOptions {{
           broadcast_mode: true,
           preferred_sender: None,
           max_retries: 5,
           timeout_ms: 60_000,
           dedup_id: Some(Uuid::new_v4().to_string()),
       }};

       let result = client.submit_transaction_with_options(tx_bytes, &options).await?;
       
       Ok(result.signature.unwrap_or_default())
   }}
"#);

    println!("=== Broadcast Mode Example Complete ===");

    Ok(())
}
