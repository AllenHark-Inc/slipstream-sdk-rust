//! Priority Fee Configuration Example
//!
//! This example demonstrates how to configure and use priority fees with the
//! Slipstream SDK. Priority fees help ensure your transactions land faster
//! during periods of network congestion.
//!
//! # Key Concepts
//!
//! - **PriorityFeeConfig**: Configure automatic priority fee optimization
//! - **PriorityFeeSpeed**: Choose between Slow, Fast, or UltraFast
//! - **max_tip**: Set a cap on the maximum tip you're willing to pay
//! - **subscribe_priority_fees()**: Stream real-time fee recommendations
//!
//! # Running this example
//!
//! ```bash
//! SLIPSTREAM_API_KEY=sk_test_xxx cargo run --example priority_fees
//! ```

use allenhark_slipstream::{
    Config, PriorityFeeConfig, PriorityFeeSpeed, SlipstreamClient,
};
use std::env;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging for visibility
    tracing_subscriber::fmt::init();

    // Get API key from environment
    let api_key = env::var("SLIPSTREAM_API_KEY")
        .unwrap_or_else(|_| "sk_test_12345678".to_string());

    println!("=== Slipstream Priority Fee Example ===\n");

    // =========================================================================
    // EXAMPLE 1: Basic Priority Fee Configuration
    // =========================================================================
    println!("1. Basic Priority Fee Configuration");
    println!("   --------------------------------");

    let config = Config::builder()
        .api_key(&api_key)
        .priority_fee(PriorityFeeConfig {
            enabled: true,
            speed: PriorityFeeSpeed::Fast,
            max_tip: None, // No cap
        })
        .build()?;

    println!("   Priority Fee Enabled: {}", config.priority_fee.enabled);
    println!("   Speed: {:?}", config.priority_fee.speed);
    println!("   Max Tip: {:?}\n", config.priority_fee.max_tip);

    // =========================================================================
    // EXAMPLE 2: Ultra-Fast with Max Tip Cap
    // =========================================================================
    println!("2. Ultra-Fast Mode with Tip Cap");
    println!("   -----------------------------");

    let config_ultra = Config::builder()
        .api_key(&api_key)
        .priority_fee(PriorityFeeConfig {
            enabled: true,
            speed: PriorityFeeSpeed::UltraFast, // Highest priority
            max_tip: Some(0.01),                 // Cap at 0.01 SOL
        })
        .build()?;

    println!("   Speed: {:?}", config_ultra.priority_fee.speed);
    println!("   Max Tip: {} SOL\n", config_ultra.priority_fee.max_tip.unwrap());

    // =========================================================================
    // EXAMPLE 3: Subscribe to Real-Time Priority Fees
    // =========================================================================
    println!("3. Streaming Priority Fee Recommendations");
    println!("   --------------------------------------");

    // Enable priority fee streaming in config
    let config_stream = Config::builder()
        .api_key(&api_key)
        .stream_priority_fees(true) // Enable the stream
        .priority_fee(PriorityFeeConfig {
            enabled: true,
            speed: PriorityFeeSpeed::Fast,
            max_tip: Some(0.005),
        })
        .build()?;

    println!("   Connecting to Slipstream...");

    match SlipstreamClient::connect(config_stream).await {
        Ok(client) => {
            println!("   Connected! Subscribing to priority fees...\n");

            // Subscribe to priority fee updates
            let mut fee_stream = client.subscribe_priority_fees().await?;

            // Collect a few updates (with timeout)
            let timeout = Duration::from_secs(10);
            let start = std::time::Instant::now();

            println!("   Listening for priority fee updates (10s timeout)...\n");

            loop {
                tokio::select! {
                    Some(fee) = fee_stream.recv() => {
                        println!("   📊 Priority Fee Update:");
                        println!("      Speed Tier: {}", fee.speed);
                        println!("      Compute Unit Price: {} micro-lamports", fee.compute_unit_price);
                        println!("      Compute Unit Limit: {}", fee.compute_unit_limit);
                        println!("      Estimated Cost: {:.6} SOL", fee.estimated_cost_sol);
                        println!("      Landing Probability: {}%", fee.landing_probability);
                        println!("      Network Congestion: {}", fee.network_congestion);
                        println!("      Recent Success Rate: {:.1}%\n", fee.recent_success_rate * 100.0);
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        if start.elapsed() > timeout {
                            println!("   Timeout reached. Disconnecting...");
                            break;
                        }
                    }
                }
            }

            client.disconnect().await?;
        }
        Err(e) => {
            println!("   ⚠️  Could not connect (expected in test): {}\n", e);
        }
    }

    // =========================================================================
    // EXAMPLE 4: Using Priority Fees in Transaction Building
    // =========================================================================
    println!("4. Using Priority Fees in Transaction Building");
    println!("   -------------------------------------------");
    println!("   When building a transaction with priority fees:");
    println!();
    println!("   // 1. Get the current priority fee recommendation");
    println!("   let fee = client.subscribe_priority_fees().await?.recv().await;");
    println!();
    println!("   // 2. Add compute budget instructions to your transaction");
    println!("   let compute_budget_ix = ComputeBudgetInstruction::set_compute_unit_price(");
    println!("       fee.compute_unit_price");
    println!("   );");
    println!();
    println!("   // 3. Add compute unit limit instruction");
    println!("   let limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(");
    println!("       fee.compute_unit_limit");
    println!("   );");
    println!();
    println!("   // 4. Prepend these instructions to your transaction");
    println!("   transaction.instructions.insert(0, limit_ix);");
    println!("   transaction.instructions.insert(0, compute_budget_ix);");
    println!();

    println!("=== Priority Fee Example Complete ===");

    Ok(())
}
