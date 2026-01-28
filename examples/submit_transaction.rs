//! Transaction Submission Example
//!
//! Demonstrates various ways to submit transactions through Slipstream:
//! - Basic submission
//! - Submission with options
//! - Broadcast mode
//! - Deduplication
//! - Error handling
//!
//! # Running this example
//!
//! ```bash
//! SLIPSTREAM_API_KEY=sk_test_xxx cargo run --example submit_transaction
//! ```

use allenhark_slipstream::{Config, SlipstreamClient, SubmitOptions};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    println!("=== Transaction Submission Example ===\n");

    let api_key = env::var("SLIPSTREAM_API_KEY")
        .unwrap_or_else(|_| "sk_test_12345678".to_string());

    // =========================================================================
    // 1. Connect
    // =========================================================================
    println!("1. Connecting...");

    let config = Config::builder()
        .api_key(&api_key)
        .stream_tip_instructions(true)
        .max_retries(5)
        .build()?;

    match SlipstreamClient::connect(config).await {
        Ok(client) => {
            println!("   ✅ Connected via {:?}\n", 
                client.connection_info().protocol);

            // Create a dummy transaction (in production, this is your signed tx)
            let dummy_tx = vec![0u8; 200];

            // =========================================================================
            // 2. Basic Submission
            // =========================================================================
            println!("2. Basic Transaction Submission");
            println!("   -----------------------------");

            match client.submit_transaction(&dummy_tx).await {
                Ok(result) => {
                    println!("   Request ID: {}", result.request_id);
                    println!("   Transaction ID: {}", result.transaction_id);
                    println!("   Status: {:?}", result.status);
                    if let Some(sig) = &result.signature {
                        println!("   Signature: {}", sig);
                    }
                }
                Err(e) => {
                    println!("   Expected error (dummy tx): {}", e);
                }
            }
            println!();

            // =========================================================================
            // 3. Submission with Retry Options
            // =========================================================================
            println!("3. Submission with Retry Options");
            println!("   ------------------------------");

            let retry_options = SubmitOptions {
                max_retries: 10,
                timeout_ms: 60_000,
                ..Default::default()
            };

            println!("   Options: {} retries, {}ms timeout", 
                retry_options.max_retries, retry_options.timeout_ms);

            match client.submit_transaction_with_options(&dummy_tx, &retry_options).await {
                Ok(result) => println!("   Status: {:?}", result.status),
                Err(e) => println!("   Expected error: {}", e),
            }
            println!();

            // =========================================================================
            // 4. Broadcast Mode (Fan-Out)
            // =========================================================================
            println!("4. Broadcast Mode Submission");
            println!("   --------------------------");

            let broadcast_options = SubmitOptions {
                broadcast_mode: true,  // Send to ALL regions
                max_retries: 3,
                timeout_ms: 60_000,
                ..Default::default()
            };

            println!("   Broadcast Mode: ENABLED (fan-out to all regions)");

            match client.submit_transaction_with_options(&dummy_tx, &broadcast_options).await {
                Ok(result) => {
                    println!("   Status: {:?}", result.status);
                    if let Some(routing) = &result.routing {
                        println!("   Routed via: {} ({})", routing.region, routing.sender);
                    }
                }
                Err(e) => println!("   Expected error: {}", e),
            }
            println!();

            // =========================================================================
            // 5. With Deduplication
            // =========================================================================
            println!("5. Submission with Deduplication");
            println!("   ------------------------------");

            let dedup_options = SubmitOptions {
                dedup_id: Some("unique-tx-id-12345".to_string()),
                max_retries: 5,
                ..Default::default()
            };

            println!("   Dedup ID: {:?}", dedup_options.dedup_id);
            println!("   → Same ID across retries prevents double-spend");

            match client.submit_transaction_with_options(&dummy_tx, &dedup_options).await {
                Ok(result) => println!("   Status: {:?}", result.status),
                Err(e) => println!("   Expected error: {}", e),
            }
            println!();

            // =========================================================================
            // 6. Preferred Sender
            // =========================================================================
            println!("6. Preferred Sender");
            println!("   -----------------");

            let sender_options = SubmitOptions {
                preferred_sender: Some("0slot".to_string()),
                broadcast_mode: false,
                ..Default::default()
            };

            println!("   Preferred Sender: {:?}", sender_options.preferred_sender);
            println!("   → Routes ONLY through this sender");

            match client.submit_transaction_with_options(&dummy_tx, &sender_options).await {
                Ok(result) => println!("   Status: {:?}", result.status),
                Err(e) => println!("   Expected error: {}", e),
            }
            println!();

            // =========================================================================
            // 7. Check Metrics After Submissions
            // =========================================================================
            println!("7. Metrics After Submissions");
            println!("   --------------------------");

            let metrics = client.metrics();
            println!("   Total Submitted: {}", metrics.transactions_submitted);
            println!("   Total Confirmed: {}", metrics.transactions_confirmed);
            println!("   Success Rate: {:.1}%", metrics.success_rate * 100.0);
            println!("   Avg Latency: {:.1}ms", metrics.average_latency_ms);

            client.disconnect().await?;
        }
        Err(e) => {
            println!("   ⚠️  Connection failed: {}", e);
        }
    }

    println!("\n=== Transaction Submission Complete ===");

    Ok(())
}
